use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, IndexPath, StyledExt, WindowExt, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, div, hsla, px, svg,
};
use std::collections::HashSet;

use crate::jj;
use crate::window::project_select::{ProjectList, ProjectSelect};

#[derive(Debug, Clone)]
struct GraphRow {
    node_lane: usize,
    lanes_before: Vec<bool>,
    lanes_after: Vec<bool>,
    parent_lanes: Vec<usize>,
    lane_count: usize,
    is_root: bool,
    has_hidden_parent: bool,
}

fn graph_rows(commits: &[jj::CommitHistoryItem]) -> Vec<GraphRow> {
    let visible_commit_ids = commits
        .iter()
        .map(|commit| commit.id.as_str())
        .collect::<HashSet<_>>();
    let mut lanes: Vec<String> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let lanes_before = lanes.iter().map(|_| true).collect::<Vec<_>>();
        let node_lane = if let Some(node_lane) = lanes.iter().position(|lane| lane == &commit.id) {
            node_lane
        } else {
            lanes.push(commit.id.clone());
            lanes.len() - 1
        };
        lanes.remove(node_lane);

        for parent in commit
            .parents
            .iter()
            .filter(|parent| visible_commit_ids.contains(parent.as_str()))
            .rev()
        {
            if let Some(existing_lane) = lanes.iter().position(|lane| lane == parent) {
                lanes.remove(existing_lane);
            }
            let insert_at = node_lane.min(lanes.len());
            lanes.insert(insert_at, parent.clone());
        }

        let parent_lanes = commit
            .parents
            .iter()
            .filter(|parent| visible_commit_ids.contains(parent.as_str()))
            .filter_map(|parent| lanes.iter().position(|lane| lane == parent))
            .collect::<Vec<_>>();
        let lanes_after = lanes.iter().map(|_| true).collect::<Vec<_>>();
        let lane_count = lanes_before
            .len()
            .max(lanes_after.len())
            .max(node_lane + 1)
            .max(parent_lanes.iter().copied().max().unwrap_or(0) + 1);

        rows.push(GraphRow {
            node_lane,
            lanes_before,
            lanes_after,
            parent_lanes,
            lane_count,
            is_root: commit.parents.is_empty(),
            has_hidden_parent: commit
                .parents
                .iter()
                .any(|parent| !visible_commit_ids.contains(parent.as_str())),
        });
    }

    rows
}

const GRAPH_ROW_HEIGHT: f32 = 50.;
const GRAPH_NODE_CENTER_Y: f32 = 15.;
const GRAPH_SIDE_EDGE_Y: f32 = 33.;
const GRAPH_LANE_WIDTH: f32 = 16.;
const GRAPH_LEFT_PADDING: f32 = 10.;
const GRAPH_LINE_WIDTH: f32 = 2.;

fn graph_element(
    row: &GraphRow,
    line_color: gpui_kit::Hsla,
    node_color: gpui_kit::Hsla,
) -> AnyElement {
    let center_y = GRAPH_NODE_CENTER_Y;
    let mut graph = div()
        .relative()
        .flex_none()
        .w(px(
            GRAPH_LEFT_PADDING + GRAPH_LANE_WIDTH * row.lane_count as f32
        ))
        .h(px(GRAPH_ROW_HEIGHT));

    for lane in 0..row.lane_count {
        let before = row.lanes_before.get(lane).copied().unwrap_or(false);
        let after = row.lanes_after.get(lane).copied().unwrap_or(false);
        let x = GRAPH_LEFT_PADDING + lane as f32 * GRAPH_LANE_WIDTH;

        if before {
            graph = graph.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top_0()
                    .w(px(GRAPH_LINE_WIDTH))
                    .h(px(center_y))
                    .bg(line_color),
            );
        }
        if after {
            graph = graph.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(center_y))
                    .w(px(GRAPH_LINE_WIDTH))
                    .h(px(GRAPH_ROW_HEIGHT - center_y))
                    .bg(line_color),
            );
        }
    }

    let node_x = GRAPH_LEFT_PADDING + row.node_lane as f32 * GRAPH_LANE_WIDTH;
    if row.has_hidden_parent {
        graph = graph.child(
            div()
                .absolute()
                .left(px(node_x))
                .top(px(center_y))
                .w(px(GRAPH_LINE_WIDTH))
                .h(px(GRAPH_ROW_HEIGHT - center_y))
                .bg(line_color),
        );
    }

    for parent_lane in &row.parent_lanes {
        let parent_x = GRAPH_LEFT_PADDING + *parent_lane as f32 * GRAPH_LANE_WIDTH;
        if (parent_x - node_x).abs() > f32::EPSILON {
            let left = node_x.min(parent_x);
            if row.parent_lanes.len() == 1 {
                // A single-parent branch changes lanes below the node. This
                // keeps the turn connected to the parent lane instead of
                // drawing a horizontal line directly through the node.
                let edge_y = GRAPH_SIDE_EDGE_Y;
                graph = graph
                    .child(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(edge_y - 1.))
                            .w(px((node_x - parent_x).abs() + 2.))
                            .h(px(GRAPH_LINE_WIDTH))
                            .bg(line_color),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(node_x))
                            .top(px(center_y))
                            .w(px(GRAPH_LINE_WIDTH))
                            .h(px(edge_y - center_y + 1.))
                            .bg(line_color),
                    );
            } else {
                graph = graph.child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(center_y - 1.))
                        .w(px((node_x - parent_x).abs() + 2.))
                        .h(px(GRAPH_LINE_WIDTH))
                        .bg(line_color),
                );
            }
        }
    }

    let node = if row.is_root {
        svg()
            .absolute()
            .left(px(node_x - 4.))
            .top(px(center_y - 4.))
            .w(px(10.))
            .h(px(10.))
            .text_color(node_color)
            .data(br#"<svg viewBox="0 0 10 10"><path d="M5 0 L10 5 L5 10 L0 5 Z"/></svg>"#)
            .into_any_element()
    } else {
        div()
            .absolute()
            .left(px(node_x - 4.))
            .top(px(center_y - 4.))
            .w(px(10.))
            .h(px(10.))
            .rounded(px(5.))
            .bg(node_color)
            .into_any_element()
    };

    graph.child(node).into_any_element()
}

