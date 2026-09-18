//! Modrinth version 接口的响应模型
//!
//! 所有可能为 null 的字段用 `DefaultOnNull` 处理，null 自动变成默认值。

use std::collections::HashMap;

use serde::Deserialize;
use serde_with::{serde_as, DefaultOnNull};

// ---------------------------------------------------------------------------
// 顶层
// ---------------------------------------------------------------------------

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthInfo {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub author_id: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub changelog: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub changelog_url: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub date_published: String,

    #[serde(default)]
    pub dependencies: Vec<VersionDependency>,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub downloads: i64,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub featured: bool,

    #[serde(default)]
    pub files: Vec<VersionFile>,

    #[serde(default)]
    pub game_versions: Vec<String>,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub id: String,

    #[serde(default)]
    pub loaders: Vec<String>,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub name: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub project_id: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub requested_status: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub status: String,

    #[serde(rename = "sync_at", default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub sync_at: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub version_number: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub version_type: String,
}

// ---------------------------------------------------------------------------
// 子结构
// ---------------------------------------------------------------------------

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct VersionDependency {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub dependency_type: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_name: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub project_id: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub version_id: String,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct VersionFile {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_type: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub filename: String,

    /// 注意：这里是对象 {sha1, sha512}，不是数组
    #[serde(default)]
    pub hashes: VersionFileHashes,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub primary: bool,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub size: i64,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub url: String,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersionFileHashes {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub sha1: String,

    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub sha512: String,
}

/// 从 JSON 字符串解析 Modrinth 批量 version_files 接口的响应。
///
/// 响应是以请求时的 sha1 为键的映射：`{ "<sha1>": {version} }`。
///
/// 出错时返回 `(path, message)`，path 形如
/// `$.files[0].hashes.sha1` 或 `<root>`。
pub fn parse_version_files_response(
    body: &str,
) -> Result<HashMap<String, ModrinthInfo>, crate::error::Error> {
    let mut de = serde_json::Deserializer::from_str(body);
    match serde_path_to_error::deserialize::<_, HashMap<String, ModrinthInfo>>(&mut de) {
        Ok(v) => Ok(v),
        Err(e) => Err(crate::error::Error::Deserialize {
            path: e.path().to_string(),
            message: e.into_inner().to_string(),
        }),
    }
}

/// 从 JSON 字符串解析单个 Modrinth version（GET /v2/version/{version_id}）。
///
/// 出错时返回 `(path, message)`，path 形如
/// `$.files[0].hashes.sha1` 或 `<root>`。
pub fn parse_version_response(body: &str) -> Result<ModrinthInfo, crate::error::Error> {
    let mut de = serde_json::Deserializer::from_str(body);
    match serde_path_to_error::deserialize::<_, ModrinthInfo>(&mut de) {
        Ok(v) => Ok(v),
        Err(e) => Err(crate::error::Error::Deserialize {
            path: e.path().to_string(),
            message: e.into_inner().to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_version_files_response, parse_version_response};

    #[test]
    fn parse_version_test() {
        // 截取自 GET /v2/version/RncWhTxD 的真实响应，含 null 字段和未知字段
        let body = r#"{
            "game_versions": ["1.21", "1.21.1"],
            "loaders": ["fabric"],
            "environment": "client_only",
            "id": "RncWhTxD",
            "project_id": "AANobbMI",
            "author_id": "DzLrfrbK",
            "featured": true,
            "name": "Sodium 0.5.11",
            "version_number": "mc1.21-0.5.11",
            "changelog_url": null,
            "date_published": "2024-06-29T13:45:49.041594Z",
            "downloads": 9620717,
            "version_type": "release",
            "status": "listed",
            "requested_status": null,
            "files": [],
            "dependencies": []
        }"#;

        let info = parse_version_response(body).unwrap();
        assert_eq!(info.id, "RncWhTxD");
        assert_eq!(info.name, "Sodium 0.5.11");
        assert_eq!(info.project_id, "AANobbMI");
        assert_eq!(info.loaders, vec!["fabric"]);
    }

    #[test]
    fn parse_batch_response_test() {
        // 来自 POST /v2/version_files 的真实响应（截取），含 null 字段和未知字段
        let body = r#"{
            "d67e66ea4bb2409997b636dae4203d33764cdcc8": {
                "game_versions": ["1.21", "1.21.1"],
                "loaders": ["fabric"],
                "environment": "client_only",
                "id": "RncWhTxD",
                "project_id": "AANobbMI",
                "author_id": "DzLrfrbK",
                "featured": true,
                "name": "Sodium 0.5.11",
                "version_number": "mc1.21-0.5.11",
                "changelog_url": null,
                "date_published": "2024-06-29T13:45:49.041594Z",
                "downloads": 9620717,
                "version_type": "release",
                "status": "listed",
                "requested_status": null,
                "files": [],
                "dependencies": []
            }
        }"#;

        let map = parse_version_files_response(body).unwrap();
        assert_eq!(map.len(), 1);

        let info = &map["d67e66ea4bb2409997b636dae4203d33764cdcc8"];
        assert_eq!(info.name, "Sodium 0.5.11");
        assert_eq!(info.project_id, "AANobbMI");
        assert_eq!(info.loaders, vec!["fabric"]);
    }

    #[test]
    fn parse_empty_response_test() {
        let map = parse_version_files_response("{}").unwrap();
        assert!(map.is_empty());
    }
}