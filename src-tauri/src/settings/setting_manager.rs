use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::PathBuf,
};

use super::{java_versions::JavaVersions, setting_trait::SettingTrait};

pub struct SettingManager {
    //-1 is global
    id: i32,
    modpack_path: PathBuf,
    setting: Settings,
}

impl SettingManager {
    pub fn read(id: i32, modpack_path: PathBuf) -> Result<Self> {
        let filepath;
        if id != -1 {
            filepath = modpack_path.join("rev").join("settings.json");
        } else {
            filepath = modpack_path.join("setting.json");
        }

        let setting: Settings;
        if filepath.exists() {
            let mut file = std::fs::File::open(filepath)?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            setting = Settings::read(serde_json::from_str(&content)?)?;
        } else {
            setting = Settings::create()?;
        }
        Ok(SettingManager {
            id,
            modpack_path,
            setting,
        })
    }

    pub fn create(id: i32, modpack_path: PathBuf) -> Result<Self> {
        let setting = Settings::create()?;
        Ok(SettingManager {
            id,
            modpack_path,
            setting,
        })
    }

    pub fn get_setting_file_path(&self) -> PathBuf {
        if self.id != -1 {
            self.modpack_path.join("rev").join("settings.json")
        } else {
            self.modpack_path.join("setting.json")
        }
    }

    pub fn save(&self) -> Result<()> {
        let settings_value = self.setting.save()?;
        let filepath = self.get_setting_file_path();
        if !filepath.exists() {
            std::fs::create_dir_all(filepath.parent().unwrap())?;
        }
        let mut file = std::fs::File::create(filepath)?;
        let content = serde_json::to_string_pretty(&settings_value)?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    pub fn get_setting(&self) -> &Settings {
        &self.setting
    }
}

pub struct Settings {
    java: JavaVersions,
}

impl Settings {
    pub fn read(json: Value) -> Result<Self> {
        let java = JavaVersions::read(json.get("java").cloned())?;
        Ok(Settings { java })
    }

    pub fn create() -> Result<Self> {
        let java = JavaVersions::read(None)?;
        Ok(Settings { java })
    }

    pub fn save(&self) -> Result<Value> {
        let java_value = self.java.write()?;
        let settings_value = serde_json::json!({
            "java": java_value,
        });
        Ok(settings_value)
    }

    pub fn get(&self, item_name: String) -> Result<Value> {
        match item_name.as_str() {
            "java" => Ok(serde_json::to_value(&self.java)?),
            _ => Err(anyhow::anyhow!("Item not found")),
        }
    }
}
