use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, IconName, IndexPath, Sizable, StyledExt, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, EventEmitter, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, WeakEntity, Window, div, px,
};

use mclib::project::game_project::{GameProject, ModLoader, find_all_game_in_project_folder};

use crate::data::settings::AppSettings;

/// 版本管理页向主窗口发出的跳转请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionControlEvent {
    /// 打开设置里的「项目管理」页面。
    OpenProjectPathManager,
}

/// 路径下拉框里的一项：某个版本路径，或者最后那项「路径管理」。
///
/// 存的是路径在 `AppSettings::project_paths` 里的下标而不是路径本身，
/// 这样即使同一个路径被添加了两次，选中项也不会有歧义。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathOption {
    Project(usize),
    Manage,
}

#[derive(Debug, Clone)]
struct PathOptionItem {
    value: PathOption,
    label: SharedString,
}

impl SearchableListItem for PathOptionItem {
    type Value = PathOption;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 把设置里的路径列表转成下拉框选项，末尾补上「路径管理」。
fn path_option_items(paths: &[PathBuf]) -> Vec<PathOptionItem> {
    let mut items = paths
        .iter()
        .enumerate()
        .map(|(ix, path)| PathOptionItem {
            value: PathOption::Project(ix),
            label: SharedString::from(path.to_string_lossy().to_string()),
        })
        .collect::<Vec<_>>();

    items.push(PathOptionItem {
        value: PathOption::Manage,
        label: SharedString::from("路径管理…"),
    });

    items
}

fn loader_name(loader: &ModLoader) -> &'static str {
    match loader {
        ModLoader::Minecraft => "原版",
        ModLoader::Forge => "Forge",
        ModLoader::Neoforge => "NeoForge",
        ModLoader::Fabric => "Fabric",
    }
}

/// 项目名后面跟着版本标签，跟启动页下拉框的样式一致。
fn project_row(project: &GameProject) -> AnyElement {
    let mut tags = h_flex().flex_none().items_center().gap_1();
    tags = tags.child(Tag::secondary().child(project.game_version.clone()));
    // 原版的加载器版本就是游戏版本，再挂一个标签就重复了。
    if project.loader != ModLoader::Minecraft {
        tags = tags.child(Tag::info().child(format!(
            "{} {}",
            loader_name(&project.loader),
            project.loader_version
        )));
    }

    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(Label::new(project.name.clone())),
        )
        .child(tags)
        .into_any_element()
}

/// 扫一个目录下的所有项目，没选路径时返回空列表。
fn scan_projects(path: Option<&PathBuf>) -> Vec<GameProject> {
    path.map(find_all_game_in_project_folder)
        .unwrap_or_default()
}

/// 当前路径下扫出来的所有项目。
///
/// 这里是磁盘上的快照，点刷新按钮才会重新扫描。
struct ProjectListDelegate {
    projects: Vec<GameProject>,
    selected_index: Option<IndexPath>,
    /// 菜单里的动作需要回调页面，所以留一个页面的弱引用。
    page: WeakEntity<VersionControlPage>,
}

impl ListDelegate for ProjectListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.projects.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let project = self.projects.get(ix.row)?.clone();
        let page = self.page.clone();
        let menu_id = SharedString::from(format!("project-menu-{}", ix.row));

        Some(
            ListItem::new(ix)
                .child(project_row(&project))
                .selected(Some(ix) == self.selected_index)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.delegate_mut().set_selected_index(Some(ix), window, cx);
                }))
                // 每一项右边挂一个「⋯」按钮，点开是这个项目的操作菜单。
                .suffix(move |_, _cx| {
                    let project = project.clone();
                    let page = page.clone();
                    let menu_id = menu_id.clone();

                    Button::new(menu_id)
                        .icon(IconName::Ellipsis)
                        .ghost()
                        .small()
                        .dropdown_menu(move |menu, _, _| {
                            let reveal_path = project.path.clone();
                            let copy_text = project.path.to_string_lossy().to_string();
                            let page = page.clone();

                            menu.item(
                                PopupMenuItem::new("打开文件夹")
                                    .icon(IconName::FolderOpen)
                                    .on_click(move |_, _, cx| cx.reveal_path(&reveal_path)),
                            )
                            .item(
                                PopupMenuItem::new("复制路径")
                                    .icon(IconName::Copy)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_text.clone(),
                                        ));
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("刷新")
                                    .icon(IconName::RotateCw)
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            page.update(cx, |page, cx| page.refresh_projects(cx));
                                    }),
                            )
                        })
                }),
        )
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(Label::new("该路径下没有找到项目"))
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

pub struct VersionControlPage {
    /// 版本路径下拉框。
    select_state: Entity<SelectState<Vec<PathOptionItem>>>,
    /// 选中路径下的项目列表。
    list_state: Entity<ListState<ProjectListDelegate>>,
    /// 保活下拉框的事件订阅。
    _select_subscription: Subscription,
    /// `AppSettings::project_paths` 的快照，用来发现设置页里改过路径。
    path_options: Vec<PathBuf>,
    /// 当前选中的路径，默认取设置里的第一个。
    selected_path: Option<PathBuf>,
}

