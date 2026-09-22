//! 下载页：左侧侧边栏切换下载类型，右侧展示对应内容。
//!
//! 目前只实现了「原版下载」：从 Mojang 的版本清单拉取所有版本，
//! 下载后会在设置的版本目录里建一个项目，游戏文件留到首次启动时再拉。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, IconName, IndexPath, StyledExt, WindowExt};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Task,
    Window, div, rgb,
};

use mclib::project::versions::minecreft::{MinecreftVersions, Version};

mod dialog;
use dialog::DownloadDialog;

/// 下载页侧边栏里可以被选中的项。
#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadSection {
    Vanilla,
    Modpack,
    Mods,
    ResourcePacks,
    Shaders,
}

impl DownloadSection {
    fn label(self) -> &'static str {
        match self {
            Self::Vanilla => "原版下载",
            Self::Modpack => "整合包下载",
            Self::Mods => "模组下载",
            Self::ResourcePacks => "资源包下载",
            Self::Shaders => "光影下载",
        }
    }
}

/// 版本清单的获取状态，渲染期根据它决定显示加载、错误还是列表。
enum ManifestStatus {
    /// 还没发起请求。
    Idle,
    /// 正在请求版本清单。
    Loading,
    /// 版本清单已就位。
    Ready,
    /// 请求失败，记录错误信息供界面展示和重试。
    Failed(String),
}

/// 原版版本列表：展示网络拉回来的清单快照。
///
/// 选中项要带着版本本体，因为过滤后行号和 `versions` 下标对不上，
/// 靠行号反查会拿错版本。
struct VersionListDelegate {
    versions: Vec<Version>,
    /// 是否显示正式版。
    show_release: bool,
    /// 是否显示快照，旧版 beta/alpha 也归在这一类。
    show_snapshot: bool,
    /// 搜索框当前的内容，按版本号做子串匹配。
    query: String,
    selected_index: Option<IndexPath>,
    /// 当前选中的版本，工具栏的「下载」按钮用它。
    selected_version: Option<Version>,
}

impl VersionListDelegate {
    /// 按类型开关和搜索词过滤后的条目，版本号不区分大小写做子串匹配。
    fn visible_versions(&self) -> Vec<&Version> {
        let query = self.query.trim().to_ascii_lowercase();
        self.versions
            .iter()
            .filter(|version| {
                let shown = match version._type.as_str() {
                    "release" => self.show_release,
                    // snapshot、old_beta、old_alpha 都算快照一类。
                    _ => self.show_snapshot,
                };
                shown && (query.is_empty() || version.id.to_ascii_lowercase().contains(&query))
            })
            .collect()
    }
}

/// 版本类型的展示标签。
fn version_type_tag(kind: &str) -> Tag {
    match kind {
        "release" => Tag::success().child("正式版"),
        "snapshot" => Tag::warning().child("快照"),
        "old_beta" => Tag::secondary().child("beta"),
        "old_alpha" => Tag::secondary().child("alpha"),
        _ => Tag::secondary().child(kind.to_owned()),
    }
}

impl ListDelegate for VersionListDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_owned();
        self.selected_index = None;
        self.selected_version = None;
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.visible_versions().len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let visible = self.visible_versions();
        // `get` 拿到的是 `&&Version`，要解引用再 clone 才是 owned 的 `Version`。
        let version = (*visible.get(ix.row)?).clone();

        Some(
            ListItem::new(ix)
                .selected(Some(ix) == self.selected_index)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.delegate_mut().set_selected_index(Some(ix), window, cx);
                }))
                .child(version_row(&version)),
        )
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let text = if self.versions.is_empty() {
            "没有获取到版本信息"
        } else {
            "没有匹配搜索的版本"
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
        // 行号和 `versions` 下标对不上，选中项要存版本本体。
        self.selected_version = ix
            .and_then(|ix| self.visible_versions().get(ix.row).map(|v| (*v).clone()));
        cx.notify();
    }
}

