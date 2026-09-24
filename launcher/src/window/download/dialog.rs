//! 原版下载弹窗：表单阶段选安装目录、填项目名称；
//! 点下载后切到和启动弹窗一样的步骤进度展示。
//!
//! 下载由 `InstallProgress::install_minecraft` 在新线程中执行：
//! 拉清单并写入 `<name>.json`，下载客户端 jar 和资源文件，
//! 最后生成项目文件；依赖库留到首次启动时再拉。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::marker::{Marker, MarkerContent, MarkerIcon, MarkerLoadingStyle};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, StyledExt, WindowExt,
};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, px, uniform_list,
};
use rfd::AsyncFileDialog;
use sharingan::downloader::Downloader;
use sharingan::status::DownloadStatus;

use mclib::java::java_version::JavaVersion;
use mclib::project::install::{install_forge, install_minecraft};
use mclib::project::install::progress::{InstallProgress, InstallState};
use mclib::project::versions::forge;
use mclib::project::versions::minecreft::Version;

use crate::data::app_data::ProjectsRevision;
use crate::data::settings::AppSettings;

const INSTALL_STEPS: [&str; 4] = [
    "下载版本清单",
    "下载运行文件",
    "下载资源文件",
    "写入项目文件",
];

/// Forge 安装的步骤列表，比原版多安装器、支持库和处理器三个阶段。
const FORGE_STEPS: [&str; 8] = [
    "下载安装器",
    "下载版本清单",
    "下载运行文件",
    "下载支持库",
    "下载资源文件",
    "运行安装处理器",
    "写入版本 JSON",
    "写入项目文件",
];

/// 安装目录下拉框里的一项：设置里的版本目录、用户另选的目录，或者末尾的「浏览…」。
///
/// 存的是路径在 `AppSettings::project_paths` 里的下标而不是路径本身，
/// 即使同一个路径被添加了两次，选中项也不会有歧义。
#[derive(Debug, Clone, PartialEq, Eq)]
enum DirOption {
    Project(usize),
    Custom(PathBuf),
    Browse,
}

#[derive(Debug, Clone)]
struct DirOptionItem {
    value: DirOption,
    label: SharedString,
}

impl SearchableListItem for DirOptionItem {
    type Value = DirOption;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 把版本目录列表转成下拉框选项，末尾补上自定义目录和「浏览…」。
fn dir_option_items(paths: &[PathBuf], custom: Option<&PathBuf>) -> Vec<DirOptionItem> {
    let mut items = paths
        .iter()
        .enumerate()
        .map(|(ix, path)| DirOptionItem {
            value: DirOption::Project(ix),
            label: SharedString::from(path.to_string_lossy().to_string()),
        })
        .collect::<Vec<_>>();

    if let Some(path) = custom {
        items.push(DirOptionItem {
            value: DirOption::Custom(path.clone()),
            label: SharedString::from(format!("{}（自定义）", path.to_string_lossy())),
        });
    }

    items.push(DirOptionItem {
        value: DirOption::Browse,
        label: SharedString::from("浏览…"),
    });

    items
}

/// 附加加载器选项：原版不装加载器，Forge 需要再选一个 Forge 版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoaderKind {
    Vanilla,
    Forge,
}

/// 加载器下拉框里的一项。
#[derive(Debug, Clone)]
struct LoaderItem {
    kind: LoaderKind,
    label: SharedString,
}

impl SearchableListItem for LoaderItem {
    type Value = LoaderKind;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.kind
    }
}

/// Forge 版本下拉框里的一项，value 是 Forge 版本号字符串。
#[derive(Debug, Clone)]
struct ForgeVersionItem {
    version: String,
}

impl SearchableListItem for ForgeVersionItem {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.version.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.version
    }
}

/// Forge 版本列表的加载状态；选中 Forge 后由后台请求填充。
enum ForgeList {
    /// 还没请求过。
    Idle,
    Loading,
    Loaded(Vec<forge::Version>),
    Failed(String),
}

