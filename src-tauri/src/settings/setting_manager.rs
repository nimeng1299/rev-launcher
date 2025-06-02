use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

use crate::api::dirs;

use super::{java_versions::JavaVersions, setting::Setting, setting_trait::SettingTrait};

pub struct SettingManager {
    //-1 is global
    setting: Settings,
}

impl SettingManager {
    pub fn read() -> Result<Self> {
        let filepath = dirs::get_config_dirs()?.join("setting.json");
        let setting: Settings;
        if filepath.exists() {
            let mut file = std::fs::File::open(filepath)?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            setting = Settings::read(serde_json::from_str(&content)?)?;
        } else {
            setting = Settings::create()?;
        }
        Ok(SettingManager { setting })
    }

    pub fn create() -> Result<Self> {
        let setting = Settings::create()?;
        Ok(SettingManager { setting })
    }

    pub fn get_setting_file_path(&self) -> PathBuf {
        dirs::get_config_dirs().unwrap()
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

    pub fn get_setting_mut(&mut self) -> &mut Settings {
        &mut self.setting
    }
}

#[derive(Clone)]
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

    pub fn change(&mut self, item_name: String, value: Vec<String>) -> Result<()> {
        match item_name.as_str() {
            "java" => {
                self.java.receive(value)?;
                Ok(())
            }
            _ => Err(anyhow::anyhow!("Item not found")),
        }
    }
}

pub struct ModpackSettingManager {
    id: i32,
    modpack_path: PathBuf,
    setting: ModpackSetting,
}

impl ModpackSettingManager {
    pub fn read(id: i32, modpack_path: PathBuf) -> Result<Self> {
        let filepath = modpack_path.join("rev").join("settings.json");
        let setting: ModpackSetting;
        if filepath.exists() {
            let mut file = std::fs::File::open(filepath)?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            setting = ModpackSetting::read(serde_json::from_str(&content)?)?;
        } else {
            setting = ModpackSetting::create()?;
        }
        Ok(ModpackSettingManager {
            id,
            modpack_path,
            setting,
        })
    }

    pub fn create(id: i32, modpack_path: PathBuf) -> Result<Self> {
        let setting = ModpackSetting::create()?;
        Ok(ModpackSettingManager {
            id,
            modpack_path,
            setting,
        })
    }

    pub fn get_setting_file_path(&self) -> PathBuf {
        self.modpack_path.join("rev").join("settings.json")
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

    pub fn get_setting(&self) -> &ModpackSetting {
        &self.setting
    }

    pub fn get_setting_mut(&mut self) -> &mut ModpackSetting {
        &mut self.setting
    }
}

pub struct ModpackSetting {
    java: Option<JavaVersions>,
}

impl ModpackSetting {
    pub fn read(json: Value) -> Result<Self> {
        let java = JavaVersions::read_modpack(json.get("java").cloned())?;
        Ok(ModpackSetting { java })
    }

    pub fn create() -> Result<Self> {
        Ok(ModpackSetting { java: None })
    }

    pub fn save(&self) -> Result<Value> {
        let mut json_data: Map<String, Value> = Map::new();

        Self::save_simple(&mut json_data, "java".to_string(), &self.java)?;

        let result: Value = Value::Object(json_data);
        Ok(result)
    }

    fn save_simple(
        map: &mut Map<String, Value>,
        key: String,
        value: &Option<impl SettingTrait>,
    ) -> Result<()> {
        if let Some(v) = value {
            map.insert(key, v.write()?);
        }
        Ok(())
    }

    pub fn get(&self, item_name: String, globle: &Settings) -> Result<Value> {
        match item_name.as_str() {
            "java" => self.get_simple(item_name, &self.java, globle),
            _ => Err(anyhow::anyhow!("Item not found")),
        }
    }

    fn get_simple(
        &self,
        item_name: String,
        item: &Option<impl SettingTrait>,
        globle: &Settings,
    ) -> Result<Value> {
        match item {
            Some(v) => Ok(v.write()?),
            None => globle.get(item_name),
        }
    }

    pub fn change(&mut self, item_name: String, value: Vec<String>) -> Result<()> {
        match item_name.as_str() {
            "java" => Self::change_simple(&mut self.java, value),
            _ => Err(anyhow::anyhow!("Item not found")),
        }
    }

    fn change_simple(item: &mut Option<impl SettingTrait>, value: Vec<String>) -> Result<()> {
        match item {
            Some(v) => v.receive(value),
            None => Err(anyhow::anyhow!("Item not found")),
        }
    }
}