fn unique_prefix_lengths(ids: &[String]) -> Vec<usize> {
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            if id.is_empty() {
                return 0;
            }

            (1..=id.len())
                .find(|length| {
                    let prefix = &id[..*length];
                    ids.iter().enumerate().all(|(other_index, other)| {
                        other_index == index || !other.starts_with(prefix)
                    })
                })
                .unwrap_or(id.len())
        })
        .collect()
}

fn colored_id(
    id: &str,
    unique_prefix_length: usize,
    prefix_color: Hsla,
    normal_color: Hsla,
) -> AnyElement {
    let prefix_length = unique_prefix_length.min(id.len());
    let mut result = h_flex().flex_none();

    if prefix_length > 0 {
        result = result.child(
            Label::new(id[..prefix_length].to_owned())
                .text_sm()
                .text_color(prefix_color),
        );
    }
    if prefix_length < id.len() {
        result = result.child(
            Label::new(id[prefix_length..].to_owned())
                .text_sm()
                .text_color(normal_color),
        );
    }

    result.into_any_element()
}

fn change_id_color() -> Hsla {
    hsla(330.0 / 360.0, 0.82, 0.68, 1.0)
}

fn commit_id_color() -> Hsla {
    hsla(270.0 / 360.0, 0.72, 0.70, 1.0)
}

fn local_bookmark_color() -> Hsla {
    hsla(210.0 / 360.0, 0.85, 0.68, 1.0)
}

fn remote_bookmark_color() -> Hsla {
    hsla(30.0 / 360.0, 0.90, 0.68, 1.0)
}

#[derive(Clone)]
struct LocalBookmarkDrag {
    name: String,
}

struct BookmarkDragPreview {
    name: String,
}

impl Render for BookmarkDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(local_bookmark_color().opacity(0.2))
            .child(Label::new(self.name.clone()).text_color(local_bookmark_color()))
    }
}

fn bookmark_element(bookmark: &jj::Bookmark, disabled: bool) -> AnyElement {
    let (display_name, color, is_local) = match &bookmark.kind {
        jj::BookmarkKind::Local => (bookmark.name.clone(), local_bookmark_color(), true),
        jj::BookmarkKind::Remote(remote) => (
            format!("{}@{}", bookmark.name, remote),
            remote_bookmark_color(),
            false,
        ),
    };

    let element = div()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1()
        .py_0()
        .rounded(px(4.))
        .border_1()
        .border_color(color.opacity(0.55))
        .bg(color.opacity(0.18))
        .text_sm()
        .text_color(color)
        .child(Label::new(display_name).text_color(color));
    if is_local && !disabled {
        return element
            .id(format!("bookmark-drag-{}", bookmark.name))
            .on_drag(
                LocalBookmarkDrag {
                    name: bookmark.name.clone(),
                },
                |drag, _, _, cx| {
                    cx.new(|_| BookmarkDragPreview {
                        name: drag.name.clone(),
                    })
                },
            )
            .into_any_element();
    }
    element.into_any_element()
}

