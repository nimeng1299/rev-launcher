use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::marker::{Marker, MarkerContent, MarkerIcon, MarkerLoadingStyle};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, WindowExt,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, px, uniform_list,
};
use mclib::error::Error;
use mclib::launch::{LaunchInfo, LaunchState};
use sharingan::downloader::Downloader;
use sharingan::status::DownloadStatus;
use std::time::Duration;

const STEPS: [&str; 6] = [
    "准备启动",
    "验证账号",
    "检查 Java",
    "下载支持库",
    "下载模组",
    "启动游戏",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepStatus {
    Pending,
    Active,
    Complete,
    Failed,
    Skipped,
}

fn step_index(state: LaunchState) -> usize {
    match state {
        LaunchState::Ready => 0,
        LaunchState::CheckAccount => 1,
        LaunchState::CheckJava => 2,
        LaunchState::DownloadLibrary => 3,
        LaunchState::DownloadMods => 4,
        LaunchState::Launch | LaunchState::Running | LaunchState::Exited => 5,
        LaunchState::Failed => 0,
    }
}

fn step_status(index: usize, state: LaunchState, failed_state: Option<LaunchState>) -> StepStatus {
    if matches!(state, LaunchState::Running | LaunchState::Exited) {
        return StepStatus::Complete;
    }
    let current = step_index(failed_state.unwrap_or(state));
    if index < current {
        StepStatus::Complete
    } else if state == LaunchState::Failed {
        if index == current {
            StepStatus::Failed
        } else {
            StepStatus::Skipped
        }
    } else if index == current {
        StepStatus::Active
    } else {
        StepStatus::Pending
    }
}

#[derive(Debug)]
struct FileProgress {
    id: usize,
    name: String,
    failed: bool,
    downloaded: u64,
    total: Option<u64>,
    speed: f64,
}

impl FileProgress {
    fn percentage(&self) -> Option<f32> {
        self.total
            .filter(|total| *total > 0)
            .map(|total| ((self.downloaded as f64 / total as f64) * 100.).clamp(0., 100.) as f32)
    }
}

#[derive(Debug, Default)]
struct DownloadSnapshot {
    total: usize,
    complete: usize,
    active: Vec<FileProgress>,
    pending: Vec<FileProgress>,
}

impl DownloadSnapshot {
    fn read(downloader: &Downloader) -> Self {
        let mut snapshot = Self {
            total: downloader.tasks().len(),
            ..Self::default()
        };
        for task in downloader.tasks().values() {
            let status = task.status();
            if status.is_success() {
                snapshot.complete += 1;
                continue;
            }
            let progress = task.progress();
            let file = FileProgress {
                id: task.id(),
                name: task.filename().clone(),
                failed: status.is_failed(),
                downloaded: progress.downloaded(),
                total: progress.total(),
                speed: progress.speed(),
            };
            match status {
                DownloadStatus::Downloading => snapshot.active.push(file),
                _ => snapshot.pending.push(file),
            }
        }
        // task 表是 HashMap，排序后轮询不会打乱文件行；失败项优先显示。
        snapshot.active.sort_by_key(|file| file.id);
        snapshot.pending.sort_by_key(|file| (!file.failed, file.id));
        snapshot
    }
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

fn error_message(error: &Error) -> String {
    match error {
        Error::AccountExpired => "账号已过期，请重新登录。".into(),
        Error::NotFindCorrectJava(version) => {
            format!("未找到 Java {version}，请在设置中添加对应版本。")
        }
        Error::ProcessExited(Some(code)) => {
            format!("游戏异常退出，退出码：{code}。请查看启动日志。")
        }
        Error::ProcessExited(None) => "游戏异常退出，请查看启动日志。".into(),
        Error::Json(error) => format!("版本配置无法读取：{error}"),
        Error::Io(error) => format!("文件读写失败：{error}"),
        Error::NotFindSettingFile(path) => format!("未找到版本配置文件：{}", path.display()),
        Error::UnknownError => "启动失败，请查看启动日志。".into(),
        // JavaCheckFailed/DownloadFailed/LaunchFailed/LogFailed 等变体的
        // Display 就是错误信息本身，直接透传即可。
        other => other.to_string(),
    }
}

pub(super) struct LaunchDialog {
    title: String,
    launch: Option<LaunchInfo>,
    preparation_error: Option<String>,
    focus_handle: FocusHandle,
}

impl LaunchDialog {
    pub(super) fn new(
        title: String,
        launch: Result<LaunchInfo, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (launch, preparation_error) = match launch {
            Ok(info) => (Some(info), None),
            Err(error) => (None, Some(error)),
        };
        if launch.is_some() {
            // spawn_in 持有视图的弱引用，页面释放或进程退出时结束轮询。
            cx.spawn_in(window, async move |view, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    let active = view
                        .update_in(cx, |view, _, cx| {
                            cx.notify();
                            view.is_active()
                        })
                        .unwrap_or(false);
                    if !active {
                        break;
                    }
                }
            })
            .detach();
        }
        Self {
            title,
            launch,
            preparation_error,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.launch.as_ref().is_some_and(|launch| {
            !matches!(launch.state(), LaunchState::Exited | LaunchState::Failed)
        })
    }

    pub(super) fn open(page: Entity<Self>, window: &mut Window, cx: &mut App) {
        if window.has_active_dialog(cx) {
            return;
        }
        let focus = page.read(cx).focus_handle.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let view = page.read(cx);
            let log_path = view.launch.as_ref().and_then(LaunchInfo::log_path);
            dialog
                .title(format!("启动 · {}", view.title))
                .overlay(true)
                .overlay_closable(true)
                .w((window.bounds().size.width * 0.85).min(px(800.)))
                .h(window.bounds().size.height * 0.8)
                .footer(
                    div()
                        .h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(
                            Label::new(if view.is_active() {
                                "关闭弹窗后任务会继续运行"
                            } else {
                                "启动流程已结束"
                            })
                            .text_sm(),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new("launch-open-log")
                                        .label("查看日志")
                                        .disabled(log_path.is_none())
                                        .on_click(move |_, _, cx| {
                                            if let Some(path) = &log_path {
                                                cx.reveal_path(path);
                                            }
                                        }),
                                )
                                .child(
                                    Button::new("launch-close-dialog")
                                        .label("关闭")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                ),
                        ),
                )
                .child(page.clone())
        });
        focus.focus(window, cx);
    }
}

