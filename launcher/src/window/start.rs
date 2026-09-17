use gpui_kit::component::button::Button;
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::{Sizable, StyledExt, WindowExt};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, px,
};

use mclib::launch::{LaunchInfo, Launcher};

use crate::data::account_data::AccountData;
use crate::data::settings::AppSettings;
use crate::window::project_select::{ProjectList, ProjectSelect};

mod launch_dialog;
use launch_dialog::LaunchDialog;

pub struct StartPage {
    /// 项目下拉框，和 VCS 页共用一份实现；选中项写回设置，两个页面自然同步。
    project_select: ProjectSelect,
    /// 保活下拉框的事件订阅。
    _select_subscription: Subscription,
    /// 保留正在进行的启动任务，关闭弹窗后可以重新查看。
    launch_dialog: Option<Entity<LaunchDialog>>,
    _launch_subscription: Option<Subscription>,
}

impl StartPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project_select = ProjectSelect::new(window, cx);

        let _select_subscription = cx.subscribe_in(
            project_select.state(),
            window,
            |this: &mut Self, _state, event: &SelectEvent<ProjectList>, _window, cx| {
                let SelectEvent::Confirm(Some(path)) = event else {
                    return;
                };

                // 写回设置由下拉框自己负责，VCS 页下次渲染就会跟上。
                if !this.project_select.confirm(path.clone(), cx) {
                    return;
                }

                cx.notify();
            },
        );

        Self {
            project_select,
            _select_subscription,
            launch_dialog: None,
            _launch_subscription: None,
        }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        if let Some(dialog) = &self.launch_dialog
            && dialog.read(cx).is_active()
        {
            LaunchDialog::open(dialog.clone(), window, cx);
            return;
        }

        let project = self.project_select.selected_project().cloned();
        let title = project
            .as_ref()
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "启动游戏".into());
        let account = cx.global::<AccountData>().get_default_account().cloned();
        let launch = match (project, account) {
            (None, _) => Err("请先选择要启动的项目；没有项目时，请在设置中添加版本目录。".into()),
            (_, None) => Err("请先在设置中添加账号，并将要使用的账号设为默认账号。".into()),
            (Some(project), Some(account)) => {
                let settings = cx.global::<AppSettings>();
                Ok(LaunchInfo::launch(Launcher::new(
                    account,
                    project,
                    settings.java_versions.clone(),
                    settings.global_settings.clone(),
                )))
            }
        };
        let dialog = cx.new(|cx| LaunchDialog::new(title, launch, window, cx));
        self._launch_subscription = Some(cx.observe(&dialog, |_, _, cx| cx.notify()));
        self.launch_dialog = Some(dialog.clone());
        LaunchDialog::open(dialog, window, cx);
        cx.notify();
    }
}

impl Render for StartPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 让下拉框跟上设置：设置页可能加过/删过版本目录，VCS 页也可能换过当前项目。
        self.project_select.sync(window, cx);

        let active = self
            .launch_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.read(cx).is_active());

        div().v_flex().size_full().justify_end().child(
            div()
                .v_flex()
                .gap_2()
                .w_full()
                .px(px(15.))
                .pb(px(15.))
                .child(
                    div().h_flex().justify_end().child(
                        Button::new("launch")
                            .label(if active {
                                "查看启动进度"
                            } else {
                                "启动游戏"
                            })
                            .large()
                            .h(px(60.))
                            .px(px(40.))
                            .text_3xl()
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    ),
                )
                .child(
                    div().w_full().child(
                        Select::new(self.project_select.state())
                            .disabled(false)
                            .large()
                            .h(px(48.))
                            .placeholder("选择要启动的项目"),
                    ),
                ),
        )
    }
}
