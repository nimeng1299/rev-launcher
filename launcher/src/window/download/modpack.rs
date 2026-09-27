//! 整合包安装页：从 Git 仓库克隆安装，或拖入整合包文件。
//!
//! 「从 Git 安装」先用 [`crate::jj::clone`] 把仓库克隆成项目目录，
//! 再接着调 `InstallProgress::install_form_folder`：按克隆下来的版本
//! JSON 里的加载器走对应的安装流程。拖入的整合包文件目前只记录在
//! 界面上，安装功能还没有实现。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, WindowExt,
};
use gpui_kit::{
    App, AppContext, Context, Entity, ExternalPaths, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Window, div, px,
};

use mclib::project::game_project::{ModLoader, get_game_project};
use mclib::project::install::install_form_folder;
use mclib::project::install::progress::{InstallProgress, InstallState};

use crate::data::app_data::ProjectsRevision;
use crate::data::settings::AppSettings;
use crate::jj;

use super::dialog::{
    FORGE_STEPS, INSTALL_STEPS, InstallStatus, StepStatus, download_panel, is_assets_step,
    is_download_step, resolve_java, step_downloader, step_index, step_marker, step_status,
    validate_project_name,
};
use super::dir_select::DirSelect;

/// 整合包安装页的内容区：仓库地址一行 + 文件拖放框。
pub(super) struct ModpackPage {
    /// 仓库地址输入框。
    url_input: Entity<InputState>,
    /// 拖进来的整合包文件；安装功能还没实现，先记下来展示。
    dropped_files: Vec<PathBuf>,
    focus_handle: FocusHandle,
}

impl ModpackPage {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            url_input: cx.new(|cx| InputState::new(window, cx).placeholder("整合包 Git 仓库地址")),
            dropped_files: Vec::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// 点「从 Git 安装」：校验地址后打开安装对话框。
    fn open_git_install_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let url = self.url_input.read(cx).value().trim().to_owned();
        if url.is_empty() {
            window.push_notification((NotificationType::Error, "请先输入仓库地址"), cx);
            return;
        }

        let dialog = cx.new(|cx| GitInstallDialog::new(url, window, cx));
        GitInstallDialog::open(dialog, window, cx);
    }

    /// 点「导入本地整合包」：占位，功能还没有实现。
    fn import_local_modpack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification(
            (NotificationType::Info, "本地整合包导入暂未实现，敬请期待"),
            cx,
        );
    }

    /// 文件拖放框：拖入整合包文件后把它们列出来；安装功能还没实现。
    fn render_drop_zone(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut zone = div()
            .id("modpack-drop-zone")
            .flex_1()
            .w_full()
            .min_h_16()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_2()
            .p_4()
            .border_1()
            .border_dashed()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                let files = paths.paths().to_vec();
                if files.is_empty() {
                    return;
                }
                this.dropped_files = files;
                cx.notify();
            }));

        if self.dropped_files.is_empty() {
            zone = zone
                .child(Icon::new(IconName::Inbox).large())
                .child(
                    Label::new("将整合包文件拖到这里")
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    Label::new("支持直接从压缩包安装（开发中）")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                );
        } else {
            zone = zone.child(Label::new(format!(
                "已接收 {} 个文件（安装功能暂未实现）",
                self.dropped_files.len()
            )));
            for path in &self.dropped_files {
                zone = zone.child(
                    div()
                        .max_w_full()
                        .truncate()
                        .child(Label::new(path.to_string_lossy().to_string()).text_sm()),
                );
            }
        }
        zone
    }
}

impl Render for ModpackPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 父容器给的是确定高度，这里用 size_full 铺满，拖放框才能撑开剩余空间。
        div()
            .id("modpack-page")
            .track_focus(&self.focus_handle)
            .v_flex()
            .size_full()
            .gap_3()
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Input::new(&self.url_input).flex_1().min_w_0(),
                    )
                    .child(
                        Button::new("modpack-git-install")
                            .icon(IconName::Github)
                            .label("从 Git 安装")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_git_install_dialog(window, cx);
                            })),
                    )
                    .child(
                        Button::new("modpack-local-import")
                            .icon(IconName::FolderOpen)
                            .label("导入本地整合包")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_local_modpack(window, cx);
                            })),
                    ),
            )
            .child(self.render_drop_zone(cx))
    }
}

