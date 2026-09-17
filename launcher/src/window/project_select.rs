//! 项目下拉框：启动页和 VCS 页共用的一份实现。
//!
//! 「当前项目」只有一个来源：`AppSettings::select_project_path`。两个页面都在渲染时读它、
//! 也都把用户的选择写回去，所以在任何一个页面里换了项目，另一个页面下次渲染就会跟上，

use std::path::PathBuf;

use gpui_kit::component::label::Label;
use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_kit::component::select::SelectState;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, IconName, IndexPath, Sizable, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, Entity, IntoElement, ParentElement, SharedString, Styled, Window,
    div, px,
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

/// 项目是不是在这份清单里。
fn has_project(projects: &[GameProject], path: &PathBuf) -> bool {
    projects.iter().any(|project| &project.path == path)
}

/// 决定该选中哪个项目：设置里记着的优先，它不在了就沿用原来的选择，再不行取第一个。
fn resolve_selection(
    projects: &[GameProject],
    settings_selection: Option<PathBuf>,
    previous: Option<PathBuf>,
) -> Option<PathBuf> {
    settings_selection
        .filter(|path| has_project(projects, path))
        .or_else(|| previous.filter(|path| has_project(projects, path)))
        .or_else(|| projects.first().map(|project| project.path.clone()))
}

