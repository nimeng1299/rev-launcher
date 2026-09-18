use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModInfo{
    filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    curseforge: Option<CurseforgeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modrinth: Option<ModrinthInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CurseforgeInfo{
    project_id: i64,
    file_id: i64
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModrinthInfo{
    id: String
}