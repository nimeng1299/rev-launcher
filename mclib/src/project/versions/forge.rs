use serde::{Deserialize, Serialize};
use tl::HTMLTag;
use crate::error::Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeVersions {
    /// Minecraft版本
    pub id: String,
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version {
    pub version: String,
    pub time: String,
    pub download_url: String,
}

impl ForgeVersions {
    /// 从 Forge 官网的版本列表页解析某个 Minecraft 版本的所有 Forge 版本。
    ///
    /// 页面地址为 `index_<id>.html`，其中 `<id>` 是 Minecraft 版本号（如 `1.20.1`）。
    /// 解析 `table.download-list` 里的 `td.download-version` / `td.download-time` /
    /// `td.download-files` 三组单元格，并按文档顺序对齐为每一行。
    pub fn get_versions(id: String) -> Result<Self, Error> {
        // url: https://files.minecraftforge.net/net/minecraftforge/forge/index_<id>.html
        let url =
            format!("https://files.minecraftforge.net/net/minecraftforge/forge/index_{id}.html");

        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .user_agent("RevLauncher/0.1")
            .build()
            .new_agent();

        let mut resp = agent.get(&url).call()?;

        if !resp.status().is_success() {
            return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
        }

        let body = resp.body_mut().read_to_string()?;

        let dom = tl::parse(&body, tl::ParserOptions::default())
            .map_err(|e| Error::DownloadFailed(format!("解析 Forge 版本页失败: {e}")))?;
        let parser = dom.parser();

        // 三组单元格在文档中顺序一致，按索引对齐成一行
        let version_cells: Vec<_> = dom
            .query_selector("td.download-version")
            .map(|it| it.collect())
            .unwrap_or_default();
        let time_cells: Vec<_> = dom
            .query_selector("td.download-time")
            .map(|it| it.collect())
            .unwrap_or_default();
        let file_cells: Vec<_> = dom
            .query_selector("td.download-files")
            .map(|it| it.collect())
            .unwrap_or_default();

        let mut versions = Vec::new();
        for (version_handle, (time_handle, files_handle)) in version_cells
            .into_iter()
            .zip(time_cells.into_iter().zip(file_cells))
        {
            let (Some(version_cell), Some(time_cell), Some(files_cell)) = (
                version_handle.get(parser).and_then(|n| n.as_tag()),
                time_handle.get(parser).and_then(|n| n.as_tag()),
                files_handle.get(parser).and_then(|n| n.as_tag()),
            ) else {
                continue;
            };

            let Some(version) = version_from_cell(version_cell, parser) else {
                continue;
            };

            // `title` 属性里存了完整时间（如 "2026-08-19 10:18:07"），
            // 没有的话退化为单元格文本（仅日期）
            let time = time_cell
                .attributes()
                .get("title")
                .flatten()
                .map(|b| b.as_utf8_str().into_owned())
                .unwrap_or_else(|| time_cell.inner_text(parser).trim().to_string());

            let download_url = download_url_from_cell(files_cell, parser, &id, &version)
                .unwrap_or_else(|| forge_maven_url(&id, &version));

            versions.push(Version {
                version,
                time,
                download_url,
            });
        }

        Ok(Self { id, versions })
    }
}

/// 从 `td.download-version` 单元格里提取 Forge 版本号。
/// 首行会混有 promo 图标文本，取第一段数字样式的 token。
fn version_from_cell(tag: &HTMLTag, parser: &tl::Parser) -> Option<String> {
    let text = tag.inner_text(parser);
    text.split_whitespace()
        .map(str::trim)
        .find(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'))
        .map(str::to_string)
}

/// 从 `td.download-files` 单元格里挑选安装器（或 universal）的直链。
///
/// 链接结构为 `<li>` 中一对 `<a>`：主链接（通常指向 adfoc.us 广告跳转）+
/// `title="Direct Download"` 的直链。优先直链；没有则解码 `adfoc.us` 的
/// `?url=` 参数；再退化为 maven 直链。
fn download_url_from_cell(
    tag: &HTMLTag,
    parser: &tl::Parser,
    mc_id: &str,
    version: &str,
) -> Option<String> {
    let mut direct_installer = None;
    let mut installer = None;
    let mut direct_fallback = None;
    let mut fallback = None;

    for node in tag.children().all(parser) {
        let Some(a) = node.as_tag() else { continue };
        if a.name().as_bytes() != b"a" {
            continue;
        }
        let Some(href) = a.attributes().get("href").flatten() else {
            continue;
        };
        let url = normalize_url(&href.as_utf8_str());
        if !url.starts_with("http") {
            continue;
        }

        let is_direct = a
            .attributes()
            .get("title")
            .flatten()
            .is_some_and(|t| t.as_utf8_str() == "Direct Download");
        let is_installer = url.contains("-installer.")
            || url.contains("-universal.")
            || url.contains("installer-win");

        if is_direct && is_installer {
            direct_installer = Some(url);
            break;
        } else if is_direct && direct_fallback.is_none() {
            direct_fallback = Some(url);
        } else if is_installer && installer.is_none() {
            installer = Some(url);
        } else if !url.contains("changelog") && fallback.is_none() {
            fallback = Some(url);
        }
    }

    direct_installer
        .or(installer)
        .or(direct_fallback)
        .or(fallback)
        .or_else(|| Some(forge_maven_url(mc_id, version)))
}

/// adfoc.us 广告跳转链接里用 `url=` 参数携带真实地址，解码出来；
/// 普通链接原样返回。
fn normalize_url(url: &str) -> String {
    if url.contains("adfoc.us") {
        if let Some(pos) = url.find("url=") {
            let target = &url[pos + 4..];
            let end = target.find('&').unwrap_or(target.len());
            return target[..end].to_string();
        }
    }
    url.to_string()
}

/// 按 maven 命名规则拼一个 installer 直链，作为页面解析失败时的兜底。
fn forge_maven_url(mc_id: &str, version: &str) -> String {
    let full = format!("{mc_id}-{version}");
    format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_versions() {
        for id in ["1.20.1", "1.7.10"] {
            let versions = ForgeVersions::get_versions(id.to_string()).unwrap();
            assert_eq!(versions.id, id);
            assert!(!versions.versions.is_empty());
            assert!(versions.versions[0].download_url.contains("installer.jar"));
        }
    }

    #[test]
    fn test_26_1() {
        let id = "26.1";
        let versions = ForgeVersions::get_versions(id.to_string()).unwrap();
        assert_eq!(versions.id, id);
        assert!(!versions.versions.is_empty());
        let early = versions.versions.last().unwrap();
        assert_eq!(early.version, "62.0.0".to_string());
        assert_eq!(early.download_url, "https://maven.minecraftforge.net/net/minecraftforge/forge/26.1-62.0.0/forge-26.1-62.0.0-installer.jar".to_string());
    }
}