struct CommitListDelegate {
    commits: Vec<jj::CommitHistoryItem>,
    graph_rows: Vec<GraphRow>,
    change_id_prefixes: Vec<usize>,
    commit_id_prefixes: Vec<usize>,
    working_copy_commit_id: Option<String>,
    selected_index: Option<IndexPath>,
    error: Option<String>,
    disabled: bool,
    page: WeakEntity<VcsPage>,
}

impl CommitListDelegate {
    fn new(page: WeakEntity<VcsPage>) -> Self {
        Self {
            commits: Vec::new(),
            graph_rows: Vec::new(),
            change_id_prefixes: Vec::new(),
            commit_id_prefixes: Vec::new(),
            working_copy_commit_id: None,
            selected_index: None,
            error: None,
            disabled: false,
            page,
        }
    }

    fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        if disabled {
            self.selected_index = None;
        }
    }

    fn set_loading(&mut self) {
        self.commits.clear();
        self.graph_rows.clear();
        self.change_id_prefixes.clear();
        self.commit_id_prefixes.clear();
        self.working_copy_commit_id = None;
        self.selected_index = None;
        self.error = Some("正在读取提交历史…".to_owned());
    }

    fn set_no_project(&mut self) {
        self.commits.clear();
        self.graph_rows.clear();
        self.change_id_prefixes.clear();
        self.commit_id_prefixes.clear();
        self.working_copy_commit_id = None;
        self.selected_index = None;
        self.error = Some("当前没有选中的整合包".to_owned());
    }

    fn set_history_result(&mut self, result: anyhow::Result<jj::CommitHistory>) {
        match result {
            Ok(history) => {
                self.graph_rows = graph_rows(&history.commits);
                self.commits = history.commits;
                self.change_id_prefixes = unique_prefix_lengths(
                    &self
                        .commits
                        .iter()
                        .map(|commit| format!("{:.8}", commit.change_id))
                        .collect::<Vec<_>>(),
                );
                self.commit_id_prefixes = unique_prefix_lengths(
                    &self
                        .commits
                        .iter()
                        .map(|commit| format!("{:.8}", commit.id))
                        .collect::<Vec<_>>(),
                );
                self.working_copy_commit_id = history.working_copy_commit_id;
                self.selected_index = None;
                self.error = None;
            }
            Err(error) => {
                self.commits.clear();
                self.graph_rows.clear();
                self.change_id_prefixes.clear();
                self.commit_id_prefixes.clear();
                self.working_copy_commit_id = None;
                self.selected_index = None;
                self.error = Some(format!("不是 jj 仓库或无法读取提交历史：{error}"));
            }
        }
    }
}

impl ListDelegate for CommitListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.commits.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let commit = self.commits.get(ix.row)?.clone();
        let graph_row = self.graph_rows.get(ix.row)?.clone();
        let page = self.page.clone();
        let commit_id = commit.id.clone();
        let is_current = self.working_copy_commit_id.as_ref() == Some(&commit.id);
        let normal_id_color = cx.theme().muted_foreground;
        let change_id_prefix_length = self.change_id_prefixes.get(ix.row).copied().unwrap_or(0);
        let short_change_id = format!("{:.8}", commit.change_id);
        let short_commit_id = format!("{:.8}", commit.id);
        let commit_id_prefix_length = self.commit_id_prefixes.get(ix.row).copied().unwrap_or(0);
        let disabled = self.disabled;
        let drop_page = self.page.clone();
        let drop_commit_id = commit.id.clone();
        let mut bookmark_elements = h_flex().flex_none().gap_1();
        for bookmark in &commit.bookmarks {
            bookmark_elements = bookmark_elements.child(bookmark_element(bookmark, disabled));
        }
        let description_line = h_flex()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(Label::new(commit.description.clone())),
            )
            .child(bookmark_elements);

        let mut meta = h_flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(colored_id(
                &short_change_id,
                change_id_prefix_length,
                change_id_color(),
                normal_id_color,
            ))
            .child(commit.timestamp.clone())
            .child(commit.author.clone());
        if is_current {
            meta = meta.child(Tag::info().child("当前"));
        }

        Some(
            ListItem::new(SharedString::from(format!("commit-{ix:?}")))
                // The graph must occupy the complete row height so adjacent rows share
                // their boundary; ListItem's default vertical padding otherwise creates
                // a visible gap in every topology edge.
                .py_0()
                .opacity(if disabled { 0.45 } else { 1.0 })
                .selected(Some(ix) == self.selected_index)
                .child(
                    h_flex()
                        .w_full()
                        .h(px(GRAPH_ROW_HEIGHT))
                        .items_center()
                        .gap_2()
                        .child(graph_element(
                            &graph_row,
                            gpui_kit::black(),
                            gpui_kit::black(),
                        ))
                        .child(
                            div()
                                .v_flex()
                                .min_w_0()
                                .flex_1()
                                .gap_1()
                                .child(description_line)
                                .child(meta),
                        )
                        .child(colored_id(
                            &short_commit_id,
                            commit_id_prefix_length,
                            commit_id_color(),
                            normal_id_color,
                        )),
                )
                .drag_over::<LocalBookmarkDrag>(|style, _, _, _| {
                    style.bg(local_bookmark_color().opacity(0.12))
                })
                .on_drop(
                    cx.listener(move |_, drag: &LocalBookmarkDrag, window, _cx| {
                        if disabled {
                            return;
                        }
                        let page = drop_page.clone();
                        let bookmark_name = drag.name.clone();
                        let commit_id = drop_commit_id.clone();
                        window.on_next_frame(move |window, cx| {
                            let _ = page.update(cx, |page, cx| {
                                page.move_bookmark(bookmark_name, commit_id, window, cx);
                            });
                        });
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if disabled {
                        return;
                    }
                    this.delegate_mut().set_selected_index(Some(ix), window, cx);
                    if event.click_count() == 2 {
                        let page = page.clone();
                        let commit_id = commit_id.clone();
                        window.on_next_frame(move |window, cx| {
                            let _ = page.update(cx, |page, cx| {
                                page.checkout_commit(commit_id, window, cx);
                            });
                        });
                    }
                })),
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
            .child(Label::new(
                self.error
                    .clone()
                    .unwrap_or_else(|| "当前整合包没有历史提交".to_owned()),
            ))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        if self.disabled {
            return;
        }
        self.selected_index = ix;
        cx.notify();
    }
}

