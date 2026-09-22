use std::path::{Path, PathBuf};

use gpui_kit::base::{Disableable, StyledExt, h_flex, v_flex};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{ActiveTheme, IconName, IndexPath};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Div, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, WeakEntity, Window, div, px, rgb,
};

use mclib::project::game_project::{GameProject, ModLoader};

use crate::jj;
use crate::window::project_select::{ProjectList, ProjectSelect, loader_name};

/// 开发页侧边栏里可以被选中的项。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DevelopSection {
    General,
    Mods,
    ResourcePacks,
    Shaders,
}

impl DevelopSection {
    fn label(self) -> &'static str {
        match self {
            Self::General => "通用",
            Self::Mods => "模组管理",
            Self::ResourcePacks => "资源包管理",
            Self::Shaders => "光影管理",
        }
    }
}

/// mods 目录里的一个模组文件。
#[derive(Debug, Clone)]
struct ModEntry {
    /// 磁盘上的完整路径，切换启用状态时直接对它重命名。
    path: PathBuf,
    /// 文件名，例如 `sodium.jar` 或 `sodium.jar.disabled`。
    file_name: String,
    /// `true` 表示 `.jar`（启用），`false` 表示 `.jar.disabled`（禁用）。
    enabled: bool,
}

/// 文件名是 `.jar`（启用）或 `.jar.disabled`（禁用）就返回对应状态，其它文件不算模组。
fn mod_enabled(file_name: &str) -> Option<bool> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".jar.disabled") {
        Some(false)
    } else if lower.ends_with(".jar") {
        Some(true)
    } else {
        None
    }
}

/// 把文件名换成另一个状态的写法：`.jar` ↔ `.jar.disabled`。
///
/// 已经是目标状态或不是模组文件时返回 `None`，不用重命名。
fn toggled_mod_name(file_name: &str, enabled: bool) -> Option<String> {
    match (enabled, mod_enabled(file_name)?) {
        (true, false) => {
            // `.disabled` 是纯 ASCII，按字节截断不会切在多字节字符中间。
            Some(file_name[..file_name.len() - ".disabled".len()].to_owned())
        }
        (false, true) => Some(format!("{file_name}.disabled")),
        _ => None,
    }
}

/// 扫一个 mods 目录，只收 `.jar` / `.jar.disabled` 文件，按文件名排序。
fn scan_mods(dir: &Path) -> Vec<ModEntry> {
    let mut mods = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return mods;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(enabled) = mod_enabled(&file_name) else {
            continue;
        };

        mods.push(ModEntry {
            path,
            file_name,
            enabled,
        });
    }

    mods.sort_by_key(|entry| entry.file_name.to_ascii_lowercase());
    mods
}

/// 模组列表：磁盘快照存在这里，点刷新或换项目时才重新扫描。
struct ModListDelegate {
    mods: Vec<ModEntry>,
    /// 搜索框当前的内容，过滤出要显示的行。
    query: String,
    selected_index: Option<IndexPath>,
    /// 勾选框的回调需要回到页面，所以留一个页面的弱引用。
    page: WeakEntity<DevelopPage>,
}

