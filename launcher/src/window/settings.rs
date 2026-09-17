pub mod account_manager;
pub mod game_settings;
pub mod java_manager;
pub mod jj_settings;
pub mod project_manager;

use crate::data::settings::AppSettings;
use crate::window::settings::account_manager::AccountManager;
use crate::window::settings::game_settings::global_game_settings_group;
use crate::window::settings::java_manager::JavaManager;
use crate::window::settings::jj_settings::jj_settings_group;
use crate::window::settings::project_manager::ProjectManager;
use gpui_kit::component::StyledExt;
use gpui_kit::component::setting::{
    SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::{
    AppContext, Background, Context, Entity, Fill, Hsla, IntoElement, ParentElement, Render,
    SharedString, StyleRefinement, Styled, Window, div,
};

pub struct SettingsPage {
    project_manager: Option<Entity<ProjectManager>>,
    java_manager: Option<Entity<JavaManager>>,
    account_manager: Option<Entity<AccountManager>>,
    /// 外部（比如标题栏的头像按钮）请求显示的设置页下标。
    requested_page: usize,
    /// 外部请求切页的次数。
    ///
    /// `Settings` 内部用 `window.use_keyed_state` 记住当前选中的页，并且只在状态首次
    /// 创建时读取 `default_selected_index`，所以只改 `requested_page` 不会真的切页。
    /// 把次数拼进 `Settings` 的 ElementId，可以让它丢掉旧状态、按新的默认页重建。
    selection_revision: u64,
}

impl SettingsPage {
    /// 「项目管理」页在设置页列表中的下标。
    pub const PROJECT_PAGE_INDEX: usize = 1;
    /// 「账号管理」页在设置页列表中的下标。
    pub const ACCOUNT_PAGE_INDEX: usize = 3;

    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            project_manager: None,
            java_manager: None,
            account_manager: None,
            requested_page: 0,
            selection_revision: 0,
        }
    }

    /// 请求切换到指定设置页，下一次渲染时生效。
    pub fn select_page(&mut self, page_ix: usize, cx: &mut Context<Self>) {
        self.requested_page = page_ix;
        self.selection_revision += 1;
        cx.notify();
    }
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let project_manager_page = self
            .project_manager
            .get_or_insert_with(|| cx.new(|_cx| ProjectManager::default()))
            .clone();

        let java_manager_page = self
            .java_manager
            .get_or_insert_with(|| cx.new(|_cx| JavaManager::default()))
            .clone();

        let account_manager_page = self
            .account_manager
            .get_or_insert_with(|| cx.new(|_cx| AccountManager::default()))
            .clone();

        let sidebar_style = StyleRefinement {
            background: Some(Fill::Color(Background::from(Hsla::transparent_black()))),
            ..StyleRefinement::default()
        };

        let _settings = cx.global::<AppSettings>();

        let options = vec![
            (SharedString::from("option1"), SharedString::from("选项一")),
            (SharedString::from("option2"), SharedString::from("选项二")),
            (SharedString::from("option3"), SharedString::from("选项三")),
        ];

        let settings_id = format!("launcher_settings-{}", self.selection_revision);

        div()
            .h_flex()
            .size_full()
            .justify_center()
            .items_center()
            .gap_2()
            .child(
                Settings::new(settings_id)
                    .default_selected_index(SelectIndex {
                        page_ix: self.requested_page,
                        group_ix: None,
                    })
                    .sidebar_style(&sidebar_style)
                    .pages(vec![
                        SettingPage::new("通用")
                            .group(SettingGroup::new().title("Basic Options").item(
                                SettingItem::new(
                                    "Enable Feature",
                                    SettingField::dropdown(
                                        options.clone(),
                                        |_cx| SharedString::from("aaa"),
                                        |_value, _cx| {},
                                    ),
                                ),
                            ))
                            .group(jj_settings_group()),
                        SettingPage::new("项目管理").group(SettingGroup::new().item(
                            SettingItem::render(move |_options, _window, _app| {
                                project_manager_page.clone().into_any_element()
                            }),
                        )),
                        SettingPage::new("Java管理").group(SettingGroup::new().item(
                            SettingItem::render(move |_options, _window, _app| {
                                java_manager_page.clone().into_any_element()
                            }),
                        )),
                        SettingPage::new("账号管理").group(SettingGroup::new().item(
                            SettingItem::render(move |_options, _window, _app| {
                                account_manager_page.clone().into_any_element()
                            }),
                        )),
                        SettingPage::new("游戏设置").group(global_game_settings_group(cx)),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jj::user_config;
    use gpui_kit::component::Root;
    use gpui_kit::{App, TestAppContext, WindowHandle};
    use std::path::PathBuf;

    /// 只渲染「jj - Jujutsu」分组的宿主，用来把设置组件真的跑一遍。
    struct JjSettingsHost;

    impl Render for JjSettingsHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                Settings::new("test-jj-settings")
                    .pages(vec![SettingPage::new("General").group(jj_settings_group())]),
            )
        }
    }

    /// 在窗口里跑一段逻辑。
    ///
    /// 不能用 `WindowHandle::update`：那会把 `Root` 压在 update 栈上，而设置组件内部的
    /// `use_keyed_state` 等都要走 `App::update_window`。
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

    /// 渲染「jj - Jujutsu」分组，确认两个输入框的取值/写回闭包能真的跑起来：
    /// 字段类型和 `SettingField::input` 对不上时，`StringField` 里的 downcast 会直接 panic。
    /// 顺带守住一个前提：仅仅渲染这个页面，不能往用户的 jj 用户配置里写东西。
    #[gpui_kit::test]
    fn rendering_jj_group_leaves_user_config_untouched(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);

        let config_path: Option<PathBuf> = user_config::user_config_path();
        let before = config_path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok());

        let window = cx.add_window(|window, cx| {
            let view = cx.new(|_| JjSettingsHost);
            Root::new(view, window, cx)
        });

        // 强制走一遍完整的渲染（布局 + 绘制），把分组里的输入框都建出来。
        with_window(cx, &window, |window, cx| {
            let _ = window.draw(cx);
        });
        cx.run_until_parked();

        let after = config_path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        assert_eq!(before, after, "只渲染设置页不该改写 jj 用户配置");
    }
}
