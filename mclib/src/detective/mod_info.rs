use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModInfo{
    pub filename: String,
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