/// 工具栏上的两个远程操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteCommand {
    Fetch,
    Push,
}

impl RemoteCommand {
    /// 通知文案里用的名字。
    fn label(self) -> &'static str {
        match self {
            Self::Fetch => "拉取",
            Self::Push => "推送",
        }
    }
}

/// 页面上正在跑的仓库操作。
///
/// 同一时刻只允许一个：切换提交、移动 bookmark、拉取、推送都会改工作副本或者仓库
/// 状态，串起来才不会互相踩。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VcsOperation {
    Checkout,
    MoveBookmark,
    Remote(RemoteCommand),
}

/// 远程列表更新后该选中哪个：原来选中的还在就用它，否则退回第一个。
fn resolve_remote(remotes: &[String], previous: Option<String>) -> Option<String> {
    previous
        .filter(|name| remotes.iter().any(|remote| remote == name))
        .or_else(|| remotes.first().cloned())
}

/// 远程名在下拉框选项里的下标，也就是下拉框的选中项。
fn remote_index(remotes: &[String], remote: Option<&String>) -> Option<IndexPath> {
    remote
        .and_then(|remote| remotes.iter().position(|name| name == remote))
        .map(IndexPath::new)
}

pub struct VcsPage {
    commit_state: Entity<ListState<CommitListDelegate>>,
    /// 项目下拉框，和启动页共用一份实现；选中项写回设置，两个页面自然同步。
    project_select: ProjectSelect,
    /// 保活项目下拉框的事件订阅。
    _select_subscription: Subscription,
    /// 远程仓库下拉框，选项是当前项目的 git remote。
    remote_select: Entity<SelectState<Vec<String>>>,
    /// 保活远程下拉框的事件订阅。
    _remote_subscription: Subscription,
    /// 当前选中的远程名，拉取和推送都作用在它上面。
    selected_remote: Option<String>,
    /// 远程列表的请求号：换项目后，早先那次读远程的结果就作废了。
    remotes_request_id: u64,
    revset: String,
    revset_input: Entity<InputState>,
    _revset_subscription: Subscription,
    history_request_id: u64,
    /// 正在跑的仓库操作，没有就是 `None`。
    operation: Option<VcsOperation>,
}

