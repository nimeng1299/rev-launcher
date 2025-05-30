use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

pub trait SettingTrait: 'static {
    //type Output: Serialize;
    //从文件读取，若无文件则为初始化
    fn read_from_file(config_path: Option<PathBuf>) -> Result<Box<dyn SettingTrait>>
    where
        Self: Sized;
    fn write_to_file(&self) -> Result<()>;
    //发送给tauri (name, value)
    fn send(&self) -> Result<(String, Value)>;
    //接收来自tauri的值
    fn receive(&mut self, value: Vec<String>) -> Result<()>;
}
