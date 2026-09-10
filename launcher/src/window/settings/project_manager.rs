use crate::data::settings::AppSettings;
use gpui_kit::base::{IndexPath, h_flex};
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::{StyledExt, WindowExt};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
};
use rfd::AsyncFileDialog;

struct ProjectListDelegate {
    selected_index: Option<IndexPath>,
}

impl ListDelegate for ProjectListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, cx: &App) -> usize {
        cx.global::<AppSettings>().project_paths.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let app_setting = cx.global::<AppSettings>();
        app_setting.project_paths.get(ix.row).map(|item| {
            ListItem::new(ix)
                .child(div().v_flex().child(Label::new(item.to_string_lossy())))
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
pub struct ProjectManager {
    state: Option<Entity<ListState<ProjectListDelegate>>>,
}

impl ProjectManager {}

impl Render for ProjectManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self
            .state
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    ListState::new(
                        ProjectListDelegate {
                            selected_index: None,
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
                    .child(Label::new("当前添加的项目文件夹"))
                    .child(
                        ButtonGroup::new("btn-group")
                            .child(Button::new("btn1").label("添加").on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    let state = state.clone();
                                    window
                                        .spawn(cx, async move |cx| {
                                            let file = AsyncFileDialog::new()
                                                .set_title("选择一个文件夹")
                                                .pick_folder()
                                                .await;

                                            let Some(file) = file else {
                                                return;
                                            };
                                            let path = file.path().to_path_buf();

                                            let _ = cx.update(|window, cx| {
                                                let app_settings = cx.global_mut::<AppSettings>();
                                                if !app_settings
                                                    .project_paths
                                                    .iter()
                                                    .any(|j| *j == path)
                                                {
                                                    app_settings.project_paths.push(path.clone());
                                                }
                                                let _ = app_settings.save();
                                                state.update(cx, |_, cx| cx.notify());
                                                window.push_notification(
                                                    (
                                                        NotificationType::Success,
                                                        format!(
                                                            "成功添加版本隔离文件夹: {:?}",
                                                            path
                                                        ),
                                                    ),
                                                    cx,
                                                );
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
                                        let remove_path = app_settings.project_paths.remove(row);
                                        let _ = cx.global::<AppSettings>().save();
                                        state.update(cx, |_, cx| cx.notify());
                                        window.push_notification(
                                            (
                                                NotificationType::Success,
                                                format!("成功删除项目文件夹: {:?}", remove_path),
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
