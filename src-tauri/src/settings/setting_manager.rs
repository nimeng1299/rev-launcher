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

#[derive(Clone, setting_derive::Setting)]
pub struct Settings {
    java: JavaVersions,
}

impl Settings {}

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

#[derive(setting_derive::ModpackSetting)]
pub struct ModpackSetting {
    java: Option<JavaVersions>,
}

impl ModpackSetting {}
