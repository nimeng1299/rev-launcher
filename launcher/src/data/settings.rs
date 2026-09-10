use anyhow::{Context, Result};
use gpui_kit::Global;
use mclib::java::java_version::JavaVersion;
use mclib::settings::GlobalSettings;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use std::env::current_dir;
use std::path::PathBuf;

const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, SmartDefault, Deserialize, Serialize)]
pub struct AppSettings {
    pub project_paths: Vec<PathBuf>,
    #[default(_code = "mclib::java::find_javas()")]
    pub java_versions: Vec<JavaVersion>,
    pub default_java_version: Option<usize>,
    pub global_settings: GlobalSettings,
}

impl Global for AppSettings {}

impl AppSettings {
    pub fn init() -> Self {
        Self::load().unwrap_or_else(|_| Self::default())
    }

    pub fn load() -> Result<Self> {
        let path = current_dir().context("Failed to read current directory")?;
        let filename = path.join(SETTINGS_FILE);
        let context = std::fs::read_to_string(filename).context("Failed to read settings file")?;
        let mut data: Self = serde_json::from_str(&context).context("Failed to parse settings")?;
        data.java_versions = data
            .java_versions
            .iter()
            .map(|version| JavaVersion::from_path(&version.path_buf))
            .filter_map(Result::ok)
            .collect();
        Ok(data)
    }

    pub fn save(&self) -> Result<()> {
        let path = current_dir().context("Failed to read current directory")?;
        let filename = path.join(SETTINGS_FILE);
        let data = serde_json::to_string_pretty(self).context("Failed to serialize settings")?;
        std::fs::write(filename, data).context("Failed to write settings")
    }
}

impl Drop for AppSettings {
    fn drop(&mut self) {
        let _ = self.save();
    }
}