fn marker(index: usize, status: StepStatus, cx: &App) -> Marker {
    let (text, color) = match status {
        StepStatus::Pending => ("等待中", cx.theme().muted_foreground),
        StepStatus::Active => ("进行中", cx.theme().primary),
        StepStatus::Complete => ("已完成", cx.theme().success),
        StepStatus::Failed => ("失败", cx.theme().danger),
        StepStatus::Skipped => ("未执行", cx.theme().muted_foreground),
    };
    Marker::new()
        .id(("launch-step", index))
        .text_color(color)
        .loading(status == StepStatus::Active)
        .with_loading_style(MarkerLoadingStyle::Spinner)
        .when(status != StepStatus::Active, |marker| {
            marker.icon(
                MarkerIcon::new().child(
                    Icon::new(match status {
                        StepStatus::Complete => IconName::CircleCheck,
                        StepStatus::Failed => IconName::Close,
                        _ => IconName::Info,
                    })
                    .xsmall(),
                ),
            )
        })
        .content(MarkerContent::new().text_color(color).text(STEPS[index]))
        .child(div().flex_1())
        .child(Label::new(text).text_sm())
}

fn download_panel(
    index: usize,
    downloader: Option<&Downloader>,
    status: StepStatus,
    cx: &App,
) -> AnyElement {
    let mut panel = div().v_flex().w_full().min_w_0().pl_6().gap_2();
    let Some(downloader) = downloader else {
        return panel
            .child(
                Label::new(if status == StepStatus::Active {
                    "正在准备下载任务…"
                } else {
                    "尚未创建下载任务"
                })
                .text_sm()
                .text_color(cx.theme().muted_foreground),
            )
            .into_any_element();
    };
    let snapshot = DownloadSnapshot::read(downloader);
    if snapshot.total == 0 {
        return panel
            .child(Label::new("文件已齐全，无需下载").text_sm())
            .into_any_element();
    }
    let failed = snapshot.pending.iter().filter(|file| file.failed).count();
    panel = panel.child(
        Label::new(format!(
            "已完成 {}/{} · 下载中 {} · 等待 {} · 失败 {}",
            snapshot.complete,
            snapshot.total,
            snapshot.active.len(),
            snapshot.pending.len() - failed,
            failed
        ))
        .text_sm()
        .text_color(cx.theme().muted_foreground),
    );
    if !snapshot.active.is_empty() {
        panel = panel.child(Label::new("正在下载").text_sm());
    }
    for file in snapshot.active {
        let percentage = file.percentage();
        let amount = match file.total {
            Some(total) => format!("{} / {}", bytes(file.downloaded), bytes(total)),
            None => format!("已下载 {} · 大小未知", bytes(file.downloaded)),
        };
        let speed = if file.speed.is_finite() && file.speed > 0. {
            file.speed as u64
        } else {
            0
        };
        let id = SharedString::from(format!("launch-{index}-file-{}", file.id));
        panel = panel.child(
            div()
                .v_flex()
                .min_w_0()
                .w_full()
                .gap_1()
                .p_2()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(
                    div()
                        .h_flex()
                        .min_w_0()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .child(Label::new(file.name.clone()).text_sm()),
                        )
                        .child(
                            Label::new(
                                percentage
                                    .map(|value| format!("{value:.1}%"))
                                    .unwrap_or_else(|| "下载中".into()),
                            )
                            .text_sm(),
                        ),
                )
                .child(
                    Progress::new(id)
                        .xsmall()
                        .value(percentage.unwrap_or(0.))
                        .loading(percentage.is_none())
                        .accessibility_label(format!("{} 下载进度", file.name)),
                )
                .child(
                    Label::new(format!("{amount} · {}/s", bytes(speed)))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                ),
        );
    }
    if !snapshot.pending.is_empty() {
        let count = snapshot.pending.len();
        panel = panel
            .child(Label::new(format!("待下载与失败的文件（{count}）")).text_sm())
            .child(
                uniform_list(("launch-pending", index), count, move |range, _, cx| {
                    range
                        .map(|row| {
                            let file = &snapshot.pending[row];
                            div()
                                .h_flex()
                                .h(px(28.))
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .items_center()
                                .child(
                                    Label::new(if file.failed { "失败" } else { "等待" })
                                        .text_sm()
                                        .text_color(if file.failed {
                                            cx.theme().danger
                                        } else {
                                            cx.theme().muted_foreground
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(Label::new(file.name.clone()).text_sm()),
                                )
                        })
                        .collect::<Vec<_>>()
                })
                .w_full()
                .h(px((count.min(6) * 28) as f32)),
            );
    }
    panel.into_any_element()
}

impl Render for LaunchDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self
            .launch
            .as_ref()
            .map(LaunchInfo::state)
            .unwrap_or(LaunchState::Failed);
        let failed_state = self.launch.as_ref().and_then(LaunchInfo::failed_state);
        let error = self.preparation_error.clone().or_else(|| {
            self.launch
                .as_ref()
                .and_then(LaunchInfo::error)
                .map(|error| error_message(&error))
        });
        let libraries = self
            .launch
            .as_ref()
            .and_then(LaunchInfo::libraries_downloader);
        let mods = self.launch.as_ref().and_then(LaunchInfo::mods_downloader);
        let mut content = div()
            .id("launch-dialog-content")
            .track_focus(&self.focus_handle)
            .v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .pb_2();
        for index in 0..STEPS.len() {
            let status = step_status(index, state, failed_state);
            content = content.child(marker(index, status, cx));
            if index == 3 || index == 4 {
                let downloader = if index == 3 {
                    libraries.as_deref()
                } else {
                    mods.as_deref()
                };
                if downloader.is_some() || matches!(status, StepStatus::Active | StepStatus::Failed)
                {
                    content = content.child(download_panel(index, downloader, status, cx));
                }
            }
            if status == StepStatus::Failed
                && let Some(error) = &error
            {
                content = content.child(
                    div()
                        .pl_6()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                );
            }
        }
        if matches!(state, LaunchState::Running | LaunchState::Exited) {
            content = content.child(
                div().pl_6().child(
                    Label::new(if state == LaunchState::Running {
                        "游戏正在运行"
                    } else {
                        "游戏已正常退出"
                    })
                    .text_sm(),
                ),
            );
        }
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::base::actions::Cancel;
    use gpui_kit::component::Root;
    use gpui_kit::{AppContext, TestAppContext, WindowHandle};
    use sharingan::downloader::DownloadBuilder;
    use sharingan::status::DownloadFailure;

    #[test]
    fn failed_steps_are_correct_even_when_polling_missed_transitions() {
        let statuses: Vec<_> = (0..STEPS.len())
            .map(|index| step_status(index, LaunchState::Failed, Some(LaunchState::DownloadMods)))
            .collect();
        assert_eq!(
            statuses,
            [
                StepStatus::Complete,
                StepStatus::Complete,
                StepStatus::Complete,
                StepStatus::Complete,
                StepStatus::Failed,
                StepStatus::Skipped
            ]
        );
        assert_eq!(
            step_status(2, LaunchState::Failed, Some(LaunchState::CheckJava)),
            StepStatus::Failed
        );
        assert_eq!(
            step_status(3, LaunchState::Failed, Some(LaunchState::CheckJava)),
            StepStatus::Skipped
        );
        for state in [LaunchState::Running, LaunchState::Exited] {
            assert!(
                (0..STEPS.len())
                    .all(|index| step_status(index, state, None) == StepStatus::Complete)
            );
        }
        assert_eq!(
            step_status(3, LaunchState::DownloadLibrary, None),
            StepStatus::Active
        );
        assert_eq!(
            step_status(4, LaunchState::DownloadLibrary, None),
            StepStatus::Pending
        );
    }

    #[test]
    fn download_snapshot_tracks_active_waiting_failed_and_completed_files() {
        // 不创建 worker，不发起网络请求；直接更新真实 Task 的原子进度。
        let mut downloader = DownloadBuilder::new().thread_num(0).build();
        for index in 0..5 {
            downloader
                .download(move |builder| builder.filename(format!("file-{index}.jar")).build());
        }
        let tasks = downloader.tasks();
        tasks[&1].change_downloading();
        tasks[&1].progress().change_total(Some(100));
        tasks[&1].progress().update(25, 1024.);
        tasks[&2].change_success();
        tasks[&3].change_failure(DownloadFailure::Unknown);
        tasks[&4].change_downloading();

        let snapshot = DownloadSnapshot::read(&downloader);
        assert_eq!(snapshot.total, 5);
        assert_eq!(snapshot.complete, 1);
        assert_eq!(
            snapshot
                .active
                .iter()
                .map(|file| file.id)
                .collect::<Vec<_>>(),
            [1, 4]
        );
        assert_eq!(
            snapshot
                .pending
                .iter()
                .map(|file| file.id)
                .collect::<Vec<_>>(),
            [3, 0]
        );
        assert!(snapshot.pending[0].failed);
        assert_eq!(snapshot.active[0].percentage(), Some(25.));
        assert_eq!(snapshot.active[0].speed, 1024.);
        assert_eq!(snapshot.active[1].percentage(), None);

        tasks[&1].progress().update(150, 0.);
        assert_eq!(
            DownloadSnapshot::read(&downloader).active[0].percentage(),
            Some(100.)
        );
        tasks[&1].change_success();
        let next = DownloadSnapshot::read(&downloader);
        assert_eq!(next.complete, 2);
        assert!(next.active.iter().all(|file| file.id != 1));
        assert_eq!(next.pending.len(), 2);
    }

    struct DialogHost;

    impl Render for DialogHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .children(Root::render_dialog_layer(window, cx))
        }
    }

    fn with_window<R>(
        cx: &mut TestAppContext,
        window: &WindowHandle<Root>,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        cx.update(|app| {
            app.update_window(**window, |_, window, cx| f(window, cx))
                .unwrap()
        })
    }

    #[gpui_kit::test]
    fn launch_dialog_opens_once_and_close_action_works(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|_| DialogHost);
            Root::new(view, window, cx)
        });
        let page = with_window(cx, &window, |window, cx| {
            let page = cx.new(|cx| {
                LaunchDialog::new(
                    "测试项目".into(),
                    Err("请先选择默认账号".into()),
                    window,
                    cx,
                )
            });
            LaunchDialog::open(page.clone(), window, cx);
            LaunchDialog::open(page.clone(), window, cx);
            page
        });
        cx.run_until_parked();
        assert!(with_window(cx, &window, |window, cx| window.has_active_dialog(cx)));
        cx.update(|cx| {
            assert!(!page.read(cx).is_active());
            assert_eq!(
                page.read(cx).preparation_error.as_deref(),
                Some("请先选择默认账号")
            );
        });
        with_window(cx, &window, |window, cx| {
            window.dispatch_action(Box::new(Cancel), cx)
        });
        cx.run_until_parked();
        assert!(
            !with_window(cx, &window, |window, cx| window.has_active_dialog(cx)),
            "一次关闭应关闭唯一的弹窗，且焦点链仍可用"
        );
    }
}
