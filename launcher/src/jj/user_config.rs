//! jj 用户级 `config.toml` 的读写。
//!
//! 设置页里的「用户名称 / 用户邮箱」直接改这个文件，和 `jj` 命令行共用同一份用户配置，
//! 所以两个输入框里显示的就是实际生效的身份。
//!
//! 改写用 `toml_edit` 做，只动 `[user]` 下的目标键，文件里原有的注释、缩进和其它配置
//! 都会原样保留。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

/// jj 用户配置的候选位置。
///
/// 系统 config 目录（Windows 上是 `%APPDATA%`）是 jj 命令行实际使用的位置；
/// `~/.config` 只是兼容类 Unix 习惯的兜底，在 Linux/macOS 上它和前者本就是同一个路径，
/// 会被去重掉。
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("jj").join("config.toml"));
    }
    if let Some(home) = dirs::home_dir() {
        let fallback = home.join(".config").join("jj").join("config.toml");
        if !paths.contains(&fallback) {
            paths.push(fallback);
        }
    }

    paths
}

/// 用户 config.toml 的读写位置：已经有配置就改那一份，否则用首选位置新建。
pub fn user_config_path() -> Option<PathBuf> {
    let paths = candidate_paths();
    paths
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .or_else(|| paths.into_iter().next())
}

/// 读取 `[user]` 下的一个字符串项，文件不存在或没配过就返回 `None`。
pub fn user_value(key: &str) -> Option<String> {
    read_user_value(&user_config_path()?, key)
}

/// 把 `[user]` 下的 `key` 改成 `new_value` 并写回用户配置。
pub fn set_user_value(key: &str, new_value: &str) -> Result<()> {
    let path = user_config_path().context("找不到 jj 用户配置目录")?;
    write_user_value(&path, key, new_value)
}

/// 从指定文件读 `[user]` 下的一项。挨个 `?` 下来：文件不在、不是合法 TOML、
/// 没有 `[user]`、没有这一项、或者这一项不是字符串，都算「没配过」。
fn read_user_value(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc = text.parse::<DocumentMut>().ok()?;
    doc.get("user")?.get(key)?.as_str().map(ToOwned::to_owned)
}

/// 改写指定文件里的 `[user].key`，父目录不存在就一并建出来。
fn write_user_value(path: &Path, key: &str, new_value: &str) -> Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // 还没配过：从一个空文档开始，正好把 `[user]` 建出来。
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("无法读取 {}", path.display()));
        }
    };

    let text = apply_user_value(&text, key, new_value)
        .with_context(|| format!("无法改写 {}", path.display()))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }

    // 输入框每敲一个键就会调一次这里，先写临时文件再改名覆盖，避免写到一半失败
    // 把用户的 jj 配置截断（rename 会替换已存在的目标文件）。
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text).with_context(|| format!("无法写入 {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("无法保存 {}", path.display()))
}

/// `write_user_value` 的纯文本部分：把改写结果算出来，方便测试不碰真实配置文件。
fn apply_user_value(text: &str, key: &str, new_value: &str) -> Result<String> {
    let mut doc = text
        .parse::<DocumentMut>()
        .context("现有的配置不是合法的 TOML")?;

    // 被写成 `user = "xxx"` 这类非表值时整项删掉重建：就地覆盖会把原来那一行的
    // 空白留在新的表头上，渲染出 `[user ]` 这种别扭的写法。
    let table_like = doc
        .as_table()
        .get("user")
        .is_some_and(|item| item.is_table() || item.is_inline_table());
    if !table_like {
        doc.as_table_mut().remove("user");
    }

    // `user` 缺失时补一张正经的表，而不是让索引把中间层建成 inline table；
    // 已经是 inline table 就沿用，只改里面的键。
    let user = doc
        .as_table_mut()
        .entry("user")
        .or_insert_with(|| Item::Table(Table::new()));
    user[key] = value(new_value);

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_creates_user_table() {
        let text = apply_user_value("", "name", "Alice").unwrap();
        assert_eq!(text, "[user]\nname = \"Alice\"\n");
    }

    #[test]
    fn existing_config_keeps_other_keys_and_comments() {
        let original = "# 我的配置\n[user]\nname = \"Bob\" # 作者\n[ui]\ncolor = \"always\"\n";
        let text = apply_user_value(original, "email", "bob@example.com").unwrap();

        assert!(text.contains("# 我的配置"));
        assert!(text.contains("name = \"Bob\" # 作者"));
        assert!(text.contains("color = \"always\""));
        assert!(text.contains("email = \"bob@example.com\""));
    }

    #[test]
    fn inline_user_table_is_updated_in_place() {
        let text = apply_user_value("user = { name = \"Bob\" }\n", "name", "Alice").unwrap();
        assert!(text.contains("name = \"Alice\""));
        assert!(!text.contains("Bob"));
    }

    #[test]
    fn non_table_user_is_replaced() {
        let text = apply_user_value("user = \"oops\"\n", "name", "Alice").unwrap();
        assert_eq!(text, "[user]\nname = \"Alice\"\n");
    }

    #[test]
    fn invalid_config_is_rejected() {
        assert!(apply_user_value("this is not = = toml", "name", "Alice").is_err());
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // 父目录故意不先建：写入时要能自己建出来。
        let path = dir.path().join("jj").join("config.toml");

        assert_eq!(read_user_value(&path, "name"), None, "文件还不存在");

        write_user_value(&path, "name", "Alice").unwrap();
        write_user_value(&path, "email", "alice@example.com").unwrap();

        assert_eq!(read_user_value(&path, "name").as_deref(), Some("Alice"));
        assert_eq!(
            read_user_value(&path, "email").as_deref(),
            Some("alice@example.com")
        );
        // 再写一次会就地覆盖，不会留下两份。
        write_user_value(&path, "name", "Bob").unwrap();
        assert_eq!(read_user_value(&path, "name").as_deref(), Some("Bob"));
        assert_eq!(
            read_user_value(&path, "email").as_deref(),
            Some("alice@example.com")
        );
        // 原子写用的临时文件不该留在配置目录里。
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn read_ignores_missing_or_non_string_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        std::fs::write(&path, "[user]\nname = 42\n").unwrap();
        assert_eq!(read_user_value(&path, "name"), None, "不是字符串");
        assert_eq!(read_user_value(&path, "email"), None, "没配过");

        std::fs::write(&path, "this is not = = toml").unwrap();
        assert_eq!(read_user_value(&path, "name"), None, "配置文件坏了");
    }

    #[test]
    fn candidate_paths_are_deduplicated() {
        // 拿不到 config/home 目录的极端环境下允许为空，这里只检查不会重复。
        let paths = candidate_paths();
        let mut deduped = paths.clone();
        deduped.dedup();
        assert_eq!(paths.len(), deduped.len());
    }
}