impl ModListDelegate {
    /// 按搜索词过滤后的模组，文件名不区分大小写做子串匹配。
    fn visible_mods(&self) -> Vec<&ModEntry> {
        let query = self.query.trim().to_ascii_lowercase();
        self.mods
            .iter()
            .filter(|entry| {
                query.is_empty() || entry.file_name.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }
}

impl ListDelegate for ModListDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_owned();
        self.selected_index = None;
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.visible_mods().len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let visible = self.visible_mods();
        let entry = visible.get(ix.row)?;
        let file_name = entry.file_name.clone();
        let path = entry.path.clone();
        let enabled = entry.enabled;
        let selected = Some(ix) == self.selected_index;
        let page = self.page.clone();

        // 点行只改变选中态；勾选状态交给前面的勾选框处理。
        Some(
            ListItem::new(ix)
                .selected(selected)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.delegate_mut().set_selected_index(Some(ix), window, cx);
                }))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(
                            Checkbox::new(SharedString::from(format!("mod-enabled-{}", ix.row)))
                                .checked(enabled)
                                .tooltip(if enabled {
                                    "点击禁用"
                                } else {
                                    "点击启用"
                                })
                                .on_click(move |checked, window, cx| {
                                    let _ = page.update(cx, |page, cx| {
                                        page.set_mod_enabled(path.clone(), *checked, window, cx);
                                    });
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(Label::new(file_name)),
                        ),
                ),
        )
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let text = if self.mods.is_empty() {
            "mods 目录里没有模组"
        } else {
            "没有匹配搜索的模组"
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(Label::new(text).text_color(rgb(0x9ca3af)))
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

pub struct DevelopPage {
    /// 当前选中的侧边栏项，每个菜单项是否高亮由它决定。
    section: DevelopSection,
    /// 项目下拉框，和启动页、VCS 页共用一份实现；选中项写回设置，几个页面自然同步。
    project_select: ProjectSelect,
    /// 保活项目下拉框的事件订阅。
    _select_subscription: Subscription,
    /// 模组列表，条目是渲染期扫出来的磁盘快照。
    mod_list: Entity<ListState<ModListDelegate>>,
    /// 当前列表对应的 mods 目录，用来发现项目切换后需要重新扫描。
    mods_dir: Option<PathBuf>,
    git_init_in_progress: bool,
}

impl DevelopPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project_select = ProjectSelect::new(window, cx);
        let select_subscription = cx.subscribe_in(
            project_select.state(),
            window,
            |page: &mut Self, _state, event: &SelectEvent<ProjectList>, _window, cx| {
                let SelectEvent::Confirm(Some(path)) = event else {
                    return;
                };

                // 写回设置由下拉框自己负责，启动页/VCS 页下次渲染就会跟上。
                if !page.project_select.confirm(path.clone(), cx) {
                    return;
                }

                cx.notify();
            },
        );

        let page = cx.entity().downgrade();
        let mod_list = cx.new(|cx| {
            ListState::new(
                ModListDelegate {
                    mods: Vec::new(),
                    query: String::new(),
                    selected_index: None,
                    page,
                },
                window,
                cx,
            )
            // 列表自带搜索框，输入时回调 delegate 的 perform_search。
            .searchable(true)
        });

        Self {
            section: DevelopSection::General,
            project_select,
            _select_subscription: select_subscription,
            mod_list,
            mods_dir: None,
            git_init_in_progress: false,
        }
    }

    /// 选中某一项。
    ///
    /// 必须 `notify`，否则 gpui 不会重新渲染，`active` 高亮也不会跟着变。
    fn select(&mut self, section: DevelopSection, cx: &mut Context<Self>) {
        if self.section != section {
            self.section = section;
            cx.notify();
        }
    }

    /// 生成一个菜单项：自己是不是当前选中项决定是否高亮，点击后把自己设为选中项。
    fn menu_item(&self, section: DevelopSection, cx: &mut Context<Self>) -> SidebarMenuItem {
        SidebarMenuItem::new(section.label())
            .active(self.section == section)
            .on_click(cx.listener(move |this, _, _, cx| this.select(section, cx)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Sidebar<SidebarGroup<SidebarMenu>> {
        Sidebar::new("develop_sidebar")
            .child(
                SidebarGroup::new("版本设置").child(
                    SidebarMenu::new()
                        .child(self.menu_item(DevelopSection::General, cx))
                ),
            )
            .child(
                SidebarGroup::new("资源管理").child(
                    SidebarMenu::new()
                        .child(self.menu_item(DevelopSection::Mods, cx))
                        .child(self.menu_item(DevelopSection::ResourcePacks, cx))
                        .child(self.menu_item(DevelopSection::Shaders, cx)),
                ),
            )
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let child = match self.section {
            DevelopSection::General => self.render_general(window, cx).into_any_element(),
            DevelopSection::Mods => self.render_mod_list(window, cx).into_any_element(),
            section => card_outline().child(section.label()).into_any_element(),
        };
        v_flex().size_full().child(child)
    }

    fn render_general(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 第一张卡片是当前项目的基本信息，跟着顶部的下拉框走。
        let info_card = project_info_card(self.project_select.selected_project());

        // 第二张卡片是仓库和项目资源转实例化。
        let git_card = card_outline().gap_2().child(Label::new("版本控制"));
        let Some(project) = self.project_select.selected_project() else {
            return v_flex()
                .size_full()
                .overflow_y_scrollbar()
                .child(git_card.child(Label::new("当前没有选中的整合包").text_color(rgb(0x9ca3af))));
        };

        let vcs_row = if let Some(info) = jj::git_backend_info(&project.path) {
            DescriptionList::new()
                .bordered(false)
                .columns(1)
                .label_width(px(72.))
                .item(
                    "当前分支",
                    info.branch.unwrap_or_else(|| "detached HEAD".to_owned()),
                    1,
                )
        } else {
            let path = project.path.clone();
            let button = Button::new("init-git-backend")
                .label(if self.git_init_in_progress {
                    "初始化中..."
                } else {
                    "初始化 Git 后端"
                })
                .disabled(self.git_init_in_progress)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.init_git_backend(path.clone(), window, cx);
                }));

            DescriptionList::new()
                .bordered(false)
                .columns(1)
                .label_width(px(72.))
                .item("仓库", button.into_any_element(), 1)
        };

        v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .gap_3()
            .child(info_card)
            .child(git_card.child(vcs_row))
    }

    /// 当前项目的 mods 目录，没选项目时为 `None`。
    fn current_mods_dir(&self) -> Option<PathBuf> {
        self.project_select
            .selected_project()
            .map(|project| project.path.join("mods"))
    }

    /// 选中项目变了就重新扫描；没变化什么都不做。
    ///
    /// 放在渲染期做，和 `project_select.sync` 一个思路，不用额外维护「项目变了」的通知。
    fn sync_mods(&mut self, cx: &mut Context<Self>) {
        let mods_dir = self.current_mods_dir();
        if mods_dir != self.mods_dir {
            self.reload_mods(cx);
        }
    }

    /// 重新扫描 `self.mods_dir` 并刷新列表。
    fn reload_mods(&mut self, cx: &mut Context<Self>) {
        self.mods_dir = self.current_mods_dir();
        let mods = self
            .mods_dir
            .as_ref()
            .map(|dir| scan_mods(dir))
            .unwrap_or_default();

        self.mod_list.update(cx, |state, cx| {
            state.delegate_mut().mods = mods;
            state.delegate_mut().selected_index = None;
            cx.notify();
        });
    }

    /// 刷新按钮：重新扫描当前项目的 mods 目录。
    fn refresh_mods(&mut self, cx: &mut Context<Self>) {
        self.reload_mods(cx);
        cx.notify();
    }

    /// 勾选框切换：把文件在 `.jar` 和 `.jar.disabled` 之间重命名。
    fn set_mod_enabled(
        &mut self,
        path: PathBuf,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_path = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| toggled_mod_name(name, enabled))
            .map(|name| path.with_file_name(name));

        if let Some(new_path) = new_path
            && let Err(error) = std::fs::rename(&path, &new_path)
        {
            window.push_notification(
                (
                    gpui_kit::component::notification::NotificationType::Error,
                    format!("切换模组状态失败: {error}"),
                ),
                cx,
            );
        }

        // 不管成败都重新扫一遍，让列表和磁盘保持一致。
        self.reload_mods(cx);
        cx.notify();
    }

    fn render_mod_list(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 顶部下拉框换项目后，这里负责把列表切到新项目的 mods 目录。
        self.sync_mods(cx);

        let Some(mods_dir) = self.mods_dir.clone() else {
            return v_flex().size_full().child(
                card_outline()
                    .child(Label::new("模组管理"))
                    .child(Label::new("当前没有选中的整合包").text_color(rgb(0x9ca3af))),
            );
        };

        let toolbar = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                Button::new("refresh-mods")
                    .icon(IconName::RotateCw)
                    .label("刷新")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_mods(cx))),
            )
            .child(
                Button::new("open-mods-folder")
                    .icon(IconName::FolderOpen)
                    .label("打开文件夹")
                    .on_click(move |_, _, cx| {
                        // 目录还不存在时先建出来，免得系统资源管理器报错。
                        let _ = std::fs::create_dir_all(&mods_dir);
                        cx.reveal_path(&mods_dir);
                    }),
            );

        v_flex()
            .size_full()
            .gap_2()
            .child(toolbar)
            .child(
                List::new(&self.mod_list)
                    .flex_1()
                    .search_placeholder("搜索模组")
                    // 给列表加个边框，行内容稍微内缩一点，不贴着边框。
                    .p_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius),
            )
    }

    fn init_git_backend(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_init_in_progress {
            return;
        }

        self.git_init_in_progress = true;
        cx.notify();

        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { jj::init_git_backend(path) })
                .await;

            let _ = page.update_in(cx, |page, window, cx| {
                page.git_init_in_progress = false;
                match result {
                    Ok(()) => window.push_notification(
                        (
                            gpui_kit::component::notification::NotificationType::Success,
                            "已初始化 Git 后端",
                        ),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        (
                            gpui_kit::component::notification::NotificationType::Error,
                            error.to_string(),
                        ),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for DevelopPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 让下拉框跟上设置：设置页可能加过/删过版本目录，启动页/VCS 页也可能换过当前项目。
        self.project_select.sync(window, cx);

        // 和启动页同一份状态，但用组件默认的常规尺寸，不再放大。
        let project_select =
            Select::new(self.project_select.state()).placeholder("选择要开发的整合包");
        let sidebar = self.render_sidebar(cx);
        let content = self.render_content(window, cx);

        div()
            .v_flex()
            .size_full()
            .gap_3()
            .child(div().w_full().child(project_select))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .gap_2()
                    .child(sidebar)
                    .child(content),
            )
    }
}

fn card_outline() -> Div {
    div()
        .flex_col()
        .border_1()
        .border_color(rgb(0xe5e7eb))
        .rounded_lg()
        .p_4()
}

/// 当前项目的基本信息卡片。
///
/// 项目来自顶部的下拉框，没有选中项目时只显示一句提示。
fn project_info_card(project: Option<&GameProject>) -> Div {
    let card = card_outline().gap_2().child(Label::new("基本信息"));

    let Some(project) = project else {
        return card.child(Label::new("当前没有选中的整合包").text_color(rgb(0x9ca3af)));
    };

    card.child(
        DescriptionList::new()
            // 外面已经有卡片边框了，列表自己不再画一层。
            .bordered(false)
            .columns(1)
            .label_width(px(72.))
            .item("项目名称", project.name.clone(), 1)
            .item("整合包版本", text_or_dash(&project.version), 1)
            .item("游戏版本", project.game_version.clone(), 1)
            .item("加载器", loader_text(project), 1)
            .item("路径", path_element(project), 1),
    )
}

/// 空值统一显示成一个短横，卡片里就不会出现空白。
fn text_or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "—".to_owned()
    } else {
        text.to_owned()
    }
}

/// 加载器：原版只有一个「原版」，装过加载器的显示「Forge 21.0.167」。
fn loader_text(project: &GameProject) -> String {
    let name = loader_name(&project.loader);
    if project.loader == ModLoader::Minecraft {
        name.to_owned()
    } else {
        format!("{} {}", name, project.loader_version)
    }
}

/// 项目路径可能很长，单行截断显示。
fn path_element(project: &GameProject) -> AnyElement {
    div()
        .min_w_0()
        .truncate()
        .child(project.path.display().to_string())
        .into_any_element()
}
