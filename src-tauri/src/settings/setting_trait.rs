use std::path::PathBuf;

use anyhow::{Ok, Result};
use serde::Serialize;
use serde_json::Value;

pub trait SettingTrait: Sized {
    fn read(json: Option<Value>) -> Result<Self>;
    fn write(&self) -> Result<Value>;
    //发送给tauri (name, value)
    fn send(&self) -> Result<Value>;
    //接收来自tauri的值
    fn receive(&mut self, value: Vec<String>) -> Result<()>;

    fn read_modpack(json: Option<Value>) -> Result<Option<Self>> {
        match json {
            Some(value) => {
                let setting = Self::read(Some(value))?;
                Ok(Some(setting))
            }
            None => Ok(None),
        }
    }
}
