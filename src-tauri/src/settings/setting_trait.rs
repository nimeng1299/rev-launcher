use anyhow::Result;
use serde::Serialize;

pub trait SettingTrait: Sized {
    type Output: Serialize;
    //从文件读取，若无文件则为初始化
    fn read_from_file(&self) -> Result<Self>;
    fn write_to_file(&self) -> Result<()>;
    //发送给tauri (name, value)
    fn send(&self) -> Result<(String, Self::Output)>;
}
