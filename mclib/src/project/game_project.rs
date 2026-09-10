use crate::project::game_project_error::GameProjectError;
use serde_json::{Value, from_str};
use std::path::PathBuf;

pub struct GameProject {
    pub name: String,
    pub version: String,
    pub game_version: String,
    pub loader: ModLoader,
    /// 原版游戏则和游戏版本一致
    pub loader_version: String,
}

pub enum ModLoader {
    None,
    Forge,
    Neoforge,
    Fabric,
}

/// 获取文件夹的游戏版本信息
pub fn get_game_project(path_buf: PathBuf) -> Result<GameProject, GameProjectError> {
    let name = path_buf
        .components()
        .last()
        .ok_or(GameProjectError::UnknownPath)?
        .as_os_str()
        .to_str()
        .ok_or(GameProjectError::UnknownPath)?;

    let json_file = path_buf.join(format!("{}.json", name));

    match std::fs::metadata(&json_file) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(GameProjectError::NotFindSettingFile(json_file));
            }
        }
        Err(e) => return Err(GameProjectError::Io(e)),
    }

    let contents = std::fs::read_to_string(json_file).or_else(|e| Err(GameProjectError::Io(e)))?;
    let data: Value = from_str(&contents).or_else(|e| Err(GameProjectError::Json(e)))?;

    Err(GameProjectError::UnknownPath)
}