/// 项目在清单里的下标，也就是下拉框的选中项。
fn index_of(projects: &[GameProject], path: Option<&PathBuf>) -> Option<IndexPath> {
    path.and_then(|path| projects.iter().position(|project| &project.path == path))
        .map(IndexPath::new)
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
pub struct ProjectItem {
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
pub struct ProjectList {
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

/// 项目下拉框的状态，启动页和 VCS 页各持有一份。
///
/// 页面在渲染时调 [`ProjectSelect::sync`] 跟设置对齐，用户确认时调
/// [`ProjectSelect::confirm`] 写回设置，两边用的是同一套「扫描目录 + 回退 + 落盘」的逻辑。
pub struct ProjectSelect {
    state: Entity<SelectState<ProjectList>>,
    /// 当前检测到的所有项目，就是下拉框的选项。
    projects: Vec<GameProject>,
    /// 当前选中的项目路径，跟 `AppSettings::select_project_path` 保持一致。
    selected_path: Option<PathBuf>,
    /// 上次同步时设置里的版本目录，用来发现设置页的增删。
    known_paths: Vec<PathBuf>,
    /// 上次同步时设置里的当前项目。
    known_selection: Option<PathBuf>,
}

impl ProjectSelect {
    /// 建下拉框：先按设置里记着的当前项目定位，定不下来就退回第一个。
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let paths = cx.global::<AppSettings>().project_paths.clone();
        let projects = detect_projects(&paths);
        let selected = resolve_selection(
            &projects,
            cx.global::<AppSettings>().select_project_path.clone(),
            None,
        );
        let index = index_of(&projects, selected.as_ref());
        let state = cx.new(|cx| SelectState::new(project_items(&projects), index, window, cx));

        // 把解析出来的当前项目写回设置，让「当前项目」只有一个来源。
        if cx.global::<AppSettings>().select_project_path != selected {
            cx.global_mut::<AppSettings>().select_project_path = selected.clone();
        }

        Self {
            state,
            projects,
            selected_path: selected.clone(),
            known_paths: paths,
            known_selection: selected,
        }
    }

    /// 让下拉框跟上设置：设置页可能加过/删过版本目录，别的页面也可能换过当前项目。
    ///
    /// 放在渲染期做，是为了不额外维护一套「设置变了」的通知。
    ///
    /// 返回选中的项目有没有变，页面可以据此决定要不要重新加载（比如 VCS 的历史）。
    pub fn sync(&mut self, window: &mut Window, cx: &mut App) -> bool {
        let paths = cx.global::<AppSettings>().project_paths.clone();
        let settings_selection = cx.global::<AppSettings>().select_project_path.clone();

        if paths == self.known_paths && settings_selection == self.known_selection {
            return false;
        }

        if paths != self.known_paths {
            self.known_paths = paths.clone();
            self.projects = detect_projects(&paths);
        }

        // 设置里指定的项目优先，它没了就沿用原来的选择，再不行取第一个。
        let selected = resolve_selection(
            &self.projects,
            settings_selection,
            self.selected_path.clone(),
        );
        let changed = selected != self.selected_path;
        self.selected_path = selected.clone();
        self.known_selection = selected.clone();

        if cx.global::<AppSettings>().select_project_path != selected {
            cx.global_mut::<AppSettings>().select_project_path = selected.clone();
        }

        let items = project_items(&self.projects);
        let index = index_of(&self.projects, selected.as_ref());
        self.state.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(index, window, cx);
        });

        changed
    }

    /// 下拉框确认某项后：更新选中状态、写回设置并落盘。
    ///
    /// 返回选中的项目有没有变，页面可以据此决定要不要重新加载（比如 VCS 的历史）。
    pub fn confirm(&mut self, path: PathBuf, cx: &mut App) -> bool {
        if self.selected_path.as_ref() == Some(&path) {
            return false;
        }

        self.selected_path = Some(path.clone());
        self.known_selection = Some(path.clone());

        let settings = cx.global_mut::<AppSettings>();
        settings.select_project_path = Some(path);
        let _ = settings.save();

        true
    }

    /// 下拉框的状态，页面的 `Select` 元素和事件订阅都用它。
    pub fn state(&self) -> &Entity<SelectState<ProjectList>> {
        &self.state
    }

    /// 当前选中的项目路径。
    pub fn selected_path(&self) -> Option<&PathBuf> {
        self.selected_path.as_ref()
    }

    /// 当前选中的项目，在检测到的项目里查。
    pub fn selected_project(&self) -> Option<&GameProject> {
        let path = self.selected_path.as_ref()?;
        self.projects.iter().find(|project| &project.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, path: &str) -> GameProject {
        GameProject {
            name: name.to_owned(),
            version: String::new(),
            path: PathBuf::from(path),
            game_version: "1.21".to_owned(),
            loader: ModLoader::Minecraft,
            loader_version: "1.21".to_owned(),
        }
    }

    #[test]
    fn resolve_selection_prefers_settings_then_previous_then_first() {
        let projects = vec![project("a", "/a"), project("b", "/b")];

        // 设置里记着的项目还在，就用它。
        assert_eq!(
            resolve_selection(
                &projects,
                Some(PathBuf::from("/b")),
                Some(PathBuf::from("/a"))
            ),
            Some(PathBuf::from("/b"))
        );
        // 设置里的项目没了（比如被设置页删掉了），沿用原来的选择。
        assert_eq!(
            resolve_selection(
                &projects,
                Some(PathBuf::from("/gone")),
                Some(PathBuf::from("/b"))
            ),
            Some(PathBuf::from("/b"))
        );
        // 都没了就退回第一个。
        assert_eq!(
            resolve_selection(
                &projects,
                Some(PathBuf::from("/gone")),
                Some(PathBuf::from("/gone-too"))
            ),
            Some(PathBuf::from("/a"))
        );
        // 一个项目都没有。
        assert_eq!(resolve_selection(&[], None, None), None);
    }

    #[test]
    fn index_of_matches_the_project_path() {
        let projects = vec![project("a", "/a"), project("b", "/b")];

        assert_eq!(index_of(&projects, Some(&PathBuf::from("/b"))), Some(IndexPath::new(1)));
        assert_eq!(index_of(&projects, Some(&PathBuf::from("/gone"))), None);
        assert_eq!(index_of(&projects, None), None);
    }

    #[test]
    fn project_items_show_version_tags_and_skip_the_vanilla_loader() {
        let mut forge = project("forge", "/forge");
        forge.loader = ModLoader::Forge;
        forge.loader_version = "21.0".to_owned();
        let list = project_items(&[project("vanilla", "/vanilla"), forge]);

        let vanilla = list.item(IndexPath::new(0)).expect("vanilla item");
        assert_eq!(vanilla.title().to_string(), "vanilla");
        assert_eq!(vanilla.game_version.to_string(), "1.21");
        assert_eq!(vanilla.loader, None);

        let forge = list.item(IndexPath::new(1)).expect("forge item");
        assert_eq!(
            forge.loader.as_ref().map(|loader| loader.to_string()),
            Some("Forge 21.0".to_owned())
        );
        // 值是项目的路径，选中项才能跟设置里的路径对上。
        assert_eq!(forge.value(), &PathBuf::from("/forge"));
    }
}
