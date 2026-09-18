//! CurseForge fingerprint 匹配接口的响应模型
//!
//! 顶层结构:
//! ```json
//! { "data": { "exactFingerprints": [...], ... } }
//! ```
//!
//! 所有可能为 null 的字段用 `DefaultOnNull` 处理,null 会自动变成类型的默认值
//! (String → "", i64 → 0, bool → false, Vec → [])。

use serde::Deserialize;
use serde_with::{serde_as, DefaultOnNull};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// 顶层
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct Envelope {
    pub data: FingerprintMatches,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintMatches {
    #[serde(default)]
    pub exact_fingerprints: Vec<i64>,
    #[serde(default)]
    pub exact_matches: Vec<FingerprintMatch>,
    #[serde(default)]
    pub installed_fingerprints: Vec<i64>,
    #[serde(default)]
    pub is_cache_built: bool,
    #[serde(default)]
    pub partial_match_fingerprints: HashMap<String, Vec<i64>>,
    #[serde(default)]
    pub partial_matches: Vec<FingerprintMatch>,
    #[serde(default)]
    pub unmatched_fingerprints: Vec<i64>,
}

// ---------------------------------------------------------------------------
// 匹配项
// ---------------------------------------------------------------------------

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintMatch {
    pub file: FileInfo,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub id: i64,
    #[serde(default)]
    pub latest_files: Vec<LatestFile>,
}

// ---------------------------------------------------------------------------
// 文件详情
// ---------------------------------------------------------------------------

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub alternate_file_id: i64,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub display_name: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub download_count: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub download_url: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub early_access_end_date: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub expose_as_alternative: bool,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_date: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_fingerprint: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_length: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_name: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_size_on_disk: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_status: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub game_id: i64,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub hashes: Vec<HashEntry>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub id: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub is_available: bool,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub is_early_access_content: bool,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub is_server_pack: bool,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub mod_id: i64,
    #[serde(default)]
    pub modules: Vec<Module>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub parent_project_file_id: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub release_type: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub server_pack_file_id: i64,
    #[serde(default)]
    pub sortable_game_versions: Vec<SortableGameVersion>,
    #[serde(rename = "sync_at", default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub sync_at: String,
}

// ---------------------------------------------------------------------------
// 子结构
// ---------------------------------------------------------------------------

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub mod_id: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub relation_type: i64,
}

/// 通用:哈希条目,FileInfo.hashes 与 LatestFile.hashes 共用
#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct HashEntry {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub algo: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub value: String,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct Module {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub fingerprint: i64,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub name: String,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortableGameVersion {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub game_version: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub game_version_name: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub game_version_padded: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub game_version_release_date: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub game_version_type_id: i64,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct LatestFile {
    #[serde(rename = "file_type", default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub file_type: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub filename: String,
    #[serde(default)]
    pub hashes: Vec<HashEntry>,
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

// ---------------------------------------------------------------------------
// 解析入口
// ---------------------------------------------------------------------------

use serde_json::Value;

/// 解析 CurseForge fingerprint 匹配响应。
///
/// 兼容两种情况:
///  - 顶层为 `{ "data": {...} }`(走 `Envelope`)
///  - 顶层直接就是内层对象(走 `FingerprintMatches`)
///
/// 出错时返回 [`crate::error::Error::Deserialize`]，其 `path` 形如
/// `$.data.exactMatches[0].latestFiles[0].hashes[0].value`
pub fn parse_fingerprint_response(
    body: &str,
) -> Result<FingerprintMatches, crate::error::Error> {
    let value: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Err(crate::error::Error::Deserialize {
                path: "<root>".to_string(),
                message: e.to_string(),
            });
        }
    };

    if value.get("data").is_some() {
        let mut de = serde_json::Deserializer::from_str(body);
        match serde_path_to_error::deserialize::<_, Envelope>(&mut de) {
            Ok(env) => Ok(env.data),
            Err(e) => Err(crate::error::Error::Deserialize {
                path: e.path().to_string(),
                message: e.into_inner().to_string(),
            }),
        }
    } else {
        let mut de = serde_json::Deserializer::from_str(body);
        match serde_path_to_error::deserialize::<_, FingerprintMatches>(&mut de) {
            Ok(v) => Ok(v),
            Err(e) => Err(crate::error::Error::Deserialize {
                path: e.path().to_string(),
                message: e.into_inner().to_string(),
            }),
        }
    }
}