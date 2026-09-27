//! 安装目录选择框：设置里的版本目录列表 + 用户另选的目录 + 「浏览…」。
//!
//! 下载弹窗和整合包安装弹窗共用；选中的目录直接从 [`DirSelect::selected`]
//! 读取，使用方不需要关心下拉框的下标和「浏览…」这类伪选项。

use std::path::PathBuf;

use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::IndexPath;
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, Render, SharedString, Styled, Subscription,
    Window,
};
use rfd::AsyncFileDialog;

use crate::data::settings::AppSettings;

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

/// 安装目录选择框：渲染成整行宽度的下拉框，默认选中第一个版本目录。
pub(super) struct DirSelect {
    select: Entity<SelectState<Vec<DirOptionItem>>>,
    /// 当前选中的安装目录；「浏览…」只触发系统目录选择框，不会被记住。
    selected: Option<PathBuf>,
    /// 用户通过「浏览…」另选的目录。
    custom: Option<PathBuf>,
    _subscription: Subscription,
}

impl DirSelect {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paths = cx.global::<AppSettings>().project_paths.clone();
        // 有版本目录就默认选中第一个。
        let selected = paths.first().cloned();
        let select = cx.new(|cx| {
            SelectState::new(
                dir_option_items(&paths, None),
                (!paths.is_empty()).then_some(IndexPath::new(0)),
                window,
                cx,
            )
        });

        let subscription = cx.subscribe_in(
            &select,
            window,
            |this: &mut Self, _state, event: &SelectEvent<Vec<DirOptionItem>>, window, cx| {
                match event {
                    SelectEvent::Confirm(Some(DirOption::Project(ix))) => {
                        this.selected = cx
                            .global::<AppSettings>()
                            .project_paths
                            .get(*ix)
                            .cloned();
                    }
                    SelectEvent::Confirm(Some(DirOption::Custom(path))) => {
                        this.selected = Some(path.clone());
                    }
                    // 「浏览…」不是真正的目录：把下拉框拨回当前项，再弹系统目录选择框。
                    SelectEvent::Confirm(Some(DirOption::Browse)) => {
                        // 此刻还在 SelectState 自己的更新栈上，直接回头改它会重入更新，
                        // 因此推到本轮效果跑完后再拨回去。
                        cx.defer_in(window, move |this, window, cx| {
                            let index = this
                                .select
                                .read(cx)
                                .selected_index(cx)
                                .filter(|ix| ix.row < this.item_count(cx) - 1);
                            this.select.update(cx, |state, cx| {
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
                                        this.custom = Some(path.clone());
                                        this.selected = Some(path.clone());

                                        // 自定义目录总是倒数第二项（「浏览…」固定在最后）。
                                        let paths =
                                            cx.global::<AppSettings>().project_paths.clone();
                                        let items = dir_option_items(&paths, Some(&path));
                                        let custom_index = IndexPath::new(items.len() - 2);
                                        this.select.update(cx, |state, cx| {
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
                        this.selected = None;
                    }
                }
            },
        );

        Self {
            select,
            selected,
            custom: None,
            _subscription: subscription,
        }
    }

    /// 当前选中的安装目录；没有任何可选目录时返回 `None`。
    pub(super) fn selected(&self) -> Option<&PathBuf> {
        self.selected.as_ref()
    }

    /// 下拉框里有多少项。
    fn item_count(&self, cx: &App) -> usize {
        let paths = cx.global::<AppSettings>().project_paths.len();
        paths + if self.custom.is_some() { 1 } else { 0 } + 1
    }
}

impl Render for DirSelect {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Select::new(&self.select)
            .w_full()
            .placeholder("选择安装目录")
    }
}
