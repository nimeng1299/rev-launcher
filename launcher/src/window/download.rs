use gpui_kit::component::StyledExt;
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct DownloadPage {}

impl DownloadPage {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {}
    }
}

impl Render for DownloadPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h_flex()
            .size_full()
            .justify_center()
            .items_center()
            .gap_2()
            .child(
                Sidebar::new("download_sidebar")
                    .child(
                        SidebarGroup::new("游戏下载").child(
                            SidebarMenu::new()
                                .child(
                                    SidebarMenuItem::new("原版下载")
                                        .on_click(|_, _, _| println!("Dashboard clicked")),
                                )
                                .child(
                                    SidebarMenuItem::new("整合包下载")
                                        .on_click(|_, _, _| println!("Settings clicked")),
                                ),
                        ),
                    )
                    .child(
                        SidebarGroup::new("资源下载").child(
                            SidebarMenu::new()
                                .child(
                                    SidebarMenuItem::new("模组下载")
                                        .on_click(|_, _, _| println!("Dashboard clicked")),
                                )
                                .child(
                                    SidebarMenuItem::new("资源包下载")
                                        .on_click(|_, _, _| println!("Settings clicked")),
                                )
                                .child(
                                    SidebarMenuItem::new("光影下载")
                                        .on_click(|_, _, _| println!("Settings clicked")),
                                ),
                        ),
                    ),
            )
            .child(div())
    }
}