/// 下载状态：`None` 还在表单阶段，`Running` 携带当前步骤下标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallStatus {
    Running(usize),
    Done,
    Failed(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepStatus {
    Pending,
    Active,
    Complete,
    Failed,
    Skipped,
}

fn step_status(index: usize, status: InstallStatus) -> StepStatus {
    match status {
        InstallStatus::Done => StepStatus::Complete,
        InstallStatus::Running(current) => {
            if index < current {
                StepStatus::Complete
            } else if index == current {
                StepStatus::Active
            } else {
                StepStatus::Pending
            }
        }
        InstallStatus::Failed(current) => {
            if index < current {
                StepStatus::Complete
            } else if index == current {
                StepStatus::Failed
            } else {
                StepStatus::Skipped
            }
        }
    }
}

/// 把安装状态映射到步骤下标；Forge 和原版的步骤含义不同。
fn step_index(state: InstallState, forge: bool) -> usize {
    match state {
        InstallState::Ready | InstallState::DownloadInstaller => 0,
        InstallState::DownloadVersionJson => {
            if forge {
                1
            } else {
                0
            }
        }
        InstallState::InstallJar => {
            if forge {
                2
            } else {
                1
            }
        }
        InstallState::DownloadLibraries => 3,
        InstallState::DownloadAssets => {
            if forge {
                4
            } else {
                2
            }
        }
        InstallState::RunProcessors => 5,
        InstallState::WriteVersionJson => 6,
        InstallState::Success | InstallState::Failed => 0,
    }
}

fn step_marker(index: usize, label: &str, status: StepStatus, cx: &App) -> Marker {
    let (text, color) = match status {
        StepStatus::Pending => ("等待中", cx.theme().muted_foreground),
        StepStatus::Active => ("进行中", cx.theme().primary),
        StepStatus::Complete => ("已完成", cx.theme().success),
        StepStatus::Failed => ("失败", cx.theme().danger),
        StepStatus::Skipped => ("未执行", cx.theme().muted_foreground),
    };
    Marker::new()
        .id(("download-step", index))
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
        .content(MarkerContent::new().text_color(color).text(label.to_owned()))
        .child(div().flex_1())
        .child(Label::new(text).text_sm())
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

/// 下载器某一时刻的任务快照：正在下载的排在 `active`，
/// 排队与失败的排在 `pending`。
#[derive(Debug, Default)]
struct DownloadSnapshot {
    total: usize,
    complete: usize,
    active: Vec<FileProgress>,
    pending: Vec<FileProgress>,
}

impl DownloadSnapshot {
    /// `names` 用于把 hash 形式的文件名（objects 资源）翻译成可读路径。
    fn read(downloader: &Downloader, names: Option<&HashMap<String, String>>) -> Self {
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
            let name = names
                .and_then(|names| names.get(task.filename().as_str()))
                .cloned()
                .unwrap_or_else(|| task.filename().clone());
            let progress = task.progress();
            let file = FileProgress {
                id: task.id(),
                name,
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

/// 下载步骤下方的明细面板：汇总计数、正在下载的条目（带进度条和速度）
/// 以及排队/失败文件列表。与启动弹窗的下载面板布局一致。
fn download_panel(
    index: usize,
    downloader: Option<&Downloader>,
    names: Option<&HashMap<String, String>>,
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
    let snapshot = DownloadSnapshot::read(downloader, names);
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
        let id = SharedString::from(format!("download-{index}-file-{}", file.id));
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
                uniform_list(("install-pending", index), count, move |range, _, cx| {
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

pub(super) struct DownloadDialog {
    version: Version,
    /// 安装目录下拉框。
    dir_select: Entity<SelectState<Vec<DirOptionItem>>>,
    /// 下拉框当前确认的值，`Browse` 只触发文件选择，不会被记住。
    selected_dir: Option<DirOption>,
    /// 用户通过「浏览…」另选的目录。
    custom_dir: Option<PathBuf>,
    /// 项目名称输入框，留空时默认用版本号。
    name_input: Entity<InputState>,
    /// 加载器下拉框。
    loader_select: Entity<SelectState<Vec<LoaderItem>>>,
    /// 当前选中的加载器。
    loader: LoaderKind,
    /// Forge 版本下拉框，选中 Forge 后才展示。
    forge_select: Entity<SelectState<Vec<ForgeVersionItem>>>,
    /// 下拉框当前确认的 Forge 版本号。
    selected_forge: Option<String>,
    /// Forge 版本列表的加载状态。
    forge_list: ForgeList,
    /// 当前展示的安装步骤；选了 Forge 后换成 `FORGE_STEPS`。
    steps: &'static [&'static str],
    /// 表单校验错误，直接画在对话框里（通知会盖住对话框）。
    form_error: Option<String>,
    /// 下载状态，由轮询 `progress` 的任务写入，渲染时读取。
    status: Arc<Mutex<Option<InstallStatus>>>,
    /// 后台安装任务句柄；`Some` 表示已经点过下载。
    progress: Option<InstallProgress>,
    /// 下载失败信息。
    error: Arc<Mutex<Option<String>>>,
    /// 下载成功后创建出来的项目名，结果页展示用。
    installed_name: Option<String>,
    focus_handle: FocusHandle,
    _dir_subscription: Subscription,
    _loader_subscription: Subscription,
    _forge_subscription: Subscription,
}

impl DownloadDialog {
    pub(super) fn new(
        version: Version,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let paths = cx.global::<AppSettings>().project_paths.clone();
        // 有版本目录就默认选中第一个。
        let selected_dir = (!paths.is_empty()).then_some(DirOption::Project(0));
        let dir_select = cx.new(|cx| {
            SelectState::new(
                dir_option_items(&paths, None),
                selected_dir.as_ref().map(|_| IndexPath::new(0)),
                window,
                cx,
            )
        });

        let dir_subscription = cx.subscribe_in(
            &dir_select,
            window,
            |this: &mut Self, _state, event: &SelectEvent<Vec<DirOptionItem>>, window, cx| {
                match event {
                    SelectEvent::Confirm(Some(DirOption::Project(ix))) => {
                        this.selected_dir = Some(DirOption::Project(*ix));
                    }
                    SelectEvent::Confirm(Some(DirOption::Custom(path))) => {
                        this.selected_dir = Some(DirOption::Custom(path.clone()));
                    }
                    // 「浏览…」不是真正的目录：把下拉框拨回当前项，再弹系统目录选择框。
                    SelectEvent::Confirm(Some(DirOption::Browse)) => {
                        // 此刻还在 SelectState 自己的更新栈上，直接回头改它会重入更新，
                        // 因此推到本轮效果跑完后再拨回去。
                        cx.defer_in(window, move |this, window, cx| {
                            let index = this
                                .dir_select
                                .read(cx)
                                .selected_index(cx)
                                .filter(|ix| ix.row < this.dir_item_count(cx) - 1);
                            this.dir_select.update(cx, |state, cx| {
                                state.set_selected_index(index, window, cx);
                            });

                            let this = cx.entity().downgrade();
                            window
                                .spawn(cx, async move |cx| {
                                    let file = AsyncFileDialog::new()
                                        .set_title("选择安装目录")
                                        .pick_folder()
                                        .await;
                                    let Some(file) = file else {
                                        return;
                                    };
                                    let path = file.path().to_path_buf();

                                    let _ = this.update_in(cx, |this, window, cx| {
                                        this.custom_dir = Some(path.clone());
                                        this.selected_dir =
                                            Some(DirOption::Custom(path.clone()));

                                        // 自定义目录总是倒数第二项（「浏览…」固定在最后）。
                                        let paths =
                                            cx.global::<AppSettings>().project_paths.clone();
                                        let items = dir_option_items(&paths, Some(&path));
                                        let custom_index = IndexPath::new(items.len() - 2);
                                        this.dir_select.update(cx, |state, cx| {
                                            state.set_items(items, window, cx);
                                            state.set_selected_index(
                                                Some(custom_index),
                                                window,
                                                cx,
                                            );
                                        });
                                        cx.notify();
                                    });
                                })
                                .detach();
                        });
                    }
                    SelectEvent::Confirm(None) => {
                        this.selected_dir = None;
                    }
                }
            },
        );

        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(version.id.clone())
                .placeholder("项目名称")
        });

        // 加载器下拉框：默认原版，选 Forge 后后台拉取该 MC 版本的 Forge 列表。
        let loader_select = cx.new(|cx| {
            SelectState::new(
                vec![
                    LoaderItem {
                        kind: LoaderKind::Vanilla,
                        label: SharedString::from("原版"),
                    },
                    LoaderItem {
                        kind: LoaderKind::Forge,
                        label: SharedString::from("Forge"),
                    },
                ],
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let loader_subscription = cx.subscribe_in(
            &loader_select,
            window,
            |this: &mut Self, _state, event: &SelectEvent<Vec<LoaderItem>>, window, cx| {
                let SelectEvent::Confirm(kind) = event;
                match kind {
                    Some(LoaderKind::Forge) => {
                        if this.loader != LoaderKind::Forge {
                            this.loader = LoaderKind::Forge;
                            this.load_forge_versions(window, cx);
                        }
                    }
                    Some(LoaderKind::Vanilla) | None => {
                        this.loader = LoaderKind::Vanilla;
                    }
                }
                cx.notify();
            },
        );

        // Forge 版本下拉框：选中 Forge 后开始后台请求，这里先放空列表。
        let forge_select = cx.new(|cx| {
            SelectState::new(Vec::new(), None, window, cx).searchable(true)
        });
        let forge_subscription = cx.subscribe_in(
            &forge_select,
            window,
            |this: &mut Self, _state, event: &SelectEvent<Vec<ForgeVersionItem>>, _window, cx| {
                let SelectEvent::Confirm(value) = event;
                this.selected_forge = value.clone();
                cx.notify();
            },
        );

        Self {
            version,
            dir_select,
            selected_dir,
            custom_dir: None,
            name_input,
            loader_select,
            loader: LoaderKind::Vanilla,
            forge_select,
            selected_forge: None,
            forge_list: ForgeList::Idle,
            steps: &INSTALL_STEPS,
            form_error: None,
            status: Arc::new(Mutex::new(None)),
            progress: None,
            error: Arc::new(Mutex::new(None)),
            installed_name: None,
            focus_handle: cx.focus_handle(),
            _dir_subscription: dir_subscription,
            _loader_subscription: loader_subscription,
            _forge_subscription: forge_subscription,
        }
    }

    /// 后台请求该 MC 版本的 Forge 列表，成功后填充版本下拉框并默认选中第一项。
    fn load_forge_versions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forge_list = ForgeList::Loading;
        let mc_version = self.version.id.clone();
        cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { forge::ForgeVersions::get_versions(mc_version) })
                .await;
            let _ = view.update_in(cx, |this, window, cx| match result {
                Ok(list) => {
                    // 默认选中列表第一项（通常是最新的 Forge 版本）。
                    this.selected_forge = list
                        .versions
                        .first()
                        .map(|version| version.version.clone());
                    let items: Vec<_> = list
                        .versions
                        .iter()
                        .map(|version| ForgeVersionItem {
                            version: version.version.clone(),
                        })
                        .collect();
                    let index = (!items.is_empty()).then(|| IndexPath::new(0));
                    this.forge_list = ForgeList::Loaded(list.versions);
                    this.forge_select.update(cx, |state, cx| {
                        state.set_items(items, window, cx);
                        state.set_selected_index(index, window, cx);
                    });
                    cx.notify();
                }
                Err(error) => {
                    this.forge_list = ForgeList::Failed(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 给安装处理器挑一个 Java：优先全局设置里指定的，其次是默认项，最后取扫描到的第一个。
    fn resolve_java(&self, cx: &App) -> Result<JavaVersion, String> {
        let settings = cx.global::<AppSettings>();
        settings
            .global_settings
            .java
            .clone()
            .or_else(|| {
                settings
                    .default_java_version
                    .and_then(|ix| settings.java_versions.get(ix).cloned())
            })
            .or_else(|| settings.java_versions.first().cloned())
            .ok_or_else(|| "未找到可用的 Java，请先在设置中添加 Java".to_owned())
    }

    /// 下拉框里有多少项。
    fn dir_item_count(&self, cx: &App) -> usize {
        let paths = cx.global::<AppSettings>().project_paths.len();
        paths + if self.custom_dir.is_some() { 1 } else { 0 } + 1
    }

    /// 表单阶段解析出来的安装目录；没选版本目录也没有自定义目录时报错。
    fn resolve_root(&self, cx: &App) -> Result<PathBuf, String> {
        match &self.selected_dir {
            Some(DirOption::Project(ix)) => cx
                .global::<AppSettings>()
                .project_paths
                .get(*ix)
                .cloned()
                .ok_or_else(|| "选择的版本目录已不存在".to_owned()),
            Some(DirOption::Custom(path)) => Ok(path.clone()),
            Some(DirOption::Browse) | None => Err("请选择安装目录".to_owned()),
        }
    }

    /// 解析项目名称：输入框留空时用版本号；非法字符直接拦下来。
    fn resolve_name(&self, cx: &App) -> Result<String, String> {
        let raw = self.name_input.read(cx).value();
        let name = raw.trim();
        let name = if name.is_empty() {
            self.version.id.as_str()
        } else {
            name
        };

        if name.is_empty()
            || name.starts_with('.')
            || name.contains("..")
            || name.chars().any(|c| c == '/' || c == '\\')
        {
            return Err(format!("无效的项目名称: {name}"));
        }
        Ok(name.to_owned())
    }

    fn is_running(&self) -> bool {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|status| matches!(status, InstallStatus::Running(_)))
    }

    /// 该步骤是否对应一个文件下载阶段（有明细面板可展示）。
    fn is_download_step(&self, index: usize) -> bool {
        if self.loader == LoaderKind::Forge {
            // 下载运行文件 / 下载支持库 / 下载资源文件
            matches!(index, 2..=4)
        } else {
            // 下载运行文件 / 下载资源文件
            matches!(index, 1 | 2)
        }
    }

    /// 步骤下标对应的下载器；该步骤不是下载阶段或下载器尚未创建时返回 `None`。
    fn step_downloader(&self, index: usize) -> Option<Arc<Downloader>> {
        let progress = self.progress.as_ref()?;
        if self.loader == LoaderKind::Forge {
            match index {
                2 => progress.jar_downloader(),
                3 => progress.libraries_downloader(),
                4 => progress.assets_downloader(),
                _ => None,
            }
        } else {
            match index {
                1 => progress.jar_downloader(),
                2 => progress.assets_downloader(),
                _ => None,
            }
        }
    }

    /// 该步骤是否是资源文件下载（文件名是 hash，需要用逻辑路径映射展示）。
    fn is_assets_step(&self, index: usize) -> bool {
        if self.loader == LoaderKind::Forge {
            index == 4
        } else {
            index == 2
        }
    }

    /// 点「下载」：校验表单，然后交给 `InstallProgress::install_minecraft`
    /// 或 `install_forge` 在新线程中执行；这里只起一个轮询任务同步界面状态。
    fn start_install(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.progress.is_some() {
            return;
        }

        let (root, name) = match (self.resolve_root(cx), self.resolve_name(cx)) {
            (Ok(root), Ok(name)) => (root, name),
            (Err(error), _) | (_, Err(error)) => {
                self.form_error = Some(error);
                cx.notify();
                return;
            }
        };

        if root.join(&name).exists() {
            self.form_error = Some(format!("目录已存在: {}", root.join(&name).display()));
            cx.notify();
            return;
        }

        // 选了 Forge 才需要版本号和 Java；校验失败就直接展示在表单上。
        let forge_version = if self.loader == LoaderKind::Forge {
            let selected = match &self.selected_forge {
                Some(version) => version.clone(),
                None => {
                    self.form_error = Some(match &self.forge_list {
                        ForgeList::Loading => "Forge 版本列表加载中，请稍候".to_owned(),
                        ForgeList::Failed(error) => format!("获取 Forge 版本失败：{error}"),
                        _ => "请选择 Forge 版本".to_owned(),
                    });
                    cx.notify();
                    return;
                }
            };
            let ForgeList::Loaded(versions) = &self.forge_list else {
                self.form_error = Some("请选择 Forge 版本".to_owned());
                cx.notify();
                return;
            };
            match versions.iter().find(|version| version.version == selected) {
                Some(version) => Some(version.clone()),
                None => {
                    self.form_error = Some("选择的 Forge 版本已失效，请重新选择".to_owned());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let java = if forge_version.is_some() {
            match self.resolve_java(cx) {
                Ok(java) => Some(java),
                Err(error) => {
                    self.form_error = Some(error);
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };

        self.form_error = None;
        let is_forge = forge_version.is_some();
        self.steps = if is_forge {
            &FORGE_STEPS
        } else {
            &INSTALL_STEPS
        };
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(InstallStatus::Running(0));
        // 支持库和资源文件放进全局目录，和启动阶段读取的位置一致。
        let settings = &cx.global::<AppSettings>().global_settings;
        let libraries = settings.libraries_path.clone();
        let assets = settings.assets_path.clone();
        let progress = match (forge_version, java) {
            (Some(version), Some(java)) => {
                install_forge(&name, &root, &libraries, &assets, java, &version)
            }
            _ => install_minecraft(&name, &root, &assets, &self.version),
        };
        self.progress = Some(progress.clone());

        // 轮询安装状态：映射到步骤下标并刷新界面，结束后写入结果。
        cx.spawn_in(_window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let keep_polling = view
                    .update_in(cx, |dialog, _window, cx| {
                        let state = progress.state();
                        match state {
                            InstallState::Success => {
                                *dialog.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(InstallStatus::Done);
                                if let Some(project) = progress.success() {
                                    dialog.installed_name = Some(project.name);
                                }
                                // 磁盘上多了一个项目，让启动页/VCS 页/版本管理页重扫。
                                cx.global_mut::<ProjectsRevision>().bump();
                                cx.notify();
                                return false;
                            }
                            InstallState::Failed => {
                                // 标记当前进行中的步骤为失败。
                                let step = match *dialog
                                    .status
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                {
                                    Some(InstallStatus::Running(step)) => step,
                                    _ => dialog.steps.len() - 1,
                                };
                                if let Some(error) = progress.error() {
                                    *dialog.error.lock().unwrap_or_else(|e| e.into_inner()) =
                                        Some(error.to_string());
                                }
                                *dialog.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(InstallStatus::Failed(step));
                                cx.notify();
                                return false;
                            }
                            state => {
                                *dialog.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(InstallStatus::Running(step_index(
                                        state, is_forge,
                                    )));
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn open(dialog: Entity<Self>, window: &mut Window, cx: &mut App) {
        if window.has_active_dialog(cx) {
            return;
        }
        let focus = dialog.read(cx).focus_handle.clone();
        let version_id = dialog.read(cx).version.id.clone();
        window.open_dialog(cx, move |builder, window, cx| {
            let view = dialog.read(cx);
            let running = view.is_running();
            let is_forge = view.loader == LoaderKind::Forge;
            let finished = view
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|status| matches!(status, InstallStatus::Done | InstallStatus::Failed(_)));

            builder
                .title(format!("下载 · {version_id}"))
                .overlay(true)
                .overlay_closable(!running)
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
                                "关闭弹窗后下载会继续进行"
                            } else if finished {
                                "下载流程已结束"
                            } else if is_forge {
                                "安装器会自动下载 Forge 支持库"
                            } else {
                                "支持库将在首次启动时下载"
                            })
                            .text_sm(),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new("download-dialog-cancel")
                                        .label(if finished { "关闭" } else { "取消" })
                                        .disabled(running)
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("download-dialog-confirm")
                                        .label("下载")
                                        .icon(IconName::ArrowDown)
                                        .loading(running)
                                        .disabled(running || finished)
                                        .on_click({
                                            let dialog = dialog.clone();
                                            move |_, window, cx| {
                                                let _ = dialog.update(cx, |dialog, cx| {
                                                    dialog.start_install(window, cx);
                                                });
                                            }
                                        }),
                                ),
                        ),
                )
                .child(dialog.clone())
        });
        focus.focus(window, cx);
    }
}

impl Render for DownloadDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = *self.status.lock().unwrap_or_else(|e| e.into_inner());
        let error = self.error.lock().unwrap_or_else(|e| e.into_inner()).clone();

        let mut content = div()
            .id("download-dialog-content")
            .track_focus(&self.focus_handle)
            .v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .pb_2();

        match status {
            // 表单阶段：安装目录 + 加载器 + 项目名称。
            None => {
                content = content
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(Label::new("安装目录").text_sm())
                            .child(
                                Select::new(&self.dir_select)
                                    .w_full()
                                    .placeholder("选择安装目录"),
                            ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(Label::new("加载器").text_sm())
                            .child(
                                div()
                                    .h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(
                                        Select::new(&self.loader_select)
                                            .w(px(120.))
                                            .placeholder("原版"),
                                    )
                                    .when(self.loader == LoaderKind::Forge, |row| {
                                        let placeholder = match &self.forge_list {
                                            ForgeList::Loading => "正在获取 Forge 版本…",
                                            ForgeList::Failed(_) => "获取失败",
                                            _ => "选择 Forge 版本",
                                        };
                                        row.child(
                                            Select::new(&self.forge_select)
                                                .flex_1()
                                                .w_full()
                                                .placeholder(placeholder)
                                                .disabled(matches!(
                                                    self.forge_list,
                                                    ForgeList::Loading
                                                        | ForgeList::Failed(_)
                                                )),
                                        )
                                        .when(
                                            matches!(self.forge_list, ForgeList::Failed(_)),
                                            |row| {
                                                row.child(
                                                    Button::new("retry-forge-versions")
                                                        .label("重试")
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.load_forge_versions(
                                                                    window, cx,
                                                                );
                                                            },
                                                        )),
                                                )
                                            },
                                        )
                                    }),
                            ),
                    )
                    .when_some(
                        match &self.forge_list {
                            ForgeList::Failed(error) => Some(error.clone()),
                            _ => None,
                        },
                        |content, error| {
                            content.child(
                                Label::new(format!("获取 Forge 版本失败：{error}"))
                                    .text_sm()
                                    .text_color(cx.theme().danger),
                            )
                        },
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(Label::new("项目名称").text_sm())
                            .child(Input::new(&self.name_input).w_full()),
                    );
                if let Some(error) = &self.form_error {
                    content = content.child(
                        Label::new(error.clone())
                            .text_sm()
                            .text_color(cx.theme().danger),
                    );
                }
            }
            // 进度阶段：和启动弹窗一样的步骤列表，下载步骤附带明细面板。
            Some(status) => {
                let asset_names = self
                    .progress
                    .as_ref()
                    .and_then(InstallProgress::asset_names);
                for index in 0..self.steps.len() {
                    let step = step_status(index, status);
                    content = content.child(step_marker(index, self.steps[index], step, cx));
                    let downloader = self.step_downloader(index);
                    if self.is_download_step(index)
                        && (downloader.is_some()
                            || matches!(step, StepStatus::Active | StepStatus::Failed))
                    {
                        let names = if self.is_assets_step(index) {
                            asset_names.as_deref()
                        } else {
                            None
                        };
                        content = content.child(download_panel(
                            index,
                            downloader.as_deref(),
                            names,
                            step,
                            cx,
                        ));
                    }
                    if step == StepStatus::Failed
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
                if status == InstallStatus::Done {
                    content = content.child(
                        div().pl_6().child(
                            Label::new(format!(
                                "已安装 {}",
                                self.installed_name.as_deref().unwrap_or(&self.version.id)
                            ))
                            .text_sm(),
                        ),
                    );
                }
            }
        }

        content
    }
}
