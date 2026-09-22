//! 原版下载弹窗：表单阶段选安装目录、填项目名称；
//! 点下载后切到和启动弹窗一样的步骤进度展示。
//!
//! 下载由 `InstallProgress::install_minecraft` 在新线程中执行：
//! 拉清单并写入 `<name>.json`，下载客户端 jar，最后生成项目文件；
//! 依赖库留到首次启动时再拉。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::marker::{Marker, MarkerContent, MarkerIcon, MarkerLoadingStyle};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, StyledExt, WindowExt,
};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Window, div, px,
};
use rfd::AsyncFileDialog;

use mclib::project::install::progress::{InstallProgress, InstallState};
use mclib::project::versions::minecreft::Version;

use crate::data::app_data::ProjectsRevision;
use crate::data::settings::AppSettings;

const INSTALL_STEPS: [&str; 3] = ["下载版本清单", "下载运行文件", "写入项目文件"];

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

fn step_marker(index: usize, status: StepStatus, cx: &App) -> Marker {
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
        .content(MarkerContent::new().text_color(color).text(INSTALL_STEPS[index]))
        .child(div().flex_1())
        .child(Label::new(text).text_sm())
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

        Self {
            version,
            dir_select,
            selected_dir,
            custom_dir: None,
            name_input,
            form_error: None,
            status: Arc::new(Mutex::new(None)),
            progress: None,
            error: Arc::new(Mutex::new(None)),
            installed_name: None,
            focus_handle: cx.focus_handle(),
            _dir_subscription: dir_subscription,
        }
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

    /// 点「下载」：校验表单，然后交给 `InstallProgress::install_minecraft`
    /// 在新线程中执行；这里只起一个轮询任务同步界面状态。
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

        self.form_error = None;
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(InstallStatus::Running(0));
        let progress = InstallProgress::install_minecraft(&name, &root, &self.version);
        self.progress = Some(progress.clone());

        // 轮询安装状态：映射到步骤下标并刷新界面，结束后写入结果。
        cx.spawn_in(_window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let keep_polling = view
                    .update_in(cx, |dialog, _window, cx| {
                        match progress.state() {
                            InstallState::Ready | InstallState::DownloadVersionJson => {
                                *dialog.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(InstallStatus::Running(0));
                            }
                            InstallState::InstallJar => {
                                *dialog.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(InstallStatus::Running(1));
                            }
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
                                    _ => INSTALL_STEPS.len() - 1,
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
                            } else {
                                "游戏文件将在首次启动时下载"
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
            // 表单阶段：安装目录 + 项目名称。
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
            // 进度阶段：和启动弹窗一样的步骤列表。
            Some(status) => {
                for index in 0..INSTALL_STEPS.len() {
                    let step = step_status(index, status);
                    content = content.child(step_marker(index, step, cx));
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