/// 列表里的一行：版本号 + 类型标签 + 发布时间。
fn version_row(version: &Version) -> AnyElement {
    let mut tags = h_flex().flex_none().items_center().gap_1();
    tags = tags.child(version_type_tag(&version._type));
    // `releaseTime` 是 RFC3339，列表里只留日期部分。
    let release_date = version.release_time.split('T').next().unwrap_or_default();
    if !release_date.is_empty() {
        tags = tags.child(Tag::secondary().outline().child(release_date.to_owned()));
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
                .child(Label::new(version.id.clone())),
        )
        .child(tags)
        .into_any_element()
}

pub struct DownloadPage {
    /// 当前选中的侧边栏项。
    section: DownloadSection,
    /// 版本清单的获取状态。
    manifest_status: ManifestStatus,
    /// 最新正式版/快照版本号，展示在工具栏上。
    latest: Option<(String, String)>,
    /// 原版版本列表。
    version_list: Entity<ListState<VersionListDelegate>>,
}

impl DownloadPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let version_list = cx.new(|cx| {
            ListState::new(
                VersionListDelegate {
                    versions: Vec::new(),
                    show_release: true,
                    show_snapshot: false,
                    query: String::new(),
                    selected_index: None,
                    selected_version: None,
                },
                window,
                cx,
            )
            // 列表自带搜索框，输入时回调 delegate 的 perform_search。
            .searchable(true)
        });

        let mut this = Self {
            section: DownloadSection::Vanilla,
            manifest_status: ManifestStatus::Idle,
            latest: None,
            version_list,
        };
        this.load_versions(window, cx);
        this
    }

    /// 后台拉取 Mojang 的版本清单，结果写回列表。
    fn load_versions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.manifest_status, ManifestStatus::Loading) {
            return;
        }

        self.manifest_status = ManifestStatus::Loading;
        cx.notify();

        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { MinecreftVersions::get_versions() })
                .await;

            let _ = page.update_in(cx, |page, _window, cx| {
                match result {
                    Ok(manifest) => {
                        page.latest =
                            Some((manifest.latest.release.clone(), manifest.latest.snapshot));
                        page.manifest_status = ManifestStatus::Ready;
                        page.version_list.update(cx, |state, cx| {
                            let delegate = state.delegate_mut();
                            delegate.versions = manifest.versions;
                            // 清单重新拉过，选中项的版本对象可能已经不是同一个了。
                            delegate.selected_index = None;
                            delegate.selected_version = None;
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        page.manifest_status = ManifestStatus::Failed(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 刷新按钮：重新拉版本清单。
    fn refresh_versions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_versions(window, cx);
    }

    /// 类型筛选开关变化时刷新列表，并清掉可能被过滤掉的选中项。
    fn set_show_kind(&mut self, release: Option<bool>, snapshot: Option<bool>, cx: &mut Context<Self>) {
        self.version_list.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            if let Some(value) = release {
                delegate.show_release = value;
            }
            if let Some(value) = snapshot {
                delegate.show_snapshot = value;
            }
            // 选中项可能被筛掉，直接清掉免得「下载」按钮对着看不见的版本。
            delegate.selected_index = None;
            delegate.selected_version = None;
            cx.notify();
        });
    }

    /// 点「下载」：对当前选中的版本弹出下载对话框。
    fn open_download_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }

        let Some(version) = self
            .version_list
            .read(cx)
            .delegate()
            .selected_version
            .clone()
        else {
            window.push_notification(
                (NotificationType::Error, "请先在列表中选择一个版本"),
                cx,
            );
            return;
        };

        let dialog = cx.new(|cx| DownloadDialog::new(version, window, cx));
        DownloadDialog::open(dialog, window, cx);
    }

    /// 选中某一项。必须 `notify`，否则 gpui 不会重新渲染，`active` 高亮也不会跟着变。
    fn select(&mut self, section: DownloadSection, cx: &mut Context<Self>) {
        if self.section != section {
            self.section = section;
            cx.notify();
        }
    }

    /// 生成一个菜单项：自己是不是当前选中项决定是否高亮，点击后把自己设为选中项。
    fn menu_item(&self, section: DownloadSection, cx: &mut Context<Self>) -> SidebarMenuItem {
        SidebarMenuItem::new(section.label())
            .active(self.section == section)
            .on_click(cx.listener(move |this, _, _, cx| this.select(section, cx)))
    }

    /// 还没实现的菜单项：保持可看点不了，免得点了没反应。
    fn placeholder_item(&self, section: DownloadSection) -> SidebarMenuItem {
        SidebarMenuItem::new(section.label()).disable(true)
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Sidebar<SidebarGroup<SidebarMenu>> {
        Sidebar::new("download_sidebar")
            .child(
                SidebarGroup::new("游戏下载").child(
                    SidebarMenu::new()
                        .child(self.menu_item(DownloadSection::Vanilla, cx))
                        .child(self.placeholder_item(DownloadSection::Modpack)),
                ),
            )
            .child(
                SidebarGroup::new("资源下载").child(
                    SidebarMenu::new()
                        .child(self.placeholder_item(DownloadSection::Mods))
                        .child(self.placeholder_item(DownloadSection::ResourcePacks))
                        .child(self.placeholder_item(DownloadSection::Shaders)),
                ),
            )
    }

    /// 原版下载的内容区：加载中/加载失败/版本列表三种状态。
    fn render_vanilla(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match &self.manifest_status {
            ManifestStatus::Idle | ManifestStatus::Loading => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Spinner::new())
                .child(Label::new("正在获取版本列表…"))
                .into_any_element(),
            ManifestStatus::Failed(error) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Label::new(format!("获取版本列表失败：{error}")))
                .child(
                    Button::new("retry-fetch-versions")
                        .icon(IconName::RotateCw)
                        .label("重试")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.load_versions(window, cx);
                        })),
                )
                .into_any_element(),
            ManifestStatus::Ready => {
                let delegate = self.version_list.read(cx).delegate();
                let show_release = delegate.show_release;
                let show_snapshot = delegate.show_snapshot;

                let mut toolbar = h_flex().w_full().items_center().gap_2();
                if let Some((release, snapshot)) = &self.latest {
                    // 和列表里的版本类型标签用同一套颜色。
                    toolbar = toolbar
                        .child(Tag::success().child(format!("最新正式版 {release}")))
                        .child(Tag::warning().child(format!("最新快照 {snapshot}")));
                }
                toolbar = toolbar
                    .child(div().flex_1())
                    .child(
                        Checkbox::new("filter-release")
                            .label("正式版")
                            .checked(show_release)
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.set_show_kind(Some(*checked), None, cx);
                            })),
                    )
                    .child(
                        Checkbox::new("filter-snapshot")
                            .label("快照")
                            .checked(show_snapshot)
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.set_show_kind(None, Some(*checked), cx);
                            })),
                    )
                    .child(
                        Button::new("refresh-versions")
                            .icon(IconName::RotateCw)
                            .label("刷新")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.refresh_versions(window, cx);
                            })),
                    )
                    .child(
                        Button::new("download-version")
                            .icon(IconName::ArrowDown)
                            .label("下载")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_download_dialog(window, cx);
                            })),
                    );

                v_flex()
                    .size_full()
                    .gap_2()
                    .child(toolbar)
                    .child(
                        List::new(&self.version_list)
                            .flex_1()
                            .search_placeholder("搜索版本")
                            // 给列表加个边框，行内容稍微内缩一点，不贴着边框。
                            .p_1()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(cx.theme().radius),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for DownloadPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.render_sidebar(cx);
        let content = self.render_vanilla(window, cx);

        // 父容器（main_window 的内容槽）是 block 布局，这里必须用 size_full 拿确定高度，
        // 不能用 flex_1；否则侧栏的 h_full 和列表的 flex_1 都会塌缩成 0。
        div()
            .h_flex()
            .size_full()
            .items_start()
            .gap_2()
            .child(sidebar)
            .child(div().flex_1().min_w_0().h_full().child(content))
    }
}
