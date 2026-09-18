use gpui_kit::base::{Disableable, StyledExt, v_flex};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::label::Label;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::{
    AnyElement, Context, Div, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, px, rgb,
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

pub struct DevelopPage {
    /// 当前选中的侧边栏项，每个菜单项是否高亮由它决定。
    section: DevelopSection,
    /// 项目下拉框，和启动页、VCS 页共用一份实现；选中项写回设置，几个页面自然同步。
    project_select: ProjectSelect,
    /// 保活项目下拉框的事件订阅。
    _select_subscription: Subscription,
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

        Self {
            section: DevelopSection::General,
            project_select,
            _select_subscription: select_subscription,
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
