//! 「jj - Jujutsu」设置分组。
//!
//! 这里的输入框直接读写 jj 的用户配置文件（`<config>/jj/config.toml`），和 `jj`
//! 命令行共用同一份配置，所以改完对命令行立即生效，不需要另外同步。

use crate::jj::user_config;
use crate::jj::user_settings::{DEFAULT_USER_EMAIL, DEFAULT_USER_NAME};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem};
use gpui_kit::{App, SharedString};

/// 一个 `[user]` 下的字符串项：渲染成输入框，编辑后立刻写回 jj 用户配置。
///
/// `fallback` 是配置里没写这一项时启动器内部实际用的值。显示兜底值而不是留空，
/// 是为了让界面上的内容就是真正生效的身份；`InputState::set_value` 不会发出
/// `Change` 事件，所以这份兜底值不会被误写进用户的配置文件。
fn user_item(
    key: &'static str,
    fallback: &'static str,
    title: &'static str,
    description: &'static str,
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::input(
            move |_cx: &App| {
                SharedString::from(
                    user_config::user_value(key).unwrap_or_else(|| fallback.to_owned()),
                )
            },
            move |value: SharedString, _cx: &mut App| {
                // 只有用户真的编辑过才会走到这里。写失败（比如现有配置不是合法
                // TOML，或目录没有写权限）时这次修改被丢掉，下一帧输入框会退回
                // 文件里的值；这里拿不到 window，没法弹通知，就沿用项目里
                // `let _ = ...save()` 的做法。
                let _ = user_config::set_user_value(key, value.as_ref());
            },
        ),
    )
    .description(description)
}

/// 构建「jj - Jujutsu」分组。
pub fn jj_settings_group() -> SettingGroup {
    let group = SettingGroup::new().title("jj - Jujutsu");
    // 把实际改动的文件写在分组描述里，方便用户直接找到它。
    let group = match user_config::user_config_path() {
        Some(path) => group.description(format!("以下两项保存在 {}", path.display())),
        None => group,
    };

    group
        .item(user_item(
            "name",
            DEFAULT_USER_NAME,
            "用户名称",
            "提交记录里的作者名（jj 的 user.name），没配置过时显示的是启动器的占位值",
        ))
        .item(user_item(
            "email",
            DEFAULT_USER_EMAIL,
            "用户邮箱",
            "提交记录里的作者邮箱（jj 的 user.email），没配置过时显示的是启动器的占位值",
        ))
}