/// 从 Git 仓库安装整合包的对话框：先克隆仓库，再按加载器安装。
struct GitInstallDialog {
    url: String,
    /// 项目名称输入框，留空时用从地址里猜出来的仓库名。
    name_input: Entity<InputState>,
    /// 安装目录选择框。
    dir_select: Entity<DirSelect>,
    /// 对话框所处阶段。
    phase: Phase,
    /// 安装步骤列表（不含克隆）；克隆完成后按加载器换成对应的一套。
    steps: &'static [&'static str],
    /// 安装是否是 Forge（决定步骤下标的映射和文件面板的展示）。
    is_forge: bool,
    /// 表单校验/克隆失败信息，直接画在对话框里（通知会盖住对话框）。
    form_error: Option<String>,
    /// 安装状态，由轮询任务写入；`None` 表示还没开始安装。
    status: Arc<Mutex<Option<InstallStatus>>>,
    /// 安装任务句柄。
    progress: Option<InstallProgress>,
    /// 安装失败信息。
    error: Arc<Mutex<Option<String>>>,
    /// 安装成功后创建出来的项目名，结果页展示用。
    installed_name: Option<String>,
    /// 克隆目标目录；点开始后才有。
    dest: Option<PathBuf>,
    focus_handle: FocusHandle,
}

/// 对话框所处阶段：表单 → 克隆中 → 安装中（安装的细节看 `status`）。
enum Phase {
    Form,
    Cloning,
    Installing,
}

