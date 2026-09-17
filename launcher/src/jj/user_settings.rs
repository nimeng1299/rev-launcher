use anyhow::{Context, anyhow};
use jj_lib::config::{ConfigLayer, ConfigSource, StackedConfig};
use jj_lib::settings::UserSettings;
use std::path::Path;

use crate::jj::user_config;

/// 用户没有配置 `user.name` 时的占位值。
///
/// `UserSettings::from_config` 要求 `user.name` / `user.email` 必须存在，缺失会直接报错，
/// 所以哪怕用户还没配过 jj，也得给一份兜底，否则加载仓库历史会失败。
pub const DEFAULT_USER_NAME: &str = "Your Name";
/// 用户没有配置 `user.email` 时的占位值。
pub const DEFAULT_USER_EMAIL: &str = "you@example.com";

pub fn create_user_settings(workspace_root: Option<&Path>) -> anyhow::Result<UserSettings> {
    // 1. 创建带内置默认值的 StackedConfig
    let mut config = StackedConfig::with_defaults();

    // 2. 添加用户配置层（内存中设置）
    let mut user_layer = ConfigLayer::empty(ConfigSource::User);
    user_layer.set_value("user.name", DEFAULT_USER_NAME)?;
    user_layer.set_value("user.email", DEFAULT_USER_EMAIL)?;
    config.add_layer(user_layer);

    // 3. 加载实际的 ~/.config/jj/config.toml（Windows 上是 %APPDATA%\jj\config.toml）。
    //    从低优先级往高优先级叠：后面的层覆盖前面的，所以 config_dir 那份最后加载，
    //    和设置页读写的位置保持一致，界面显示的就是实际生效的值。
    for jj_config in user_config::candidate_paths().into_iter().rev() {
        if jj_config.is_file() {
            let _ = config.load_file(ConfigSource::User, &jj_config);
        }
    }

    // jj stores repository and workspace configuration outside the working
    // copy and references it through .jj/repo/config-id and
    // .jj/workspace-config-id. Load those layers so revset aliases such as
    // `trunk()` behave the same way as the jj command line.
    if let Some(workspace_root) = workspace_root {
        if let Some(config_dir) = dirs::config_dir() {
            let jj_config_dir = config_dir.join("jj");
            let repo_config_id = workspace_root.join(".jj").join("repo").join("config-id");
            if let Ok(config_id) = std::fs::read_to_string(repo_config_id) {
                let repo_config = jj_config_dir
                    .join("repos")
                    .join(config_id.trim())
                    .join("config.toml");
                if repo_config.exists() {
                    let _ = config.load_file(ConfigSource::Repo, repo_config);
                }
            }

            let workspace_config_id = workspace_root.join(".jj").join("workspace-config-id");
            if let Ok(config_id) = std::fs::read_to_string(workspace_config_id) {
                let workspace_config = jj_config_dir
                    .join("workspaces")
                    .join(config_id.trim())
                    .join("config.toml");
                if workspace_config.exists() {
                    let _ = config.load_file(ConfigSource::Workspace, workspace_config);
                }
            }
        }
    }

    // 4. 转换为 UserSettings
    UserSettings::from_config(config).context(anyhow!("Could not create user settings"))
}
