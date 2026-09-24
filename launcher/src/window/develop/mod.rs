pub(super) mod resource_dialog;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::base::{Disableable, StyledExt, h_flex, v_flex};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{ActiveTheme, IconName, IndexPath};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Div, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, WeakEntity, Window, div, px, rgb,
};

use mclib::detective::base::{
    ResourceKind, prune_files_without_record, prune_records_without_file,
};
use mclib::project::game_project::{GameProject, ModLoader};

use resource_dialog::ResourceSyncDialog;

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

/// 项目资源目录里的一个可启停文件（模组、资源包、光影通用）。
#[derive(Debug, Clone)]
struct PackEntry {
    /// 磁盘上的完整路径，切换启用状态时直接对它重命名。
    path: PathBuf,
    /// 文件名，例如 `sodium.jar` 或 `sodium.jar.disabled`。
    file_name: String,
    /// `true` 表示启用（`.jar` / `.zip`），`false` 表示禁用（末尾追加 `.disabled`）。
    enabled: bool,
}

/// 可以被勾选启停的资源类别，和项目下的目录一一对应。
///
/// 推送弹窗也要用这一套（同一个资源类别、同一套说法），所以对 `window` 模块可见。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackKind {
    /// `mods` 目录，`.jar` 文件。
    Mod,
    /// `resourcepacks` 目录，`.zip` 文件。
    ResourcePack,
    /// `shaderpacks` 目录，`.zip` 文件。
    Shader,
}

impl PackKind {
    /// 侧边栏项对应的资源类别，「通用」页没有。
    pub(crate) fn from_section(section: DevelopSection) -> Option<Self> {
        match section {
            DevelopSection::Mods => Some(Self::Mod),
            DevelopSection::ResourcePacks => Some(Self::ResourcePack),
            DevelopSection::Shaders => Some(Self::Shader),
            DevelopSection::General => None,
        }
    }

    /// 在项目目录下的文件夹名。
    pub(crate) fn dir_name(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::ResourcePack => "resourcepacks",
            Self::Shader => "shaderpacks",
        }
    }

    /// 启用文件的后缀，禁用态是在末尾再追加 `.disabled`。
    fn ext(self) -> &'static str {
        match self {
            Self::Mod => ".jar",
            Self::ResourcePack | Self::Shader => ".zip",
        }
    }

    /// 界面文案里的叫法。
    pub(crate) fn noun(self) -> &'static str {
        match self {
            Self::Mod => "模组",
            Self::ResourcePack => "资源包",
            Self::Shader => "光影",
        }
    }

    /// 序列化/反序列化用的资源类别，目录和 ext 跟这里保持一致。
    pub(crate) fn resource_kind(self) -> ResourceKind {
        match self {
            Self::Mod => ResourceKind::Mod,
            Self::ResourcePack => ResourceKind::ResourcePack,
            Self::Shader => ResourceKind::Shader,
        }
    }
}

/// 文件名是 `ext`（启用）或 `ext + .disabled`（禁用）就返回对应状态，其它文件不算数。
fn pack_enabled(file_name: &str, ext: &str) -> Option<bool> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(&format!("{ext}.disabled")) {
        Some(false)
    } else if lower.ends_with(ext) {
        Some(true)
    } else {
        None
    }
}

/// 把文件名换成另一个状态的写法：`x.zip` ↔ `x.zip.disabled`。
///
/// 已经是目标状态或后缀不匹配时返回 `None`，不用重命名。
fn toggled_pack_name(file_name: &str, ext: &str, enabled: bool) -> Option<String> {
    match (enabled, pack_enabled(file_name, ext)?) {
        (true, false) => {
            // `.disabled` 是纯 ASCII，按字节截断不会切在多字节字符中间。
            Some(file_name[..file_name.len() - ".disabled".len()].to_owned())
        }
        (false, true) => Some(format!("{file_name}.disabled")),
        _ => None,
    }
}

/// 扫一个资源目录，只收 `ext` / `ext + .disabled` 文件，按文件名排序。
fn scan_packs(dir: &Path, ext: &str) -> Vec<PackEntry> {
    let mut packs = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return packs;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(enabled) = pack_enabled(&file_name, ext) else {
            continue;
        };

        packs.push(PackEntry {
            path,
            file_name,
            enabled,
        });
    }

    packs.sort_by_key(|entry| entry.file_name.to_ascii_lowercase());
    packs
}

/// 资源文件列表：磁盘快照存在这里，点刷新、换项目或换栏目时才重新扫描。
struct PackListDelegate {
    /// 当前列表是哪一类资源，决定空态文案的后缀叫法。
    kind: PackKind,
    packs: Vec<PackEntry>,
    /// 搜索框当前的内容，过滤出要显示的行。
    query: String,
    selected_index: Option<IndexPath>,
    /// 勾选框的回调需要回到页面，所以留一个页面的弱引用。
    page: WeakEntity<DevelopPage>,
}

