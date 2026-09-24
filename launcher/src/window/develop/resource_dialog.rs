//! 资源序列化/反序列化进度弹窗。
//!
//! 开发页「资源管理」三个栏目（模组/资源包/光影）共用这一个弹窗：
//! - 序列化：`serialize_resources` 在工作线程里扫资源目录、查平台指纹并写
//!   `<项目>/.rev_launcher/<目录>/*.toml`，弹窗通过 channel 收进度事件。
//! - 反序列化：`deserialize_resources` 读 toml 记录并返回一个下载器，
//!   弹窗轮询下载任务状态展示每个文件的进度。
//!
//! 任务在弹窗打开时就已经开始，关掉弹窗只丢接收端，不中断磁盘写入或下载。
//!
//! 弹窗不认开发页：跑完只是回调一下 `on_finished`，谁打开的谁自己决定要刷新什么
//! （开发页重扫资源列表，推送弹窗重数一遍工作副本的改动）。

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Sizable, StyledExt, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, AppContext, Context, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, px, uniform_list,
};

use mclib::detective::base::{
    ResourceKind, SerializeModsProgress, deserialize_resources, serialize_resources,
};
use mclib::error::Error;
use mclib::project::game_project::GameProject;
use sharingan::downloader::Downloader;
use sharingan::status::{DownloadFailure, DownloadStatus};

use super::PackKind;

/// 序列化时保留在弹窗里的最近处理文件名数量。
const RECENT_FILES: usize = 30;

/// 反序列化时下载线程数。
const DOWNLOAD_THREADS: usize = 10;

/// 弹窗承载的后台任务。
enum Task {
    /// 序列化进度 channel；`serialize_resources` 的内部线程会往里发事件。
    Serialize(Receiver<Result<SerializeModsProgress, Error>>),
    /// 反序列化返回的下载器，任务状态随时可读；弹窗存活期间不能丢，
    /// 丢掉会取消下载。
    Deserialize(Downloader),
}

/// 序列化进度快照，`total` 为 `None` 表示 `WriteStart` 还没来
/// （正在扫目录/算哈希）。
#[derive(Debug, Default)]
struct SerializeStatus {
    /// 已完成的文件数。
    done: usize,
    /// 文件总数，`WriteStart` 后才有值。
    total: Option<usize>,
    /// 最近处理完的文件名，最新的在尾部。
    recent: VecDeque<String>,
    /// 当前阶段说明文字。
    phase: String,
    /// 收到 `Done`（或线程异常结束）后置真。
    finished: bool,
}

/// 反序列化文件行的状态；`DownloadStatus` 不是 `Copy`，
/// 快照时换成自己的枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowStatus {
    Waiting,
    Preparing,
    Downloading,
    Succeeded,
    Failed,
}

/// 反序列化的单个文件行快照。
#[derive(Debug)]
struct FileRow {
    name: String,
    status: RowStatus,
    /// 本地 sha1 一致被跳过（`task.options().skip_download`）。
    skipped: bool,
    downloaded: u64,
    total: Option<u64>,
    failure: Option<String>,
}

fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", value as f64 / (1024. * 1024. * 1024.))
    } else if value >= 1024 * 1024 {
        format!("{:.1} MiB", value as f64 / (1024. * 1024.))
    } else if value >= 1024 {
        format!("{:.1} KiB", value as f64 / 1024.)
    } else {
        format!("{value} B")
    }
}

/// 一条反序列化失败原因的可读描述。
fn failure_message(reason: &DownloadFailure) -> String {
    match reason {
        DownloadFailure::PreparationError(message) => {
            format!("查询下载信息失败：{message}")
        }
        DownloadFailure::UserCancel => "已取消".to_owned(),
        DownloadFailure::NetworkError(error) => format!("网络错误：{error}"),
        DownloadFailure::HttpStatusCode(code) => format!("服务器返回 {code}"),
        DownloadFailure::IOError(error) => format!("文件读写失败：{error}"),
        DownloadFailure::ValidationError => "下载完成但校验失败".to_owned(),
        DownloadFailure::Unknown => "未知错误".to_owned(),
    }
}

