use std::path::PathBuf;

use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, IconName, IndexPath, Sizable, StyledExt, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, px,
};

use mclib::project::game_project::{GameProject, ModLoader, find_all_game_in_project_folder};

use crate::data::settings::AppSettings;

/// 扫一遍设置里的所有版本目录，拿到全部项目。
fn detect_projects(paths: &[PathBuf]) -> Vec<GameProject> {
    let mut projects = paths
        .iter()
        .flat_map(find_all_game_in_project_folder)
        .collect::<Vec<_>>();

    // 同一个目录被重复添加时会出现重复项，按路径去重；顺便按名字排序，选项顺序才稳定。
    projects.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    projects.dedup_by(|a, b| a.path == b.path);

    projects
}

fn has_project(projects: &[GameProject], path: &PathBuf) -> bool {
    projects.iter().any(|project| &project.path == path)
}

fn loader_name(loader: &ModLoader) -> &'static str {
    match loader {
        ModLoader::Minecraft => "原版",
        ModLoader::Forge => "Forge",
        ModLoader::Neoforge => "NeoForge",
        ModLoader::Fabric => "Fabric",
    }
}

/// 下拉框里的一项，值就是项目的路径。
#[derive(Debug, Clone)]
struct ProjectItem {
    value: PathBuf,
    /// 项目名。触发器和下拉列表里都用它。
    label: SharedString,
    /// 游戏版本的标签。
    game_version: SharedString,
    /// 加载器的标签，原版没有额外的加载器信息，所以是 None。
    loader: Option<SharedString>,
}

impl ProjectItem {
    /// 下拉列表里的一行：项目名 + 用标签展示的版本信息。
    ///
    /// `checked` 是当前选中的那一项，行尾会画一个对勾。
    fn row(&self, checked: bool) -> AnyElement {
        let mut tags = h_flex().flex_none().items_center().gap_1();
        tags = tags.child(Tag::secondary().child(self.game_version.clone()));
        // 原版的加载器版本就是游戏版本，再挂一个标签就重复了。
        if let Some(loader) = self.loader.clone() {
            tags = tags.child(Tag::info().child(loader));
        }

        // 没选中时也占住同样大小，选项不会因为对勾出现/消失而抖动。
        let check = if checked {
            Icon::new(IconName::Check).xsmall().into_any_element()
        } else {
            div().w(px(16.)).h(px(16.)).into_any_element()
        };

        h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            // 项目名和标签挨在一起：左边名字，标签紧跟在名字后面，和名字留一点距离。
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .truncate()
                            .child(Label::new(self.label.clone())),
                    )
                    .child(tags),
            )
            .child(check)
            .into_any_element()
    }
}

impl SearchableListItem for ProjectItem {
    type Value = PathBuf;

    /// 选中的那一项只显示名字，版本信息只在下拉列表里用标签展示。
    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 下拉框的选项：所有目录里检测到的项目。
///
/// 自己实现 delegate（而不是直接用 `Vec<ProjectItem>`），是为了能拿到 `checked`，
/// 在自定义的行布局里把「当前选中」的对勾画出来。
#[derive(Debug, Clone)]
struct ProjectList {
    items: Vec<ProjectItem>,
}

impl SearchableListDelegate for ProjectList {
    type Item = ProjectItem;

    fn items_count(&self, _section: usize) -> usize {
        self.items.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.items.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.items
            .iter()
            .position(|item| item.value() == value)
            .map(IndexPath::new)
    }

    fn render_item(
        &self,
        _ix: IndexPath,
        item: &Self::Item,
        checked: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<AnyElement> {
        Some(item.row(checked))
    }
}

fn project_items(projects: &[GameProject]) -> ProjectList {
    ProjectList {
        items: projects
            .iter()
            .map(|project| ProjectItem {
                value: project.path.clone(),
                label: SharedString::from(project.name.clone()),
                game_version: SharedString::from(project.game_version.clone()),
                loader: match project.loader {
                    ModLoader::Minecraft => None,
                    _ => Some(SharedString::from(format!(
                        "{} {}",
                        loader_name(&project.loader),
                        project.loader_version
                    ))),
                },
            })
            .collect(),
    }
}

pub struct StartPage {
    select_state: Entity<SelectState<ProjectList>>,
    /// 保活下拉框的事件订阅。
    _select_subscription: Subscription,
    /// 当前检测到的所有项目，就是下拉框的选项。
    projects: Vec<GameProject>,
    /// 当前选中的项目，跟 `AppSettings::select_project_path` 保持一致。
    selected_project: Option<PathBuf>,
    /// 上次同步时设置里的版本目录，用来发现设置页的增删。
    known_paths: Vec<PathBuf>,
    /// 上次同步时设置里的当前项目。
    known_selection: Option<PathBuf>,
}

impl StartPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paths = cx.global::<AppSettings>().project_paths.clone();
        let projects = detect_projects(&paths);

