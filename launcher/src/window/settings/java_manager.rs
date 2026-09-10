use crate::data::settings::AppSettings;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{IndexPath, WindowExt};
use gpui_kit::component::{StyledExt, gray};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
};
use mclib::java::java_version::JavaVersion;
use rfd::AsyncFileDialog;

struct JavaListDelegate {
    selected_index: Option<IndexPath>,
}

impl ListDelegate for JavaListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, cx: &App) -> usize {
        cx.global::<AppSettings>().java_versions.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let app_setting = cx.global::<AppSettings>();
        app_setting.java_versions.get(ix.row).map(|item| {
            let body = if Some(ix.row) == app_setting.default_java_version {
                div()
                    .v_flex()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(item.version.clone()).text_2xl())
                            .child(
                                Tag::secondary()
                                    .outline()
                                    .child(item.java_runtime.to_string()),
                            )
                            .child(Tag::secondary().outline().child("默认")),
                    )
                    .child(
                        Label::new(item.path_buf.to_string_lossy())
                            .text_sm()
                            .text_color(gray(600)),
                    )
            } else {
                div()
                    .v_flex()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(item.version.clone()).text_2xl())
                            .child(
                                Tag::secondary()
                                    .outline()
                                    .child(item.java_runtime.to_string()),
                            ),
                    )
                    .child(
                        Label::new(item.path_buf.to_string_lossy())
                            .text_sm()
                            .text_color(gray(600)),
                    )
            };

            ListItem::new(ix)
                .child(body)
                .selected(Some(ix) == self.selected_index)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.delegate_mut().set_selected_index(Some(ix), window, cx);
                }))
        })
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
        cx.notify();
    }
}

#[derive(Default)]
pub struct JavaManager {
    state: Option<Entity<ListState<JavaListDelegate>>>,
}

impl JavaManager {}

impl Render for JavaManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self
            .state
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    let app_settings = cx.global::<AppSettings>();

                    ListState::new(
                        JavaListDelegate {
                            selected_index: match app_settings.default_java_version {
                                Some(i) => Some(IndexPath::new(i)),
                                None => None,
                            },
                        },
                        window,
                        cx,
                    )
                })
            })
            .clone();

        let max_height = window.bounds().size.height * 0.6;

        div()
            .v_flex()
            .flex_1()
            .w_full()
            .gap_5()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("当前添加的Java版本"))
                    .child(
                        ButtonGroup::new("btn-group")
                            .child(Button::new("btn1").label("添加").on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    let state = state.clone();
                                    window
                                        .spawn(cx, async move |cx| {
                                            let file = AsyncFileDialog::new()
                                                .add_filter("java", &["exe"])
                                                .set_title("Select an java.exe")
                                                .pick_file()
                                                .await;

                                            let Some(file) = file else {
                                                return; // 用户取消了选择
                                            };
                                            let path = file.path().to_path_buf();

                                            // 解析 Java 版本（会执行 `java -version`），放到后台线程，不卡界面
                                            let parsed = cx
                                                .background_executor()
                                                .spawn(async move { JavaVersion::from_path(&path) })
                                                .await;

                                            let _ = cx.update(|window, cx| match parsed {
                                                Ok(java) => {
                                                    let app_settings =
                                                        cx.global_mut::<AppSettings>();
                                                    if !app_settings
                                                        .java_versions
                                                        .iter()
                                                        .any(|j| j.path_buf == java.path_buf)
                                                    {
                                                        app_settings
                                                            .java_versions
                                                            .push(java.clone());
                                                    }
                                                    let _ = app_settings.save();
                                                    state.update(cx, |_, cx| cx.notify());
                                                    window.push_notification(
                                                        (
                                                            NotificationType::Success,
                                                            format!(
                                                                "成功添加Java版本: {}",
                                                                java.version
                                                            ),
                                                        ),
                                                        cx,
                                                    );
                                                }
                                                Err(err) => {
                                                    window.push_notification(
                                                        (
                                                            NotificationType::Error,
                                                            format!("无法解析Java: {}", err),
                                                        ),
                                                        cx,
                                                    );
                                                }
                                            });
                                        })
                                        .detach();
                                }
                            }))
                            .child(Button::new("btn2").label("删除").on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    let selected_row =
                                        state.read(cx).selected_index().map(|ix| ix.row);
                                    if let Some(row) = selected_row {
                                        let app_settings = cx.global_mut::<AppSettings>();
                                        let remove_version = app_settings.java_versions.remove(row);
                                        if let Some(id) = app_settings.default_java_version {
                                            if id == row {
                                                app_settings.default_java_version = None;
                                            } else if id > row {
                                                app_settings.default_java_version = Some(id - 1);
                                            }
                                        }
                                        let _ = cx.global::<AppSettings>().save();
                                        state.update(cx, |_, cx| cx.notify());
                                        window.push_notification(
                                            (
                                                NotificationType::Success,
                                                format!(
                                                    "成功删除Java版本: {}",
                                                    remove_version.version
                                                ),
                                            ),
                                            cx,
                                        );
                                    }
                                }
                            }))
                            .child(Button::new("btn3").label("设为默认").on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    let selected_row =
                                        state.read(cx).selected_index().map(|ix| ix.row);
                                    if let Some(row) = selected_row {
                                        cx.global_mut::<AppSettings>().default_java_version =
                                            Some(row);
                                        let _ = cx.global::<AppSettings>().save();
                                        state.update(cx, |_, cx| cx.notify());
                                        window.push_notification(
                                            (
                                                NotificationType::Success,
                                                format!(
                                                    "当前默认Java版本更改为: {}",
                                                    cx.global_mut::<AppSettings>().java_versions
                                                        [row]
                                                        .version
                                                ),
                                            ),
                                            cx,
                                        );
                                    }
                                }
                            })),
                    ),
            )
            .child(List::new(&state).flex_1().max_h(max_height))
    }
}
