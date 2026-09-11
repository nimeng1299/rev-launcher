use crate::data::account_data::AccountData;
use crate::window::develop::DevelopPage;
use crate::window::download::DownloadPage;
use crate::window::settings::SettingsPage;
use crate::window::start::StartPage;
use crate::window::version_control::{VersionControlEvent, VersionControlPage};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{button::*, *};
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Window, div, px,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum MainPageTab {
    #[default]
    Start = 0,
    Download = 1,
    VersionControl = 2,
    Develop = 3,
    Settings = 4,
}

impl MainPageTab {
    fn from_index(index: usize) -> Self {
        match index {
            0 => MainPageTab::Start,
            1 => MainPageTab::Download,
            2 => MainPageTab::VersionControl,
            3 => MainPageTab::Develop,
            4 => MainPageTab::Settings,
            _ => MainPageTab::Start,
        }
    }
}

pub struct MainWindow {
    active_tab: MainPageTab,
    start_page: Option<Entity<StartPage>>,
    download_page: Option<Entity<DownloadPage>>,
    version_control_page: Option<Entity<VersionControlPage>>,
    develop_page: Option<Entity<DevelopPage>>,
    settings_page: Option<Entity<SettingsPage>>,
}

impl MainWindow {
    fn set_active_tab(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        self.active_tab = MainPageTab::from_index(index);
        cx.notify();
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let start_page = self
            .start_page
            .get_or_insert_with(|| cx.new(|cx| StartPage::new(window, cx)))
            .clone();
        let download_page = self
            .download_page
            .get_or_insert_with(|| cx.new(|cx| DownloadPage::new(window, cx)))
            .clone();
        let version_control_page = self
            .version_control_page
            .get_or_insert_with(|| {
                let page = cx.new(|cx| VersionControlPage::new(window, cx));

                // 版本管理页请求跳转（下拉框里的「路径管理」）时，切到设置的项目管理页。
                cx.subscribe_in(
                    &page,
                    window,
                    |this: &mut MainWindow, _page, event: &VersionControlEvent, window, cx| {
                        let VersionControlEvent::OpenProjectPathManager = *event;

                        let settings_page = this
                            .settings_page
                            .get_or_insert_with(|| cx.new(|cx| SettingsPage::new(window, cx)))
                            .clone();
                        settings_page.update(cx, |page, cx| {
                            page.select_page(SettingsPage::PROJECT_PAGE_INDEX, cx);
                        });

                        this.set_active_tab(MainPageTab::Settings as usize, window, cx);
                    },
                )
                .detach();

                page
            })
            .clone();
        let develop_page = self
            .develop_page
            .get_or_insert_with(|| cx.new(|cx| DevelopPage::new(window, cx)))
            .clone();

        let settings_page = self
            .settings_page
            .get_or_insert_with(|| cx.new(|cx| SettingsPage::new(window, cx)))
            .clone();

        div()
            .v_flex()
            .gap_2()
            .size_full()
            .child(
                TitleBar::new()
                    .child(div().flex().items_center().gap_3().child("rev launcher"))
                    .child(
                        div()
                            .flex()
                            .occlude() // 标题栏的按钮必须加这个，不然不生效
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("profile")
                                    .icon(IconName::User)
                                    .label({
                                        let account = cx.global::<AccountData>();
                                        if let Some(account) = account.get_default_account() {
                                            account.name.clone()
                                        } else {
                                            "添加账号".to_string()
                                        }
                                    })
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        // 头像按钮直接跳到设置里的账号管理页
                                        let settings_page = this
                                            .settings_page
                                            .get_or_insert_with(|| {
                                                cx.new(|cx| SettingsPage::new(window, cx))
                                            })
                                            .clone();
                                        settings_page.update(cx, |page, cx| {
                                            page.select_page(SettingsPage::ACCOUNT_PAGE_INDEX, cx);
                                        });
                                        this.set_active_tab(
                                            MainPageTab::Settings as usize,
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .flex_1() // 让内容区撑满窗口剩余高度，页面才能把工具栏钉在最下面
                    .items_start()
                    .justify_start()
                    .mx(px(15.))
                    .gap_5()
                    .child(
                        TabBar::new("underline-tabs")
                            .underline()
                            .selected_index(self.active_tab as usize)
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                this.set_active_tab(*ix, window, cx);
                            }))
                            .child(Tab::new().label("启动"))
                            .child(Tab::new().label("下载"))
                            .child(Tab::new().label("版本管理"))
                            .child(Tab::new().label("开发"))
                            .child(Tab::new().label("设置")),
                    )
                    .child(div().flex_1().w_full().child(match self.active_tab {
                        MainPageTab::Start => start_page.into_any_element(),
                        MainPageTab::Download => download_page.into_any_element(),
                        MainPageTab::VersionControl => version_control_page.into_any_element(),
                        MainPageTab::Develop => develop_page.into_any_element(),
                        MainPageTab::Settings => settings_page.into_any_element(),
                    })),
            )
            // 通知层画在对话框层之上，而通知本身是 occlude 的（会吞掉鼠标事件）。
            // 所以只要对话框打开时弹通知，通知就可能盖住对话框右上角的关闭按钮，
            // 让对话框关不掉（鼠标悬在通知上还会暂停它的自动消失）。
            // 对话框内的反馈请直接画在对话框里，不要用 push_notification。
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

impl Default for MainWindow {
    fn default() -> Self {
        Self {
            active_tab: MainPageTab::default(),
            start_page: None,
            download_page: None,
            version_control_page: None,
            develop_page: None,
            settings_page: None,
        }
    }
}
