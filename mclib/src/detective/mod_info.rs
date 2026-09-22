use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModInfo{
    pub filename: String,
    /// 对应 jar 文件的 SHA-1，用于序列化时判断结果是否仍然有效
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<CurseforgeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modrinth: Option<ModrinthInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CurseforgeInfo{
    pub project_id: i64,
    pub file_id: i64
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModrinthInfo{
    pub id: String
}