impl VcsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project_select = ProjectSelect::new(window, cx);
        let select_subscription = cx.subscribe_in(
            project_select.state(),
            window,
            |page: &mut Self, _state, event: &SelectEvent<ProjectList>, window, cx| {
                let SelectEvent::Confirm(Some(path)) = event else {
                    return;
                };

                // 写回设置由下拉框自己负责，启动页下次渲染就会跟上。
                if !page.project_select.confirm(path.clone(), cx) {
                    return;
                }

                // 换了项目，远程仓库和历史都得重新读。
                page.reload_remotes(window, cx);
                page.reload_history(window, cx);
                cx.notify();
            },
        );

        // 远程下拉框一开始是空的，等 `reload_remotes` 后台读完再填。
        let remote_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let remote_subscription = cx.subscribe_in(
            &remote_select,
            window,
            |page: &mut Self, _state, event: &SelectEvent<Vec<String>>, _window, cx| {
                let SelectEvent::Confirm(Some(remote)) = event else {
                    return;
                };
                if page.selected_remote.as_ref() == Some(remote) {
                    return;
                }

                page.selected_remote = Some(remote.clone());
                cx.notify();
            },
        );

        let revset = jj::DEFAULT_REVSET.to_owned();
        let revset_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(jj::DEFAULT_REVSET)
                .placeholder("Revset")
        });
        let page = cx.entity().downgrade();
        let commit_state = cx.new(|cx| ListState::new(CommitListDelegate::new(page), window, cx));
        let revset_subscription = cx.subscribe_in(
            &revset_input,
            window,
            |page, input, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                page.revset = input.read(cx).value().to_string();
                page.reload_history(window, cx);
            },
        );

        let mut page = Self {
            commit_state,
            project_select,
            _select_subscription: select_subscription,
            remote_select,
            _remote_subscription: remote_subscription,
            selected_remote: None,
            remotes_request_id: 0,
            revset,
            revset_input,
            _revset_subscription: revset_subscription,
            history_request_id: 0,
            operation: None,
        };
        page.reload_remotes(window, cx);
        page.reload_history(window, cx);
        page
    }

    fn reload_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_request_id = self.history_request_id.wrapping_add(1);
        let request_id = self.history_request_id;
        let selected_project = self.project_select.selected_path().cloned();
        let revset = self.revset.clone();
        let Some(path) = selected_project else {
            self.commit_state.update(cx, |state, cx| {
                state.delegate_mut().set_no_project();
                cx.notify();
            });
            return;
        };

        self.commit_state.update(cx, |state, cx| {
            state.delegate_mut().set_loading();
            cx.notify();
        });

        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { jj::load_history(path, &revset) })
                .await;
            let _ = page.update(cx, |page, cx| {
                if page.history_request_id != request_id {
                    return;
                }
                page.commit_state.update(cx, |state, cx| {
                    state.delegate_mut().set_history_result(result);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// 后台读当前项目的 git remote 列表，填进下拉框。
    ///
    /// 换项目、点刷新，或者进页面时都会走一遍。
    fn reload_remotes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remotes_request_id = self.remotes_request_id.wrapping_add(1);
        let request_id = self.remotes_request_id;
        let Some(path) = self.project_select.selected_path().cloned() else {
            self.set_remotes(Vec::new(), window, cx);
            return;
        };

        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { jj::load_remotes(path) })
                .await;
            let _ = page.update_in(cx, |page, window, cx| {
                if page.remotes_request_id != request_id {
                    return;
                }
                // 读不到就当没有远程仓库：是不是 jj 仓库、仓库坏没坏，历史列表那边
                // 已经会给出提示，这里再弹一次只是重复。
                let remotes = result.unwrap_or_default();
                page.set_remotes(remotes, window, cx);
            });
        })
        .detach();
    }

    /// 把读回来的远程列表写进下拉框，尽量保留原来的选择。
    fn set_remotes(&mut self, remotes: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let selected = resolve_remote(&remotes, self.selected_remote.clone());
        self.selected_remote = selected.clone();

        let index = remote_index(&remotes, selected.as_ref());
        self.remote_select.update(cx, |state, cx| {
            state.set_items(remotes, window, cx);
            state.set_selected_index(index, window, cx);
        });
    }

    /// 让下拉框跟上设置：设置页/启动页可能换过当前项目，也可能增删过版本目录。
    ///
    /// 选中的项目变了才重新读历史，只是选项列表变了不用重读。
    fn sync_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_select.sync(window, cx) {
            self.reload_remotes(window, cx);
            self.reload_history(window, cx);
        }
    }

    fn checkout_commit(&mut self, commit_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation.is_some() {
            return;
        }

        let Some(path) = self.project_select.selected_path().cloned() else {
            window.push_notification(
                (NotificationType::Error, "当前没有选中的整合包".to_owned()),
                cx,
            );
            return;
        };

        self.operation = Some(VcsOperation::Checkout);
        self.history_request_id = self.history_request_id.wrapping_add(1);
        let request_id = self.history_request_id;
        self.commit_state.update(cx, |state, cx| {
            state.delegate_mut().set_disabled(true);
            cx.notify();
        });

        let checkout_path = path.clone();
        let history_path = path;
        let revset = self.revset.clone();
        let notification_commit_id = commit_id.clone();
        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    jj::checkout(&checkout_path, &commit_id)
                        .map_err(|error| format!("切换提交失败：{error}"))?;
                    jj::load_history(&history_path, &revset)
                        .map_err(|error| format!("读取提交历史失败：{error}"))
                })
                .await;

            let _ = page.update_in(cx, |page, window, cx| {
                page.operation = None;
                page.commit_state.update(cx, |state, cx| {
                    state.delegate_mut().set_disabled(false);
                    cx.notify();
                });
                if page.history_request_id != request_id {
                    return;
                }

                match result {
                    Ok(history) => {
                        page.commit_state.update(cx, |state, cx| {
                            state.delegate_mut().set_history_result(Ok(history));
                            cx.notify();
                        });
                        window.push_notification(
                            (
                                NotificationType::Success,
                                format!("已切换到提交 {:.8}", notification_commit_id),
                            ),
                            cx,
                        );
                    }
                    Err(error) => {
                        page.commit_state.update(cx, |state, cx| {
                            state
                                .delegate_mut()
                                .set_history_result(Err(anyhow::anyhow!(error.clone())));
                            cx.notify();
                        });
                        window.push_notification((NotificationType::Error, error), cx);
                    }
                }
            });
        })
        .detach();
    }

    fn move_bookmark(
        &mut self,
        bookmark_name: String,
        commit_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.operation.is_some() {
            return;
        }

        let Some(path) = self.project_select.selected_path().cloned() else {
            window.push_notification(
                (NotificationType::Error, "当前没有选中的整合包".to_owned()),
                cx,
            );
            return;
        };

        self.operation = Some(VcsOperation::MoveBookmark);
        self.history_request_id = self.history_request_id.wrapping_add(1);
        let request_id = self.history_request_id;
        self.commit_state.update(cx, |state, cx| {
            state.delegate_mut().set_disabled(true);
            cx.notify();
        });

        let revset = self.revset.clone();
        let notification_bookmark = bookmark_name.clone();
        let notification_commit_id = commit_id.clone();
        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    jj::move_local_bookmark(&path, &bookmark_name, &commit_id)
                        .map_err(|error| format!("移动 bookmark 失败：{error}"))?;
                    jj::load_history(&path, &revset)
                        .map_err(|error| format!("读取提交历史失败：{error}"))
                })
                .await;

            let _ = page.update_in(cx, |page, window, cx| {
                page.operation = None;
                page.commit_state.update(cx, |state, cx| {
                    state.delegate_mut().set_disabled(false);
                    cx.notify();
                });
                if page.history_request_id != request_id {
                    return;
                }

                match result {
                    Ok(history) => {
                        page.commit_state.update(cx, |state, cx| {
                            state.delegate_mut().set_history_result(Ok(history));
                            cx.notify();
                        });
                        window.push_notification(
                            (
                                NotificationType::Success,
                                format!(
                                    "已将 bookmark {} 移动到提交 {:.8}",
                                    notification_bookmark, notification_commit_id
                                ),
                            ),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification((NotificationType::Error, error), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// 拉取当前远程：把远程上的 bookmark 拉下来，再重读一遍历史。
    fn fetch_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_remote_command(RemoteCommand::Fetch, window, cx);
    }

    /// 推送所有本地 bookmark 到当前远程。
    ///
    /// 这一步会动远程仓库，所以先弹一个确认框；真正执行的是 [`Self::push_remote_now`]。
    fn open_push_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation.is_some() || window.has_active_dialog(cx) {
            return;
        }

        if self.project_select.selected_path().is_none() {
            window.push_notification(
                (NotificationType::Error, "当前没有选中的整合包".to_owned()),
                cx,
            );
            return;
        }
        let Some(remote) = self.selected_remote.clone() else {
            window.push_notification(
                (
                    NotificationType::Error,
                    "请先选择要推送的远程仓库".to_owned(),
                ),
                cx,
            );
            return;
        };
        // 弹窗里点名是哪个整合包，免得用户在几个项目之间点错了还看不出来。
        let project = self
            .project_select
            .selected_project()
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "当前整合包".to_owned());

        let page = cx.entity().downgrade();
        let confirm_remote = remote.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let page = page.clone();
            let remote = confirm_remote.clone();
            dialog
                .title("确认推送")
                .overlay_closable(true)
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("push-cancel")
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("push-confirm").label("推送").on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ =
                                    page.update(cx, |page, cx| page.push_remote_now(window, cx));
                            },
                        )),
                )
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(Label::new(format!(
                            "将把「{project}」的所有本地 bookmark 推送到远程 {remote}。"
                        )))
                        .child(
                            Label::new(format!(
                                "会把「{project}」的所有本地 bookmark 推到远程 {remote}：远程上不存在的 \
                                 会被新建，已存在的会被更新。推送前会先跟远程对账，远程那边已经动过就拒绝，\
                                 不会盖掉别人的提交。"
                            ))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground),
                        ),
                )
        });
    }

    /// 确认推送后真正执行，和拉取共用一条流程。
    fn push_remote_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_remote_command(RemoteCommand::Push, window, cx);
    }

    /// 拉取/推送共用的一条流程：锁住页面、后台跑 jj、回来后重读历史。
    ///
    /// 两个操作都会改动远程上的 bookmark，历史列表里的远程标签要重读才是最新的。
    fn run_remote_command(
        &mut self,
        command: RemoteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.operation.is_some() {
            return;
        }

        let Some(path) = self.project_select.selected_path().cloned() else {
            window.push_notification(
                (NotificationType::Error, "当前没有选中的整合包".to_owned()),
                cx,
            );
            return;
        };
        let Some(remote) = self.selected_remote.clone() else {
            window.push_notification((NotificationType::Error, "请先选择远程仓库".to_owned()), cx);
            return;
        };

        self.operation = Some(VcsOperation::Remote(command));
        // 还在飞的那次「读历史」已经过期，别让它把新状态覆盖回去。
        self.history_request_id = self.history_request_id.wrapping_add(1);
        let request_id = self.history_request_id;
        self.commit_state.update(cx, |state, cx| {
            state.delegate_mut().set_disabled(true);
            cx.notify();
        });

        let revset = self.revset.clone();
        let page = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            // 显式写出返回类型：成功分支带摘要、失败分支带错误文案，靠推断定不下来。
            let result: Result<(jj::CommitHistory, String), String> = cx
                .background_executor()
                .spawn(async move {
                    let summary = match command {
                        RemoteCommand::Fetch => jj::fetch_remote(&path, &remote),
                        RemoteCommand::Push => jj::push_remote(&path, &remote),
                    }
                    .map_err(|error| format!("{}失败：{error}", command.label()))?;
                    let history = jj::load_history(&path, &revset)
                        .map_err(|error| format!("读取提交历史失败：{error}"))?;

                    Ok((history, summary))
                })
                .await;

            let _ = page.update_in(cx, |page, window, cx| {
                page.operation = None;
                page.commit_state.update(cx, |state, cx| {
                    state.delegate_mut().set_disabled(false);
                    cx.notify();
                });
                if page.history_request_id != request_id {
                    return;
                }

                match result {
                    Ok((history, summary)) => {
                        page.commit_state.update(cx, |state, cx| {
                            state.delegate_mut().set_history_result(Ok(history));
                            cx.notify();
                        });
                        // 摘要自己会说明推了什么（也可能是什么都没推），直接当通知正文。
                        window.push_notification((NotificationType::Success, summary), cx);
                    }
                    // 拉取/推送失败（比如没配凭据）时历史本身还是好的，
                    // 列表留着不动，只把失败原因说出来。
                    Err(error) => {
                        window.push_notification((NotificationType::Error, error), cx);
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for VcsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_selection(window, cx);

        div()
            .v_flex()
            .size_full()
            .gap_3()
            .pb_5()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(Label::new("VCS"))
                    .child(
                        div().flex_1().min_w_0().child(
                            Select::new(self.project_select.state())
                                .h(px(32.))
                                .placeholder("选择要查看的整合包"),
                        ),
                    )
                    .child(
                        Button::new("refresh-vcs")
                            .icon(gpui_kit::component::IconName::RotateCw)
                            .label("刷新")
                            .disabled(self.operation.is_some())
                            .on_click(cx.listener(|this, _, window, cx| {
                                // 先把下拉框和设置对齐，再按当前项目重读远程和历史。
                                this.project_select.sync(window, cx);
                                this.reload_remotes(window, cx);
                                this.reload_history(window, cx);
                            })),
                    ),
            )
            .child(
                // 远程操作工具栏：远程仓库下拉框 + 拉取 + 推送。
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(Label::new("远程仓库"))
                    .child(
                        div().flex_1().min_w_0().child(
                            Select::new(&self.remote_select)
                                .h(px(32.))
                                .disabled(self.operation.is_some())
                                .placeholder("选择远程仓库"),
                        ),
                    )
                    .child(
                        Button::new("fetch-remote")
                            .icon(gpui_kit::component::IconName::ArrowDown)
                            .label("拉取")
                            .disabled(self.operation.is_some() || self.selected_remote.is_none())
                            .loading(
                                self.operation == Some(VcsOperation::Remote(RemoteCommand::Fetch)),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fetch_remote(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("push-remote")
                            .icon(gpui_kit::component::IconName::ArrowUp)
                            .label("推送")
                            .disabled(self.operation.is_some() || self.selected_remote.is_none())
                            .loading(
                                self.operation == Some(VcsOperation::Remote(RemoteCommand::Push)),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_push_dialog(window, cx)
                                }),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(Label::new("显示提交"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.revset_input).w_full()),
                    ),
            )
            .child(
                List::new(&self.commit_state)
                    .flex_1()
                    .p_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> jj::CommitHistoryItem {
        jj::CommitHistoryItem {
            id: id.to_owned(),
            change_id: String::new(),
            description: String::new(),
            author: String::new(),
            timestamp: String::new(),
            parents: parents.iter().map(|parent| (*parent).to_owned()).collect(),
            bookmarks: Vec::new(),
        }
    }

    #[test]
    fn graph_rows_keep_linear_history_in_one_lane() {
        let rows = graph_rows(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);

        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.node_lane == 0));
        assert!(rows.iter().all(|row| row.lane_count == 1));
        assert_eq!(rows[0].parent_lanes, vec![0]);
        assert_eq!(rows[1].parent_lanes, vec![0]);
        assert!(rows[2].parent_lanes.is_empty());
        assert!(rows[2].is_root);
    }

    #[test]
    fn graph_rows_drop_parents_outside_the_visible_history() {
        let rows = graph_rows(&[commit("head", &["hidden"]), commit("other", &[])]);

        assert!(rows.iter().all(|row| row.lane_count == 1));
        assert!(rows.iter().all(|row| row.parent_lanes.is_empty()));
        assert!(rows.iter().all(|row| row.lanes_after.is_empty()));
        assert!(!rows[0].is_root);
        assert!(rows[0].has_hidden_parent);
        assert!(!rows[1].has_hidden_parent);
    }

    #[test]
    fn graph_rows_mark_hidden_parent_continuation_without_creating_a_lane() {
        let rows = graph_rows(&[commit("head", &["hidden"])]);

        assert!(rows[0].has_hidden_parent);
        assert!(rows[0].lanes_after.is_empty());
    }

    #[test]
    fn graph_rows_add_lanes_for_merge_parents() {
        let rows = graph_rows(&[
            commit("merge", &["left", "right"]),
            commit("left", &["base"]),
            commit("right", &["base"]),
            commit("base", &[]),
        ]);

        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[0].parent_lanes, vec![0, 1]);
        assert_eq!(rows[0].lane_count, 2);
        assert_eq!(rows[1].node_lane, 0);
        assert_eq!(rows[2].node_lane, 1);
        assert_eq!(rows[3].lane_count, 1);
    }

    #[test]
    fn graph_rows_keep_a_new_head_to_the_right_of_existing_lanes() {
        let rows = graph_rows(&[
            commit("working-copy", &["base"]),
            commit("branch", &["base"]),
            commit("base", &[]),
        ]);

        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[1].node_lane, 1);
        assert_eq!(rows[1].lanes_before, vec![true]);
        assert_eq!(rows[1].parent_lanes, vec![0]);
        assert_eq!(rows[2].node_lane, 0);
    }

    #[test]
    fn unique_prefix_lengths_match_jj_style_shortest_unique_prefixes() {
        let ids = vec![
            "abc123".to_owned(),
            "abd456".to_owned(),
            "abc999".to_owned(),
        ];

        assert_eq!(unique_prefix_lengths(&ids), vec![4, 3, 4]);
    }

    #[test]
    fn unique_prefix_lengths_handle_duplicate_ids() {
        let ids = vec!["abc".to_owned(), "abc".to_owned()];

        assert_eq!(unique_prefix_lengths(&ids), vec![3, 3]);
    }

    #[test]
    fn resolve_remote_keeps_the_previous_choice_and_falls_back_to_the_first() {
        let remotes = vec!["origin".to_owned(), "upstream".to_owned()];

        // 原来选中的还在，就还是它。
        assert_eq!(
            resolve_remote(&remotes, Some("upstream".to_owned())),
            Some("upstream".to_owned())
        );
        // 选中的远程没了（比如换了项目），退回第一个。
        assert_eq!(
            resolve_remote(&remotes, Some("gone".to_owned())),
            Some("origin".to_owned())
        );
        // 还没选过。
        assert_eq!(resolve_remote(&remotes, None), Some("origin".to_owned()));
        // 一个远程都没有。
        assert_eq!(resolve_remote(&[], Some("origin".to_owned())), None);
    }

    #[test]
    fn remote_index_matches_the_remote_name() {
        let remotes = vec!["origin".to_owned(), "upstream".to_owned()];

        assert_eq!(
            remote_index(&remotes, Some(&"upstream".to_owned())),
            Some(IndexPath::new(1))
        );
        assert_eq!(remote_index(&remotes, Some(&"gone".to_owned())), None);
        assert_eq!(remote_index(&remotes, None), None);
    }
}
