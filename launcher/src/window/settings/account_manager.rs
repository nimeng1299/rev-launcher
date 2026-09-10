use crate::data::account_data::AccountData;
use crate::data::settings::AppSettings;
use gpui_kit::base::{IndexPath, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::marker::{Marker, MarkerContent, MarkerIcon, MarkerLoadingStyle};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable, Icon, IconName, StyledExt, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, div,
};
use mclib::account::{Account, AccountType};

struct AccountListDelegate {
    selected_index: Option<IndexPath>,
}

impl ListDelegate for AccountListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, cx: &App) -> usize {
        cx.global::<AccountData>().len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let account_data = cx.global::<AccountData>();
        account_data.get(ix.row).map(|item| {
            ListItem::new(ix)
                .child(
                    h_flex()
                        .gap_3p5()
                        .child(Label::new(item.name.clone()))
                        .child(Tag::secondary().outline().child(match item.account_type {
                            AccountType::Online => "正版账号",
                            AccountType::Offline => "离线账号",
                            AccountType::Other => "第三方账号",
                        }))
                        .when(Some(ix.row) == account_data.default, |div| {
                            div.child(Tag::secondary().outline().child("默认账号"))
                        }),
                )
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
pub struct AccountManager {
    state: Option<Entity<ListState<AccountListDelegate>>>,
}

impl Render for AccountManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self
            .state
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    ListState::new(
                        AccountListDelegate {
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
                    .child(Label::new("当前添加的账号"))
                    .child(
                        ButtonGroup::new("btn-group")
                            .child(Button::new("btn1").label("添加").on_click({
                                // 把账号列表交给对话框，登录成功后由它刷新列表
                                let account_list = state.clone();
                                move |_, window, cx| {
                                    let account_list = account_list.clone();
                                    let page =
                                        cx.new(|cx| AddAccount::new(account_list, window, cx));

                                    window.open_dialog(cx, move |dialog, window, _cx| {
                                        let size = window.bounds().size;

                                        dialog
                                            .title("添加账号")
                                            .overlay(true)
                                            .overlay_closable(true)
                                            .w(size.width * 0.8)
                                            .h(size.height * 0.8)
                                            .child(page.clone())
                                    });
                                }
                            }))
                            .child(Button::new("btn2").label("删除").on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    let selected_row =
                                        state.read(cx).selected_index().map(|ix| ix.row);
                                    if let Some(row) = selected_row {
                                        let account_data = cx.global_mut::<AccountData>();
                                        let remove_account = account_data.remove(row);
                                        if let Some(id) = account_data.default {
                                            if id == row {
                                                account_data.default = None;
                                            } else if id > row {
                                                account_data.default = Some(id - 1);
                                            }
                                        }
                                        let _ = cx.global::<AccountData>().save();
                                        state.update(cx, |_, cx| cx.notify());
                                        window.push_notification(
                                            (
                                                NotificationType::Success,
                                                format!("成功删除账号: {}", remove_account.name),
                                            ),
                                            cx,
                                        );
                                    }
                                }
                            }))
                            .child(Button::new("btn3").label("设为默认").on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    let selected_row =
                                        state.read(cx).selected_index().map(|ix| ix.row);
                                    if let Some(row) = selected_row {
                                        cx.global_mut::<AccountData>().default = Some(row);
                                        let _ = cx.global::<AccountData>().save();
                                        state.update(cx, |_, cx| cx.notify());
                                        window.push_notification(
                                            (
                                                NotificationType::Success,
                                                format!(
                                                    "当前默认账号为: {}",
                                                    cx.global_mut::<AccountData>()[row].name
                                                ),
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

/// 登录流程的状态，用于驱动按钮的加载态与失败提示。
#[derive(Default, Clone, PartialEq, Eq)]
enum LoginState {
    /// 空闲（尚未登录，或登录失败后可以重试）
    #[default]
    Idle,
    /// 登录请求进行中
    Loading,
    /// 登录成功，携带账号名称
    Success(String),
    /// 登录失败，携带错误信息
    Failed(String),
}

impl LoginState {
    fn is_loading(&self) -> bool {
        matches!(self, LoginState::Loading)
    }
}

/// 账号名称的本地校验，避免把明显无效的输入丢进后台任务。
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("账号名称不能为空".to_string());
    }
    if name.chars().count() > 16 {
        return Err("账号名称长度不能超过 16 个字符".to_string());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("账号名称只能包含字母、数字和下划线".to_string());
    }
    Ok(())
}

/// 执行登录的地方，运行在后台线程上（联网、耗时计算都放在这里）。
fn login_blocking(account_type: AccountType, name: &str) -> anyhow::Result<Account> {
    match account_type {
        AccountType::Offline => Ok(Account::new_offline(name)),
        AccountType::Online => anyhow::bail!("正版账号登录尚未实现"),
        AccountType::Other => anyhow::bail!("第三方账号登录尚未实现"),
    }
}

struct AddAccount {
    selected_index: Entity<usize>,
    account_type: Entity<AccountType>,

    name_input: Entity<InputState>,

    /// 账号列表实体，登录成功后用它刷新列表
    account_list: Entity<ListState<AccountListDelegate>>,
    /// 当前登录状态
    login_state: LoginState,
    /// 对话框内容的焦点句柄（见 `set_step_and_focus`）
    focus_handle: FocusHandle,
}

impl AddAccount {
    fn new(
        account_list: Entity<ListState<AccountListDelegate>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            selected_index: cx.new(|_cx| 0),
            account_type: cx.new(|_cx| AccountType::Online),
            name_input: cx.new(|cx| InputState::new(window, cx).placeholder("用户名")),
            account_list,
            login_state: LoginState::default(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// 切换向导步骤。
    ///
    /// `selected_index` 只是一个普通实体，通知它不会让当前视图重绘，
    /// 所以这里同时通知 `AddAccount` 自己。
    fn set_step(&mut self, step: usize, cx: &mut Context<Self>) {
        self.selected_index.update(cx, |state, cx| {
            *state = step;
            cx.notify();
        });
        cx.notify();
    }

    /// 切换步骤，并把焦点收回到对话框内容上。
    ///
    /// 标题栏右上角那个关闭按钮 (`DialogClose`) 只是派发一个 `Cancel` action，
    /// 而 gpui 的 action 只沿**焦点链**派发。一旦焦点落在已经卸载的控件上
    /// （比如切到「登录中」时输入框被移除），`Window::focused` 仍然会返回那个
    /// 已经不存在的句柄，gpui 便退回从根节点派发，对话框收不到 `Cancel`，
    /// 关闭按钮就点不动了（点遮罩还能关，因为那条路径直接调 `window.close_dialog`）。
    /// 所以每次切步骤都把焦点放到一直存在的对话框内容根节点上。
    fn set_step_and_focus(&mut self, step: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_step(step, cx);
        self.focus_handle.focus(window, cx);
    }

    fn set_login_state(&mut self, state: LoginState, cx: &mut Context<Self>) {
        self.login_state = state;
        cx.notify();
    }

    /// 异步登录。
    ///
    /// 主线程只做本地校验，然后把登录任务丢到后台线程，
    /// 完成后再回到主线程更新界面、写入全局账号数据并持久化。
    fn login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 防止重复点击
        if self.login_state.is_loading() {
            return;
        }

        let name = self.name_input.read(cx).value().trim().to_string();
        let account_type = *self.account_type.read(cx);

        if let Err(err) = validate_name(&name) {
            self.set_login_state(LoginState::Failed(err), cx);
            self.set_step_and_focus(3, window, cx);
            return;
        }

        // 已经添加过的账号不需要重复登录
        if cx
            .global::<AccountData>()
            .iter()
            .any(|account| account.name == name && account.account_type == account_type)
        {
            self.set_login_state(LoginState::Failed(format!("账号 {name} 已存在")), cx);
            self.set_step_and_focus(3, window, cx);
            return;
        }

        self.set_login_state(LoginState::Loading, cx);

        let account_list = self.account_list.clone();
        let login_name = name.clone();

        // `spawn_in` 给出 WeakEntity<Self>，避免任务长期持有对话框
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { login_blocking(account_type, &login_name) })
                .await;

            let _ = cx.update(|window, cx| match result {
                Ok(account) => {
                    // 写入全局账号数据并落盘
                    let row = {
                        let account_data = cx.global_mut::<AccountData>();
                        account_data.push(account);
                        let _ = account_data.save();
                        account_data.len() - 1
                    };

                    // 刷新账号列表，并选中刚登录的账号
                    account_list.update(cx, |list, cx| {
                        list.set_selected_index(Some(IndexPath::new(row)), window, cx);
                    });

                    let _ = this.update(cx, |this, cx| {
                        this.set_login_state(LoginState::Success(name.clone()), cx);
                        // 这里没有 window 可用；焦点在第 2 步切换时已经收回内容根节点，
                        // 而该节点一直存在，所以关闭按钮照常可用。
                        this.set_step(3, cx);
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    let _ = this.update(cx, |this, cx| {
                        this.set_login_state(LoginState::Failed(message), cx);
                        this.set_step(3, cx);
                    });
                }
            });
        })
        .detach();
    }
}

impl Render for AddAccount {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let index = *self.selected_index.read(cx);
        let account_type = self.account_type.read(cx);
        let loading = self.login_state.is_loading();

        let mut page = v_flex()
            .size_full()
            .items_center()
            .gap_5()
            // 让对话框内容一直有一个可聚焦的节点，用来把焦点留在对话框里
            .track_focus(&self.focus_handle);

        // 选择账号类型
        page = if index == 0 {
            page.child(
                Marker::new()
                    .loading(true)
                    .with_loading_style(MarkerLoadingStyle::Spinner)
                    .content(MarkerContent::new().text("选择账号类型")),
            )
            .child(
                ButtonGroup::new("0-btn-group")
                    .child(Button::new("0-btn1").label("添加正版账号"))
                    .child(
                        Button::new("0-btn2")
                            .label("添加离线账号")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.account_type
                                    .update(cx, |state, _| *state = AccountType::Offline);
                                this.set_step_and_focus(1, window, cx);
                            })),
                    )
                    .child(Button::new("0-btn3").label("添加第三方账号")),
            )
        } else {
            page.child(
                Marker::new()
                    .icon(MarkerIcon::new().child(Icon::new(IconName::CircleCheck)))
                    .content(MarkerContent::new().text("选择账号类型")),
            )
        };

        //账号输入
        page = if index == 1 {
            page = page.child(
                Marker::new()
                    .loading(true)
                    .with_loading_style(MarkerLoadingStyle::Spinner)
                    .content(MarkerContent::new().text("输入账号信息")),
            );
            match account_type {
                AccountType::Online => page,
                AccountType::Offline => page.child(
                    v_form()
                        .child(
                            field()
                                .label("账号名称")
                                .child(Input::new(&self.name_input)),
                        )
                        .child(
                            field().label_indent(false).child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("1-1-back")
                                            .label("返回")
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_step_and_focus(0, window, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("1-2-login")
                                            .primary()
                                            // 登录中时按钮进入加载态并停止响应点击
                                            .loading(loading)
                                            .label(if loading { "登录中…" } else { "登录" })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_step_and_focus(2, window, cx);
                                                this.login(window, cx);
                                            })),
                                    ),
                            ),
                        ),
                ),
                AccountType::Other => page,
            }
        } else if index < 1 {
            page.child(
                Marker::new()
                    .icon(MarkerIcon::new().child(Icon::new(IconName::Info)))
                    .content(MarkerContent::new().text("输入账号信息")),
            )
        } else {
            page.child(
                Marker::new()
                    .icon(MarkerIcon::new().child(Icon::new(IconName::CircleCheck)))
                    .content(MarkerContent::new().text("输入账号信息")),
            )
        };

        // 登录中
        page = if index == 2 {
            page.child(
                Marker::new()
                    .loading(true)
                    .with_loading_style(MarkerLoadingStyle::Spinner)
                    .content(MarkerContent::new().text("登录中")),
            )
        } else if index < 2 {
            page.child(
                Marker::new()
                    .icon(MarkerIcon::new().child(Icon::new(IconName::Info)))
                    .content(MarkerContent::new().text("等待登录")),
            )
        } else {
            page.child(
                Marker::new()
                    .icon(MarkerIcon::new().child(Icon::new(IconName::CircleCheck)))
                    .content(MarkerContent::new().text("登录")),
            )
        };

        // 登录完成
        page = if index == 3 {
            if let LoginState::Success(name) = &self.login_state {
                page.child(
                    Marker::new()
                        .icon(MarkerIcon::new().child(Icon::new(IconName::CircleCheck)))
                        .content(MarkerContent::new().text(format!("登录成功，用户名:{}", name))),
                )
            } else if let LoginState::Failed(message) = &self.login_state {
                page.child(
                    Marker::new()
                        .icon(MarkerIcon::new().child(Icon::new(IconName::Close)))
                        .content(MarkerContent::new().text(format!("登录失败: {}", message))),
                )
            } else {
                page.child(
                    Marker::new()
                        .icon(MarkerIcon::new().child(Icon::new(IconName::Close)))
                        .content(MarkerContent::new().text("未知错误，请重试")),
                )
            }
        } else {
            page.child(
                Marker::new()
                    .icon(MarkerIcon::new().child(Icon::new(IconName::Info)))
                    .content(MarkerContent::new().text("结果")),
            )
        };

        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_legal_names() {
        assert!(validate_name("Notch").is_ok());
        assert!(validate_name("player_01").is_ok());
    }

    #[test]
    fn validate_name_rejects_illegal_names() {
        assert!(validate_name("").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("中文名字").is_err());
        assert!(validate_name(&"a".repeat(17)).is_err());
    }

    #[test]
    fn offline_login_builds_account() {
        let account = login_blocking(AccountType::Offline, "bot_xiao").expect("离线登录应当成功");
        assert_eq!(account.name, "bot_xiao");
        assert_eq!(account.account_type, AccountType::Offline);
        assert_eq!(account.uuid, "a3b83191-c9ac-3b20-9f61-782908edb07a");
    }

    #[test]
    fn online_login_is_not_implemented_yet() {
        assert!(login_blocking(AccountType::Online, "Notch").is_err());
        assert!(login_blocking(AccountType::Other, "Notch").is_err());
    }

    mod dialog {
        use super::*;
        use gpui_kit::base::actions::Cancel;
        use gpui_kit::component::Root;
        use gpui_kit::{Focusable, TestAppContext, WindowHandle};

        struct DialogHost;

        impl Render for DialogHost {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .children(Root::render_dialog_layer(window, cx))
            }
        }

        /// 在窗口里跑一段逻辑。
        ///
        /// 不能用 `WindowHandle::update`：那会把 `Root` 压在 update 栈上，
        /// 而 `open_dialog` / `has_active_dialog` 内部都要 `Root::update`/`Root::read`。
        fn with_window<R>(
            cx: &mut TestAppContext,
            window: &WindowHandle<Root>,
            f: impl FnOnce(&mut Window, &mut App) -> R,
        ) -> R {
            cx.update(|app| {
                app.update_window(**window, |_, window, cx| f(window, cx))
                    .unwrap()
            })
        }

        /// 回归测试：切换步骤会把输入框卸载，而标题栏关闭按钮是靠 Cancel action
        /// 沿焦点链派发的，所以此时必须把焦点留在对话框里，否则关闭按钮点不动。
        #[gpui_kit::test]
        fn close_button_still_works_after_step_change(cx: &mut TestAppContext) {
            cx.update(gpui_kit::init);
            cx.update(|cx| cx.set_global(AccountData::default()));

            let window = cx.add_window(|window, cx| {
                let view = cx.new(|_| DialogHost);
                Root::new(view, window, cx)
            });

            // 打开真实的「添加账号」对话框
            let page = with_window(cx, &window, |window, cx| {
                let list_state = cx.new(|cx| {
                    ListState::new(
                        AccountListDelegate {
                            selected_index: None,
                        },
                        window,
                        cx,
                    )
                });
                let page = cx.new(|cx| AddAccount::new(list_state, window, cx));
                let dialog_page = page.clone();
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog.title("添加账号").child(dialog_page.clone())
                });
                page
            });
            cx.run_until_parked();

            let input_focus = cx.update(|cx| page.read(cx).name_input.read(cx).focus_handle(cx));

            // 模拟：进入输入步骤 → 输入框拿到焦点 → 点「登录」切到「登录中」（输入框被卸载）
            with_window(cx, &window, |window, cx| {
                page.update(cx, |page, cx| page.set_step_and_focus(1, window, cx));
                input_focus.focus(window, cx);
                page.update(cx, |page, cx| page.set_step_and_focus(2, window, cx));
                // 标题栏关闭按钮做的事
                window.dispatch_action(Box::new(Cancel), cx);
            });
            cx.run_until_parked();

            assert!(
                !with_window(cx, &window, |window, cx| window.has_active_dialog(cx)),
                "输入框被卸载后，标题栏关闭按钮仍然要能关掉对话框"
            );
        }
    }
}
