use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    str::FromStr,
    sync::{OnceLock, RwLock},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::api::dirs;

use super::setting_manager::{ModpackSettingManager, SettingManager};

pub struct Setting {
    globle: SettingManager,
    settings: HashMap<i32, ModpackSettingManager>,
}

impl Setting {
    pub fn instance() -> &'static RwLock<Setting> {
        static INSTANCE: OnceLock<RwLock<Setting>> = OnceLock::new();
        INSTANCE.get_or_init(|| RwLock::new(Self::create()))
    }

    pub fn get_globle(&self) -> &SettingManager {
        &self.globle
    }

    pub fn get_globle_mut(&mut self) -> &mut SettingManager {
        &mut self.globle
    }

    pub fn get(&self, id: i32) -> Option<&ModpackSettingManager> {
        self.settings.get(&id)
    }

    pub fn get_mut(&mut self, id: i32) -> Option<&mut ModpackSettingManager> {
        self.settings.get_mut(&id)
    }

    pub fn create() -> Self {
        let file_path = dirs::get_config_dirs().unwrap().join("id_setting.json");
        let mut file;
        if file_path.exists() {
            file = File::open(file_path).expect("Failed to open setting file");
        } else {
            file = File::create(&file_path).expect("Failed to create setting file");
            //先写入一个空的
            let setting: Vec<SettingPath> = Vec::new();
            let content = serde_json::to_string(&setting).expect("Failed to serialize setting");
            file.write_all(content.as_bytes())
                .expect("Failed write setting_id.json");
            file = File::open(&file_path).expect("Failed to create setting file");
        }

        let mut content = String::new();
        file.read_to_string(&mut content)
            .expect("Failed to read setting file");
        let mut setting: Vec<SettingPath> =
            serde_json::from_str(&content).expect("Failed to parse setting file");
        let mut settings = HashMap::new();

        for s in setting.iter_mut() {
            let id = s.id;
            if id == -1 {
                continue;
            }
            let modpack_path = PathBuf::from_str(s.modpack_path.as_str()).unwrap();
            settings.insert(
                id,
                ModpackSettingManager::read(id, modpack_path)
                    .expect("Failed to read setting manager"),
            );
        }

        Setting {
            globle: SettingManager::read().unwrap(),
            settings,
        }
    }

    pub fn change(&mut self, id: i32, name: String, value: Vec<String>) -> Result<()> {
        if id == -1 {
            self.globle.get_setting_mut().change(name, value)?;
            self.globle.save()?;
        } else if let Some(setting_manager) = self.settings.get_mut(&id) {
            setting_manager.get_setting_mut().change(name, value)?;
            setting_manager.save()?;
        } else {
            return Err(anyhow::anyhow!("Setting manager not found for id: {}", id));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct SettingPath {
    id: i32,
    modpack_path: String,
}
