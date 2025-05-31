use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

pub trait SettingTrait: Sized {
    fn read(json: Option<Value>) -> Result<Self>;
    fn write(&self) -> Result<Value>;
    //发送给tauri (name, value)
    fn send(&self) -> Result<(String, Value)>;
    //接收来自tauri的值
    fn receive(&mut self, value: Vec<String>) -> Result<()>;
}