impl GitInstallDialog {
    fn new(url: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(repo_name_from_url(&url).unwrap_or_default())
                .placeholder("项目名称")
        });
        let dir_select = cx.new(|cx| DirSelect::new(window, cx));

        Self {
            url,
            name_input,
            dir_select,
            phase: Phase::Form,
            steps: &INSTALL_STEPS,
            is_forge: false,
            form_error: None,
            status: Arc::new(Mutex::new(None)),
            progress: None,
            error: Arc::new(Mutex::new(None)),
            installed_name: None,
            dest: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(super) fn open(dialog: Entity<Self>, window: &mut Window, cx: &mut App) {
        if window.has_active_dialog(cx) {
            return;
        }
        let focus = dialog.read(cx).focus_handle.clone();
        window.open_dialog(cx, move |builder, window, cx| {
            let view = dialog.read(cx);
            let running = view.is_running();
            let finished = view.is_finished();

            builder
                .title("从 Git 安装整合包")
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
                                "关闭弹窗后操作会继续进行"
                            } else if finished {
                                "安装流程已结束"
                            } else {
                                "克隆完成后会按仓库里的版本 JSON 自动安装"
                            })
                            .text_sm(),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new("git-install-dialog-cancel")
                                        .label(if finished { "关闭" } else { "取消" })
                                        .disabled(running)
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("git-install-dialog-confirm")
                                        .label("克隆并安装")
                                        .icon(IconName::ArrowDown)
                                        .loading(running)
                                        .disabled(running || finished)
                                        .on_click({
                                            let dialog = dialog.clone();
                                            move |_, window, cx| {
                                                dialog.update(cx, |dialog, cx| {
                                                    dialog.start(window, cx);
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

    fn is_running(&self) -> bool {
        match self.phase {
            Phase::Cloning => true,
            Phase::Installing => self
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|status| matches!(status, InstallStatus::Running(_))),
            Phase::Form => false,
        }
    }

    fn is_finished(&self) -> bool {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|status| matches!(status, InstallStatus::Done | InstallStatus::Failed(_)))
    }

    /// 点「克隆并安装」：校验表单，克隆仓库，成功后接着安装。
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Form) {
            return;
        }

        let raw = self.name_input.read(cx).value().trim().to_owned();
        let name = if raw.is_empty() {
            match repo_name_from_url(&self.url) {
                Some(name) => name,
                None => {
                    self.form_error = Some("请输入项目名称".to_owned());
                    cx.notify();
                    return;
                }
            }
        } else if let Err(error) = validate_project_name(&raw) {
            self.form_error = Some(error);
            cx.notify();
            return;
        } else {
            raw
        };

        let Some(root) = self.dir_select.read(cx).selected().cloned() else {
            self.form_error = Some("请选择安装目录".to_owned());
            cx.notify();
            return;
        };
        let dest = root.join(&name);
        if dest.exists() {
            self.form_error = Some(format!("目录已存在: {}", dest.display()));
            cx.notify();
            return;
        }

        self.form_error = None;
        self.dest = Some(dest.clone());
        self.phase = Phase::Cloning;
        cx.notify();

        let url = self.url.clone();
        cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { jj::clone(&url, &dest) })
                .await;
            let _ = view.update_in(cx, |this, window, cx| match result {
                Ok(summary) => this.start_install(&summary, window, cx),
                Err(error) => {
                    // 回到表单让用户改地址重试；克隆失败时目标目录已被清理掉。
                    this.phase = Phase::Form;
                    this.form_error = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 克隆成功后接着安装：读项目信息判断加载器，交给
    /// `InstallProgress::install_form_folder` 在新线程执行，并起轮询任务。
    fn start_install(&mut self, summary: &str, window: &mut Window, cx: &mut Context<Self>) {
        let dest = self.dest.clone().expect("克隆完成后才调用");

        // 克隆下来的版本 JSON 决定加载器和步骤列表。
        let project = match get_game_project(&dest) {
            Ok(project) => project,
            Err(error) => {
                self.phase = Phase::Form;
                self.form_error = Some(format!("读取克隆下来的项目失败：{error}"));
                cx.notify();
                return;
            }
        };
        let is_forge = project.loader == ModLoader::Forge;
        let java = if is_forge {
            match resolve_java(cx) {
                Ok(java) => Some(java),
                Err(error) => {
                    self.phase = Phase::Form;
                    self.form_error = Some(error);
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };

        self.is_forge = is_forge;
        self.steps = if is_forge {
            &FORGE_STEPS
        } else {
            &INSTALL_STEPS
        };
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(InstallStatus::Running(0));
        self.phase = Phase::Installing;

        // 支持库和资源文件放进全局目录，和启动阶段读取的位置一致。
        let settings = &cx.global::<AppSettings>().global_settings;
        let libraries = settings.libraries_path.clone();
        let assets = settings.assets_path.clone();
        let progress = install_form_folder(&dest, &libraries, &assets, java);
        self.progress = Some(progress.clone());

        window.push_notification((NotificationType::Info, summary), cx);
        cx.notify();

        // 轮询安装状态：映射到步骤下标并刷新界面，结束后写入结果。
        cx.spawn_in(window, async move |view, cx| {
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
                                        state,
                                        dialog.is_forge,
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
}

impl Render for GitInstallDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id("git-install-dialog-content")
            .track_focus(&self.focus_handle)
            .v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .pb_2();

        match self.phase {
            // 表单阶段：仓库地址 + 项目名称 + 安装目录。
            Phase::Form => {
                content = content
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(Label::new("仓库地址").text_sm())
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(
                                        Label::new(self.url.clone())
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(Label::new("项目名称").text_sm())
                            .child(Input::new(&self.name_input).w_full()),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(Label::new("安装目录").text_sm())
                            .child(self.dir_select.clone()),
                    );
                if let Some(error) = &self.form_error {
                    content = content.child(
                        Label::new(error.clone())
                            .text_sm()
                            .text_color(cx.theme().danger),
                    );
                }
            }
            // 克隆阶段：一个转圈的步骤标记，地址太长时截断展示。
            Phase::Cloning => {
                content = content
                    .child(step_marker(0, "克隆仓库", StepStatus::Active, cx))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(
                                Label::new(format!("正在克隆 {} …", self.url))
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    );
            }
            // 安装阶段：克隆一步已完成，后面是和下载弹窗一样的步骤列表。
            Phase::Installing => {
                let status = self
                    .status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .unwrap_or(InstallStatus::Running(0));
                let error = self.error.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let asset_names = self
                    .progress
                    .as_ref()
                    .and_then(InstallProgress::asset_names);
                content = content.child(step_marker(0, "克隆仓库", StepStatus::Complete, cx));
                for index in 0..self.steps.len() {
                    let step = step_status(index, status);
                    content = content.child(step_marker(index + 1, self.steps[index], step, cx));
                    let downloader =
                        step_downloader(self.progress.as_ref(), self.is_forge, index);
                    if is_download_step(self.is_forge, index)
                        && (downloader.is_some()
                            || matches!(step, StepStatus::Active | StepStatus::Failed))
                    {
                        let names = if is_assets_step(self.is_forge, index) {
                            asset_names.as_deref()
                        } else {
                            None
                        };
                        content = content.child(download_panel(
                            index + 1,
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
                                self.installed_name.as_deref().unwrap_or("整合包")
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

/// 从仓库地址猜项目名：取最后一段路径，去掉 `.git` 后缀。
fn repo_name_from_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let tail = url.rsplit(['/', ':']).next()?;
    let name = tail.strip_suffix(".git").unwrap_or(tail).trim();
    if name.is_empty() || name.starts_with('.') || name.contains("..") {
        return None;
    }
    Some(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_from_url_extracts_last_segment() {
        assert_eq!(
            repo_name_from_url("https://example.com/group/MyPack.git"),
            Some("MyPack".to_owned())
        );
        assert_eq!(
            repo_name_from_url("git@example.com:group/pack/"),
            Some("pack".to_owned())
        );
        assert_eq!(repo_name_from_url("https://example.com/.git"), None);
        assert_eq!(repo_name_from_url("   "), None);
    }
}