        // 优先用设置里记着的当前项目，它不在了就退回第一个。
        let selected = cx
            .global::<AppSettings>()
            .select_project_path
            .clone()
            .filter(|path| has_project(&projects, path))
            .or_else(|| projects.first().map(|project| project.path.clone()));

        let index = selected
            .as_ref()
            .and_then(|path| projects.iter().position(|project| &project.path == path))
            .map(IndexPath::new);

        let select_state =
            cx.new(|cx| SelectState::new(project_items(&projects), index, window, cx));

        let _select_subscription = cx.subscribe_in(
            &select_state,
            window,
            |this: &mut Self, _state, event: &SelectEvent<ProjectList>, _window, cx| {
                let SelectEvent::Confirm(Some(path)) = event else {
                    return;
                };

                if this.selected_project.as_ref() == Some(path) {
                    return;
                }

                this.selected_project = Some(path.clone());
                this.known_selection = Some(path.clone());

                {
                    let settings = cx.global_mut::<AppSettings>();
                    settings.select_project_path = Some(path.clone());
                    let _ = settings.save();
                }

                cx.notify();
            },
        );

        // 把解析出来的当前项目写回设置，让「当前项目」只有一个来源。
        if cx.global::<AppSettings>().select_project_path != selected {
            cx.global_mut::<AppSettings>().select_project_path = selected.clone();
        }

        Self {
            select_state,
            _select_subscription,
            projects,
            selected_project: selected.clone(),
            known_paths: paths,
            known_selection: selected,
        }
    }

    /// 让下拉框跟上设置：设置页可能加过/删过版本目录，别的页面也可能改过当前项目。
    ///
    /// 放在渲染期做，是为了不额外维护一套「设置变了」的通知。
    fn sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.global::<AppSettings>().project_paths.clone();
        let settings_selection = cx.global::<AppSettings>().select_project_path.clone();

        let paths_changed = paths != self.known_paths;
        if !paths_changed && settings_selection == self.known_selection {
            return;
        }

        if paths_changed {
            self.known_paths = paths.clone();
            self.projects = detect_projects(&paths);
        }

        // 设置里指定的项目优先，它没了就沿用原来的选择，再不行取第一个。
        let selected = settings_selection
            .filter(|path| has_project(&self.projects, path))
            .or_else(|| {
                self.selected_project
                    .clone()
                    .filter(|path| has_project(&self.projects, path))
            })
            .or_else(|| self.projects.first().map(|project| project.path.clone()));

        self.selected_project = selected.clone();
        self.known_selection = selected.clone();

        if cx.global::<AppSettings>().select_project_path != selected {
            cx.global_mut::<AppSettings>().select_project_path = selected.clone();
        }

        let items = project_items(&self.projects);
        let index = selected
            .and_then(|path| {
                self.projects
                    .iter()
                    .position(|project| project.path == path)
            })
            .map(IndexPath::new);

        self.select_state.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(index, window, cx);
        });
    }
}

impl Render for StartPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_settings(window, cx);

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
                            .label("启动游戏")
                            .large()
                            .h(px(60.))
                            .px(px(40.))
                            .text_3xl()
                            .on_click(|_, _, _| {
                                println!("launch button clicked");
                            }),
                    ),
                )
                .child(
                    div().w_full().child(
                        Select::new(&self.select_state)
                            .disabled(false)
                            .large()
                            .h(px(48.))
                            .placeholder("选择要启动的项目"),
                    ),
                ),
        )
    }
}
