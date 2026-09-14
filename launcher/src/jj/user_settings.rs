use anyhow::{Context, anyhow};
use jj_lib::config::{ConfigLayer, ConfigSource, StackedConfig};
use jj_lib::settings::UserSettings;
use std::path::Path;

pub fn create_user_settings(workspace_root: Option<&Path>) -> anyhow::Result<UserSettings> {
    // 1. 创建带内置默认值的 StackedConfig
    let mut config = StackedConfig::with_defaults();

    // 2. 添加用户配置层（内存中设置）
    let mut user_layer = ConfigLayer::empty(ConfigSource::User);
    user_layer.set_value("user.name", "Your Name")?;
    user_layer.set_value("user.email", "you@example.com")?;
    config.add_layer(user_layer);

    // 3. 可选：加载实际的 ~/.config/jj/config.toml 文件
    if let Some(config_dir) = dirs::config_dir() {
        let jj_config = config_dir.join("jj").join("config.toml");
        if jj_config.exists() {
            let _ = config.load_file(ConfigSource::User, &jj_config);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let jj_config = home.join(".config").join("jj").join("config.toml");
        if jj_config.exists() {
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