/// 排空序列化 channel 里积累的事件，更新进度快照；
/// 返回任务是否结束（Done、出错或 channel 断开）。
fn poll_serialize(
    status: &mut SerializeStatus,
    error: &mut Option<String>,
    rx: &mut Receiver<Result<SerializeModsProgress, Error>>,
) -> bool {
    loop {
        match rx.try_recv() {
            Ok(Ok(SerializeModsProgress::QueryCurseforge)) => {
                status.phase = "正在查询 CurseForge 指纹…".to_owned();
            }
            Ok(Ok(SerializeModsProgress::QueryModrinth)) => {
                status.phase = "正在查询 Modrinth…".to_owned();
            }
            Ok(Ok(SerializeModsProgress::WriteStart { total })) => {
                status.total = Some(total);
                status.phase = format!("正在写入 {total} 个文件…");
            }
            Ok(Ok(SerializeModsProgress::WriteDone { filename, .. })) => {
                status.done += 1;
                status.recent.push_back(filename);
                while status.recent.len() > RECENT_FILES {
                    status.recent.pop_front();
                }
            }
            Ok(Ok(SerializeModsProgress::Done { total })) => {
                status.total = Some(total);
                status.done = total;
                status.finished = true;
                return true;
            }
            Ok(Err(err)) => {
                *error = Some(format!("序列化失败：{err}"));
                return true;
            }
            // 没有新事件，任务还在跑。
            Err(TryRecvError::Empty) => return false,
            // channel 关闭却没收到 Done，按异常结束处理。
            Err(TryRecvError::Disconnected) => {
                if error.is_none() {
                    *error = Some("序列化任务意外结束".to_owned());
                }
                status.finished = true;
                return true;
            }
        }
    }
}

