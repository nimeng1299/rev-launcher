use anyhow::Context;
use gpui_kit::Global;
use mclib::account::Account;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

const ACCOUNT_DATA_FILE: &str = "account.json";

#[derive(Default, Deserialize, Serialize)]
pub struct AccountData {
    pub data: Vec<Account>,
    pub default: Option<usize>,
}

impl Global for AccountData {}

impl AccountData {
    pub fn init() -> Self {
        Self::load().unwrap_or_else(|_| Self::default())
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = dirs::config_dir().context("Failed to read current directory")?;
        let filename = path.join("rev_launcher").join(ACCOUNT_DATA_FILE);
        let context = std::fs::read_to_string(filename).context("Failed to read settings file")?;
        let data: Self = serde_json::from_str(&context).context("Failed to parse settings")?;
        Ok(data)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = dirs::config_dir().context("Failed to read current directory")?;
        let pick_path = path.join("rev_launcher");
        std::fs::create_dir_all(&pick_path)
            .context("Failed to create settings directory")?;
        let filename = pick_path.join(ACCOUNT_DATA_FILE);
        let data = serde_json::to_string_pretty(self).context("Failed to serialize settings")?;
        std::fs::write(filename, data).context("Failed to write settings")
    }

    pub fn get_default_account(&self) -> Option<&Account> {
        if let Some(default) = self.default {
            self.get(default)
        } else {
            None
        }
    }
}

impl Drop for AccountData {
    fn drop(&mut self) {
        let _ = self.save();
    }
}

impl Deref for AccountData {
    type Target = Vec<Account>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for AccountData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}