impl VersionControlPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paths: Vec<PathBuf> = cx.global::<AppSettings>().project_paths.clone();
        let selected_path = paths.first().cloned();

        // 有路径就默认选中第一个。
        let select_state = cx.new(|cx| {
            SelectState::new(
                path_option_items(&paths),
                selected_path.as_ref().map(|_| IndexPath::new(0)),
                window,
                cx,
            )
        });

        let _select_subscription = cx.subscribe_in(
            &select_state,
            window,
            |this: &mut Self, _state, event: &SelectEvent<Vec<PathOptionItem>>, window, cx| {
                let SelectEvent::Confirm(value) = event;

                match value {
                    Some(PathOption::Project(ix)) => {
                        let path = this.path_options.get(*ix).cloned();
                        if path != this.selected_path {
                            this.selected_path = path;
                            this.reload_projects(cx);
                            cx.notify();
                        }
                    }
                    // 「路径管理」不是真正的路径：把下拉框拨回当前路径，再请主窗口跳设置页。
                    Some(PathOption::Manage) => {
                        let index = this.path_index(this.selected_path.as_ref());
                        // 此刻还在 SelectState 自己的更新栈上，直接回头改它会重入更新，
                        // 因此推到本轮效果跑完后再拨回去。
                        cx.defer_in(window, move |this, window, cx| {
                            this.select_state.update(cx, |state, cx| {
                                state.set_selected_index(index, window, cx);
                            });
                        });
                        cx.emit(VersionControlEvent::OpenProjectPathManager);
                    }
                    None => {}
                }
            },
        );

        let page = cx.entity().downgrade();
        let list_state = cx.new(|cx| {
            ListState::new(
                ProjectListDelegate {
                    projects: scan_projects(selected_path.as_ref()),
                    selected_index: None,
                    page,
                },
                window,
                cx,
            )
        });

        Self {
            select_state,
            list_state,
            _select_subscription,
            path_options: paths,
            selected_path,
        }
    }

    /// 当前路径在 `path_options` 里的下标。
    fn path_index(&self, path: Option<&PathBuf>) -> Option<IndexPath> {
        path.and_then(|path| self.path_options.iter().position(|it| it == path))
            .map(IndexPath::new)
    }

    /// 重新扫描当前路径并刷新列表。
    fn reload_projects(&mut self, cx: &mut Context<Self>) {
        let projects = scan_projects(self.selected_path.as_ref());
        self.list_state.update(cx, |state, cx| {
            state.delegate_mut().projects = projects;
            state.delegate_mut().selected_index = None;
            cx.notify();
        });
    }

    /// 刷新按钮：重新扫描设置里的所有路径，再刷新当前路径的列表。
    ///
    /// 会连别的路径一起扫，是因为 `find_all_game_in_project_folder` 顺带把识别出来的
    /// 版本写回磁盘上的 `project.json`，跟启动时 `AppSettings::load` 做的事一致。
    fn refresh_projects(&mut self, cx: &mut Context<Self>) {
        for path in cx.global::<AppSettings>().project_paths.clone() {
            let _ = find_all_game_in_project_folder(&path);
        }

        self.reload_projects(cx);
        cx.notify();
    }

    /// 让下拉框跟上设置里的路径：设置页可能加过或删过路径。
    ///
    /// 放在渲染期做，是为了不额外维护一套「设置变了」的通知。
    fn sync_paths(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = cx.global::<AppSettings>().project_paths.clone();

        if paths == self.path_options {
            return;
        }

        self.path_options = paths;

        // 原来选中的路径没了（比如在设置里删掉了）就退回第一个。
        if !self
            .selected_path
            .as_ref()
            .is_some_and(|path| self.path_options.contains(path))
        {
            self.selected_path = self.path_options.first().cloned();
        }

        let items = path_option_items(&self.path_options);
        let index = self.path_index(self.selected_path.as_ref());
        self.select_state.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(index, window, cx);
        });

        self.reload_projects(cx);
    }
}

impl EventEmitter<VersionControlEvent> for VersionControlPage {}

impl Render for VersionControlPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_paths(window, cx);

        div()
            .v_flex()
            .size_full()
            .gap_3()
            // 底部留出 5 个单位（20px），列表不要贴着窗口底边。
            .pb_5()
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Select::new(&self.select_state)
                                .h(px(32.))
                                .placeholder("选择一个版本路径"),
                        ),
                    )
                    .child(
                        Button::new("refresh-projects")
                            .icon(IconName::RotateCw)
                            .label("刷新")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_projects(cx))),
                    ),
            )
            .child(
                List::new(&self.list_state)
                    .flex_1()
                    // 给列表加个边框，行内容稍微内缩一点，不贴着边框。
                    .p_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius),
            )
    }
}