/// 反序列化任务的文件行快照，按文件名排序。
fn file_rows(downloader: &Downloader) -> Vec<FileRow> {
    let mut rows: Vec<FileRow> = downloader
        .tasks()
        .values()
        .map(|task| {
            let status = task.status();
            let progress = task.progress();
            let failure = if status.is_failed() {
                Some(failure_message(&task.failed_reason()))
            } else {
                None
            };
            FileRow {
                name: task.filename().clone(),
                status: match status {
                    DownloadStatus::Ready => RowStatus::Waiting,
                    DownloadStatus::Preparing => RowStatus::Preparing,
                    DownloadStatus::Downloading => RowStatus::Downloading,
                    DownloadStatus::Succeeded => RowStatus::Succeeded,
                    DownloadStatus::Failed => RowStatus::Failed,
                },
                skipped: task.options().skip_download && status.is_success(),
                downloaded: progress.downloaded(),
                total: progress.total(),
                failure,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

pub(crate) struct ResourceSyncDialog {
    title: String,
    noun: &'static str,
    /// 后台任务；`None` 时任务没起来，`error` 里有原因。
    task: Option<Task>,
    serialize: SerializeStatus,
    /// 启动失败或序列化线程报告的错误。
    error: Option<String>,
    /// 反序列化整体结束标记：任务表全部进入终态。
    deserialize_finished: bool,
    /// 是否已经回调过调用方。
    notified: bool,
    /// 结束（成功或失败）后回调一次，让调用方把自己的列表刷回磁盘状态。
    on_finished: Rc<dyn Fn(&mut Window, &mut App)>,
    focus_handle: FocusHandle,
}

impl ResourceSyncDialog {
    /// 启动序列化任务并返回弹窗；进度轮询由 `start_poll` 负责。
    fn serialize(
        project: &GameProject,
        kind: ResourceKind,
        noun: &'static str,
        on_finished: Rc<dyn Fn(&mut Window, &mut App)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (task, error) = match serialize_resources(project, kind) {
            Ok(rx) => (Some(Task::Serialize(rx)), None),
            Err(error) => (None, Some(format!("启动序列化失败：{error}"))),
        };
        Self {
            title: format!("序列化{noun} · {}", project.name),
            noun,
            task,
            serialize: SerializeStatus::default(),
            error,
            deserialize_finished: false,
            notified: false,
            on_finished,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 启动反序列化任务并返回弹窗。
    fn deserialize(
        project: &GameProject,
        kind: ResourceKind,
        noun: &'static str,
        on_finished: Rc<dyn Fn(&mut Window, &mut App)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (task, error) = match deserialize_resources(project, kind, DOWNLOAD_THREADS) {
            Ok(downloader) => (Some(Task::Deserialize(downloader)), None),
            Err(error) => (None, Some(format!("启动反序列化失败：{error}"))),
        };
        Self {
            title: format!("反序列化{noun} · {}", project.name),
            noun,
            task,
            serialize: SerializeStatus::default(),
            error,
            deserialize_finished: false,
            notified: false,
            on_finished,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 打开弹窗；任务此时已在跑，弹窗只是观察进度。
    ///
    /// 这里不看有没有别的弹窗：开发页的工具栏调用前自己会看，推送弹窗则要压在它上面。
    pub(crate) fn open<P: 'static>(
        project: &GameProject,
        kind: PackKind,
        serialize: bool,
        on_finished: Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut Context<P>,
    ) {
        let noun = kind.noun();
        let resource_kind = kind.resource_kind();

        let dialog = cx.new(|cx| {
            let dialog = if serialize {
                Self::serialize(project, resource_kind, noun, on_finished, cx)
            } else {
                Self::deserialize(project, resource_kind, noun, on_finished, cx)
            };
            dialog.start_poll(window, cx);
            dialog
        });

        let focus = dialog.read(cx).focus_handle.clone();
        window.open_dialog(cx, move |builder, window, cx| {
            let view = dialog.read(cx);
            let running = view.is_running();
            builder
                .title(view.title.clone())
                .overlay(true)
                .overlay_closable(true)
                .w((window.bounds().size.width * 0.6).min(px(560.)))
                .footer(
                    div()
                        .h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(
                            Label::new(if running {
                                "关闭弹窗后任务会继续进行"
                            } else {
                                "任务已结束"
                            })
                            .text_sm(),
                        )
                        .child(
                            Button::new("resource-sync-close")
                                .label("关闭")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
                .child(dialog.clone())
        });
        focus.focus(window, cx);
    }

    /// 任务是否还在跑：序列化等 Done/错误，反序列化等任务表全部终态。
    fn is_running(&self) -> bool {
        match &self.task {
            Some(Task::Serialize(_)) => !self.serialize_finished(),
            Some(Task::Deserialize(_)) => !self.deserialize_finished,
            None => false,
        }
    }

    /// 序列化是否已经收尾（Done、出错或线程断开）。
    fn serialize_finished(&self) -> bool {
        self.error.is_some() || self.serialize.finished
    }

    /// 后台轮询：每 150ms 同步一次任务进度，结束后回调一次调用方。
    fn start_poll(&self, window: &mut Window, cx: &mut Context<Self>) {
        // 轮询自己握着弹窗的强引用：任务跑完还要回调调用方，哪怕用户先把弹窗关了
        // （外面的引用就没了），回调也不能半路失踪——不然调用方那边的「正在跑」
        // 状态永远解不开。
        let dialog = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let running = dialog
                    .update_in(cx, |dialog, window, cx| {
                        let finished = dialog.poll_progress(cx);
                        if finished && !dialog.notified {
                            dialog.notified = true;
                            // 序列化写了 toml、反序列化落了资源文件，
                            // 都让调用方把自己的列表回到磁盘状态。
                            (dialog.on_finished)(window, cx);
                        }
                        cx.notify();
                        !finished
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
    }

    /// 把任务侧的最新进度搬进弹窗状态；返回任务是否结束。
    fn poll_progress(&mut self, _cx: &mut Context<Self>) -> bool {
        match &mut self.task {
            Some(Task::Serialize(rx)) => {
                poll_serialize(&mut self.serialize, &mut self.error, rx)
            }
            Some(Task::Deserialize(downloader)) => {
                self.deserialize_finished = downloader.is_finished();
                self.deserialize_finished
            }
            None => true,
        }
    }

    /// 序列化进度面板：阶段说明 + 进度条 + 最近处理的文件。
    fn render_serialize(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.serialize.total;
        let running = self.is_running();

        let mut panel = div().v_flex().w_full().min_w_0().gap_2();

        let phase = if self.serialize_finished() {
            if self.error.is_some() {
                "序列化失败".to_owned()
            } else {
                format!("完成，共处理 {} 个{}", total.unwrap_or(0), self.noun)
            }
        } else if self.serialize.phase.is_empty() {
            format!("正在扫描{}目录…", self.noun)
        } else {
            self.serialize.phase.clone()
        };
        panel = panel.child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .when(running, |row| row.child(Spinner::new().small()))
                .child(Label::new(phase).text_sm()),
        );

        // 拿到总数前用 indeterminate 进度条。
        let value = match total {
            Some(total) if total > 0 => {
                self.serialize.done as f32 / total as f32 * 100.
            }
            Some(_) => 100.,
            None => 0.,
        };
        panel = panel.child(
            Progress::new("resource-serialize-progress")
                .small()
                .value(value)
                .loading(running && total.is_none())
                .accessibility_label("序列化进度"),
        );

        if !self.serialize.recent.is_empty() {
            let recent: Vec<SharedString> = self
                .serialize
                .recent
                .iter()
                .rev()
                .map(|name| SharedString::from(name.clone()))
                .collect();
            let count = recent.len();
            panel = panel.child(Label::new("最近处理").text_sm()).child(
                uniform_list("resource-serialize-files", count, move |range, _, _| {
                    range
                        .map(|row| {
                            div()
                                .h(px(24.))
                                .w_full()
                                .min_w_0()
                                .truncate()
                                .child(Label::new(recent[row].clone()).text_sm())
                        })
                        .collect::<Vec<_>>()
                })
                .w_full()
                .h(px((count.min(8) * 24) as f32)),
            );
        }

        if let Some(error) = &self.error {
            panel = panel.child(
                Label::new(error.clone())
                    .text_sm()
                    .text_color(cx.theme().danger),
            );
        }

        panel
    }

    /// 反序列化进度面板：汇总行 + 进度条 + 每个文件的下载状态列表。
    fn render_deserialize(
        &self,
        downloader: &Downloader,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = file_rows(downloader);
        let total = rows.len();
        let downloaded = rows
            .iter()
            .filter(|row| row.status == RowStatus::Succeeded && !row.skipped)
            .count();
        let skipped = rows.iter().filter(|row| row.skipped).count();
        let failed = rows
            .iter()
            .filter(|row| row.status == RowStatus::Failed)
            .count();
        let running = self.is_running();

        let mut panel = div().v_flex().w_full().min_w_0().gap_2();

        let summary = if total == 0 {
            format!("没有可恢复的{}记录", self.noun)
        } else {
            format!("共 {total} 个 · 下载 {downloaded} · 跳过 {skipped} · 失败 {failed}")
        };
        panel = panel.child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .when(running, |row| row.child(Spinner::new().small()))
                .child(Label::new(summary).text_sm()),
        );

        if total > 0 {
            let finished = downloaded + skipped + failed;
            panel = panel.child(
                Progress::new("resource-deserialize-progress")
                    .small()
                    .value(finished as f32 / total as f32 * 100.)
                    .accessibility_label("反序列化进度"),
            );

            panel = panel.child(
                uniform_list("resource-deserialize-files", total, move |range, _, cx| {
                    range
                        .map(|row_ix| {
                            let row = &rows[row_ix];
                            let (text, color) = match row.status {
                                RowStatus::Succeeded if row.skipped => {
                                    ("跳过", cx.theme().muted_foreground)
                                }
                                RowStatus::Succeeded => ("完成", cx.theme().success),
                                RowStatus::Failed => ("失败", cx.theme().danger),
                                RowStatus::Downloading => ("下载中", cx.theme().primary),
                                RowStatus::Preparing => ("查询中", cx.theme().primary),
                                RowStatus::Waiting => ("等待", cx.theme().muted_foreground),
                            };
                            let detail = match (row.status, row.total) {
                                (RowStatus::Downloading, Some(total)) if total > 0 => {
                                    format!("{} / {}", bytes(row.downloaded), bytes(total))
                                }
                                (RowStatus::Downloading, _) => {
                                    format!("已下载 {}", bytes(row.downloaded))
                                }
                                (RowStatus::Failed, _) => row
                                    .failure
                                    .clone()
                                    .unwrap_or_else(|| "失败".to_owned()),
                                _ => String::new(),
                            };
                            div()
                                .h_flex()
                                .h(px(24.))
                                .w_full()
                                .min_w_0()
                                .items_center()
                                .gap_2()
                                .child(Label::new(text).text_sm().text_color(color))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(Label::new(row.name.clone()).text_sm()),
                                )
                                .child(
                                    Label::new(detail)
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground),
                                )
                        })
                        .collect::<Vec<_>>()
                })
                .w_full()
                .h(px((total.min(10) * 24) as f32)),
            );
        }

        if let Some(error) = &self.error {
            panel = panel.child(
                Label::new(error.clone())
                    .text_sm()
                    .text_color(cx.theme().danger),
            );
        }

        panel
    }
}

impl Render for ResourceSyncDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id("resource-sync-dialog-content")
            .track_focus(&self.focus_handle)
            .v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .pb_2();

        match &self.task {
            Some(Task::Serialize(_)) => {
                content = content.child(self.render_serialize(cx));
            }
            Some(Task::Deserialize(downloader)) => {
                content = content.child(self.render_deserialize(downloader, cx));
            }
            // 任务没起来（比如目录读不了），直接显示错误。
            None => {
                content = content.child(
                    Label::new(
                        self.error
                            .clone()
                            .unwrap_or_else(|| "任务无法启动".to_owned()),
                    )
                    .text_sm()
                    .text_color(cx.theme().danger),
                );
            }
        }

        content
    }
}
