use serde::Serialize;

pub trait SettingTrait: Sized {
    type Output: Serialize;
    type Err: std::fmt::Display;
    //从文件读取，若无文件则为初始化
    fn read_from_file(&self) -> Result<Self, Self::Err>;
    fn write_to_file(&self) -> Result<(), Self::Err>;
    fn send(&self) -> Result<Self::Output, Self::Err>;
}
