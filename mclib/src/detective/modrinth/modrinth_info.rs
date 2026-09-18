//! Modrinth version 接口的响应模型
//!
//! 所有可能为 null 的字段用 `DefaultOnNull` 处理，null 自动变成默认值。

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

/// 从 JSON 字符串解析 Modrinth version。
///
/// 出错时返回 `(path, message)`，path 形如
/// `$.files[0].hashes.sha1` 或 `<root>`。
pub fn parse_fingerprint_response(body: &str) -> Result<ModrinthInfo, crate::error::Error> {
    let mut de = serde_json::Deserializer::from_str(body);
    match serde_path_to_error::deserialize::<_, ModrinthInfo>(&mut de) {
        Ok(v) => Ok(v),
        Err(e) => Err(crate::error::Error::Deserialize {
            path: e.path().to_string(),
            message: e.into_inner().to_string(),
        }),
    }
}