impl PackListDelegate {
    /// 按搜索词过滤后的条目，文件名不区分大小写做子串匹配。
    fn visible_packs(&self) -> Vec<&PackEntry> {
        let query = self.query.trim().to_ascii_lowercase();
        self.packs
            .iter()
            .filter(|entry| {
                query.is_empty() || entry.file_name.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }
}

impl ListDelegate for PackListDelegate {
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
        self.visible_packs().len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let visible = self.visible_packs();
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
                            Checkbox::new(SharedString::from(format!(
                                "pack-enabled-{}",
                                ix.row
                            )))
                            .checked(enabled)
                            .tooltip(if enabled {
                                "点击禁用"
                            } else {
                                "点击启用"
                            })
                            .on_click(move |checked, window, cx| {
                                let _ = page.update(cx, |page, cx| {
                                    page.set_pack_enabled(path.clone(), *checked, window, cx);
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
        let text = if self.packs.is_empty() {
            format!("{} 目录里没有{}", self.kind.dir_name(), self.kind.noun())
        } else {
            format!("没有匹配搜索的{}", self.kind.noun())
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
    /// 资源文件列表（模组/资源包/光影共用），条目是渲染期扫出来的磁盘快照。
    pack_list: Entity<ListState<PackListDelegate>>,
    /// 当前列表对应的资源类别，切换侧边栏后需要重新扫描。
    pack_kind: Option<PackKind>,
    /// 当前列表对应的资源目录，用来发现项目切换后需要重新扫描。
    pack_dir: Option<PathBuf>,
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
        let pack_list = cx.new(|cx| {
            ListState::new(
                PackListDelegate {
                    // 首次进入哪个栏目，sync_packs 会把 kind 拨过去。
                    kind: PackKind::Mod,
                    packs: Vec::new(),
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
            pack_list,
            pack_kind: None,
            pack_dir: None,
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
            // 模组/资源包/光影共用同一个列表，只是目录和后缀不同。
            _ => self.render_pack_list(window, cx).into_any_element(),
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

    /// 当前侧边栏项对应的资源目录（如 `<project>/resourcepacks`），没选项目或「通用」页为 `None`。
    fn current_pack_dir(&self) -> Option<PathBuf> {
        let kind = PackKind::from_section(self.section)?;
        self.project_select
            .selected_project()
            .map(|project| project.path.join(kind.dir_name()))
    }

    /// 选中项目或侧边栏变了就重新扫描；没变化什么都不做。
    ///
    /// 放在渲染期做，和 `project_select.sync` 一个思路，不用额外维护「项目变了」的通知。
    fn sync_packs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = PackKind::from_section(self.section);
        let pack_dir = self.current_pack_dir();
        if kind != self.pack_kind || pack_dir != self.pack_dir {
            self.reload_packs(window, cx);
        }
    }

    /// 重新扫描 `self.pack_dir` 并刷新列表。
    fn reload_packs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = PackKind::from_section(self.section).unwrap_or(PackKind::Mod);
        let kind_changed = Some(kind) != self.pack_kind;
        self.pack_kind = Some(kind);
        self.pack_dir = self.current_pack_dir();
        let packs = self
            .pack_dir
            .as_ref()
            .map(|dir| scan_packs(dir, kind.ext()))
            .unwrap_or_default();

        self.pack_list.update(cx, |state, cx| {
            state.delegate_mut().kind = kind;
            state.delegate_mut().packs = packs;
            state.delegate_mut().selected_index = None;
            // 换栏目时把搜索词清掉，不然新的列表会被上个栏目的词过滤掉。
            if kind_changed {
                state.set_query("", window, cx);
            }
            cx.notify();
        });
    }

    /// 刷新按钮：重新扫描当前项目的资源目录。
    fn refresh_packs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_packs(window, cx);
        cx.notify();
    }

    /// 「序列化」/「反序列化」按钮：启动任务并打开进度弹窗。
    ///
    /// 任务本身由弹窗负责启动和观察；没选项目（`pack_dir` 为空时按钮也不会渲染，
    /// 这里是双保险）就什么都不做。
    fn open_sync_dialog(&mut self, serialize: bool, window: &mut Window, cx: &mut Context<Self>) {
        // 已经有别的弹窗开着就别再叠一层了。
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(project) = self.project_select.selected_project().cloned() else {
            return;
        };
        let Some(kind) = PackKind::from_section(self.section) else {
            return;
        };

        let page = cx.entity().downgrade();
        ResourceSyncDialog::open(
            &project,
            kind,
            serialize,
            Rc::new(move |window, cx| {
                let Some(page) = page.upgrade() else {
                    return;
                };
                page.update(cx, |page, cx| page.reload_packs(window, cx));
            }),
            window,
            cx,
        );
    }

    /// 强制同步按钮：删掉多余的一边，让资源文件和 toml 记录一一对应。
    ///
    /// `by_records` 为 `true` 时以记录为准，删掉资源目录里没有 toml 的文件；
    /// 为 `false` 时以文件为准，删掉 `.rev_launcher/<目录>` 里没有资源文件的记录。
    fn force_sync(&mut self, by_records: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project_select.selected_project().cloned() else {
            return;
        };
        let Some(kind) = PackKind::from_section(self.section) else {
            return;
        };
        let noun = kind.noun();

        let result = if by_records {
            prune_files_without_record(&project, kind.resource_kind())
        } else {
            prune_records_without_file(&project, kind.resource_kind())
        };

        match result {
            Ok(deleted) if deleted.is_empty() => {
                window.push_notification(
                    (NotificationType::Info, "文件和记录已经一致，无需删除".to_owned()),
                    cx,
                );
            }
            Ok(deleted) => {
                let target = if by_records { "文件" } else { "记录" };
                window.push_notification(
                    (
                        NotificationType::Success,
                        format!("已删除 {} 个多余{noun}{target}", deleted.len()),
                    ),
                    cx,
                );
            }
            Err(error) => {
                window.push_notification(
                    (NotificationType::Error, format!("同步失败：{error}")),
                    cx,
                );
            }
        }

        // 不管成败都重新扫一遍，让列表和磁盘保持一致。
        self.reload_packs(window, cx);
        cx.notify();
    }

    /// 勾选框切换：把文件在正常后缀和追加 `.disabled` 之间重命名。
    fn set_pack_enabled(
        &mut self,
        path: PathBuf,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext = self.pack_list.read(cx).delegate().kind.ext();
        let new_path = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| toggled_pack_name(name, ext, enabled))
            .map(|name| path.with_file_name(name));

        if let Some(new_path) = new_path
            && let Err(error) = std::fs::rename(&path, &new_path)
        {
            window.push_notification(
                (
                    gpui_kit::component::notification::NotificationType::Error,
                    format!("切换{}状态失败: {error}", self.pack_list.read(cx).delegate().kind.noun()),
                ),
                cx,
            );
        }

        // 不管成败都重新扫一遍，让列表和磁盘保持一致。
        self.reload_packs(window, cx);
        cx.notify();
    }

    fn render_pack_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 顶部下拉框换项目、侧边栏换栏目后，这里负责把列表切到对应目录。
        self.sync_packs(window, cx);

        let Some(pack_dir) = self.pack_dir.clone() else {
            return v_flex().size_full().child(
                card_outline()
                    .child(Label::new(self.section.label()))
                    .child(Label::new("当前没有选中的整合包").text_color(rgb(0x9ca3af))),
            );
        };
        let noun = PackKind::from_section(self.section)
            .unwrap_or(PackKind::Mod)
            .noun();

        let toolbar = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                Button::new("refresh-packs")
                    .icon(IconName::RotateCw)
                    .label("刷新")
                    .on_click(cx.listener(|this, _, window, cx| this.refresh_packs(window, cx))),
            )
            .child(
                Button::new("open-pack-folder")
                    .icon(IconName::FolderOpen)
                    .label("打开文件夹")
                    .on_click(move |_, _, cx| {
                        // 目录还不存在时先建出来，免得系统资源管理器报错。
                        let _ = std::fs::create_dir_all(&pack_dir);
                        cx.reveal_path(&pack_dir);
                    }),
            )
            .child(div().flex_1())
            .child(
                Button::new("serialize-packs")
                    .icon(IconName::ArrowUp)
                    .label(format!("序列化{noun}"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_sync_dialog(true, window, cx);
                    })),
            )
            .child(
                Button::new("deserialize-packs")
                    .icon(IconName::ArrowDown)
                    .label(format!("反序列化{noun}"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_sync_dialog(false, window, cx);
                    })),
            )
            .child(
                Button::new("prune-packs-records")
                    .icon(IconName::Delete)
                    .label("删除多余记录")
                    .tooltip(format!("删掉没有对应{noun}文件的 toml 记录"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.force_sync(false, window, cx);
                    })),
            )
            .child(
                Button::new("prune-packs-files")
                    .icon(IconName::Delete)
                    .label("删除多余文件")
                    .tooltip(format!("删掉没有 toml 记录的{noun}文件"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.force_sync(true, window, cx);
                    })),
            );

        v_flex()
            .size_full()
            .gap_2()
            .child(toolbar)
            .child(
                List::new(&self.pack_list)
                    .flex_1()
                    .search_placeholder(format!("搜索{noun}"))
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
