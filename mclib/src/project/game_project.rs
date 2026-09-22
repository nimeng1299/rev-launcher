use crate::project::project_json_file::json_get_patches;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_str};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GameProject {
    pub name: String,
    /// 整合包的版本
    pub version: String,
    pub path: PathBuf,
    /// 游戏的版本
    pub game_version: String,
    pub loader: ModLoader,
    /// 加载器的版本，原版游戏则和游戏版本一致
    pub loader_version: String,
}

impl GameProject {
    const PROJECT_FILE: &'static str = "project.json";

    pub fn load(path_buf: &PathBuf) -> Result<Self, crate::error::Error> {
        let filename = get_launcher_path(path_buf).join(Self::PROJECT_FILE);
        let context =
            std::fs::read_to_string(filename).or_else(|e| Err(crate::error::Error::Io(e)))?;
        let data: Self =
            serde_json::from_str(&context).or_else(|e| Err(crate::error::Error::Json(e)))?;
        Ok(data)
    }

    pub fn save(&self) -> Result<(), crate::error::Error> {
        let pick_path = get_launcher_path(&self.path);
        std::fs::create_dir_all(&pick_path).or_else(|e| Err(crate::error::Error::Io(e)))?;
        let filename = pick_path.join(Self::PROJECT_FILE);
        let data =
            serde_json::to_string_pretty(self).or_else(|e| Err(crate::error::Error::Json(e)))?;
        std::fs::write(filename, data).or_else(|e| Err(crate::error::Error::Io(e)))
    }

    ///  读取游戏目录下的<name>.json配置文件
    pub fn read_json(&self) -> Result<Value, crate::error::Error> {
        let name = self
            .path
            .components()
            .last()
            .ok_or(crate::error::Error::UnknownPath)?
            .as_os_str()
            .to_str()
            .ok_or(crate::error::Error::UnknownPath)?;
        let json_file = self.path.join(format!("{}.json", name));
        let contents =
            std::fs::read_to_string(json_file).or_else(|e| Err(crate::error::Error::Io(e)))?;
        from_str(&contents).or_else(|e| Err(crate::error::Error::Json(e)))
    }
}

/// 获取游戏目录下启动器文件的存放地址
pub fn get_launcher_path(path_buf: &PathBuf) -> PathBuf {
    path_buf.join(".rev_launcher")
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ModLoader {
    Minecraft,
    Forge,
    Neoforge,
    Fabric,
}

pub fn find_all_game_in_project_folder(path_buf: &PathBuf) -> Vec<GameProject> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path_buf) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.push(entry.path());
            }
        }
    }

    dirs.iter()
        .map(|path_buf| get_game_project(path_buf))
        .filter_map(Result::ok)
        .collect()
}

/// 获取文件夹的游戏版本信息
pub fn get_game_project(path_buf: &PathBuf) -> Result<GameProject, crate::error::Error> {
    // 1. 尝试从文件中读取
    if let Ok(game) = read_from_project_file(path_buf) {
        return Ok(game);
    }

    // 2. 尝试从json文件中读取
    read_from_json_file(path_buf)
}

/// 尝试从启动器的版本文件中读取
fn read_from_project_file(path_buf: &PathBuf) -> Result<GameProject, crate::error::Error> {
    GameProject::load(path_buf)
}

/// 尝试从目录下的同名json文件中读取
fn read_from_json_file(path_buf: &PathBuf) -> Result<GameProject, crate::error::Error> {
    let name = path_buf
        .components()
        .last()
        .ok_or(crate::error::Error::UnknownPath)?
        .as_os_str()
        .to_str()
        .ok_or(crate::error::Error::UnknownPath)?;

    let json_file = path_buf.join(format!("{}.json", name));

    match std::fs::metadata(&json_file) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(crate::error::Error::NotFindSettingFile(json_file));
            }
        }
        Err(e) => return Err(crate::error::Error::Io(e)),
    }

    let contents = std::fs::read_to_string(json_file).or_else(|e| Err(crate::error::Error::Io(e)))?;
    let data: Value = from_str(&contents).or_else(|e| Err(crate::error::Error::Json(e)))?;

    // 通过patches来判断版本，所有未判断出来的统一为原版
    // 原版清单没有 patches 数组，视为空列表
    let lib = match data.get("patches") {
        Some(_) => json_get_patches(&data)?,
        None => Vec::new(),
    };
    let mut game_version = None;
    let mut loader = ModLoader::Minecraft;
    let mut loader_version = None;
    for value in lib {
        // 声明接下来读到的版本是否为游戏本体版本
        let mut is_game_version = false;
        // 声明接下来读到的版本是否为加载器版本
        let mut is_loader_version = false;

        if let Some(v) = value.get("id")
            && let Some(v) = v.as_str()
        {
            if v.starts_with("neoforge") {
                loader = ModLoader::Neoforge;
                is_loader_version = true;
            } else if v.starts_with("forge") {
                loader = ModLoader::Forge;
                is_loader_version = true;
            } else if v.starts_with("fabric") {
                loader = ModLoader::Fabric;
                is_loader_version = true;
            } else if v.starts_with("game") {
                is_game_version = true;
            }
        }

        if let Some(v) = value.get("version")
            && let Some(v) = v.as_str()
        {
            if is_game_version {
                game_version = Some(v.to_string());
            } else if is_loader_version {
                loader_version = Some(v.to_string());
            }
        }
    }

    // 原版清单没有 patches，直接用 JSON 的 id 字段作为游戏版本
    let game_version = game_version.or_else(|| {
        data.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
    });

    if let Some(game_version) = game_version {
        if loader == ModLoader::Minecraft {
            let project = GameProject {
                name: name.to_string(),
                version: "".to_string(),
                path: path_buf.to_path_buf(),
                game_version: game_version.clone(),
                loader,
                loader_version: game_version,
            };
            let _ = project.save();
            return Ok(project);
        } else if let Some(loader_version) = loader_version {
            let project = GameProject {
                name: name.to_string(),
                version: "".to_string(),
                path: path_buf.to_path_buf(),
                game_version,
                loader,
                loader_version,
            };
            let _ = project.save();
            return Ok(project);
        }
    }

    Err(crate::error::Error::UnknownPath)
}
