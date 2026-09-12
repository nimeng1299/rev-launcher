use crate::java::java_version::JavaVersion;
use optfield::optfield;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use std::path::PathBuf;

#[optfield(pub ProjectSettings, attrs, doc, field_doc, merge_fn)]
#[derive(Debug, Clone, SmartDefault, Deserialize, Serialize)]
pub struct GlobalSettings {
    /// 选择的java版本，None则是自动选择
    pub java: Option<JavaVersion>,
    /// 分配的内存，单位M，None则是自动分配
    pub memory: Option<usize>,
    /// 游戏启动的窗口大小
    pub window_size: GameWindowSize,
    /// 支持库的路径，一般在启动器文件夹的/.minecraft/libraries中
    #[default(
        _code = "std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(\".\")).join(\".minecraft\").join(\"libraries\")"
    )]
    pub libraries_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum GameWindowSize {
    Windowed(usize, usize),
    Minimum,
    Maximum,
    Fullscreen,
}

impl Default for GameWindowSize {
    fn default() -> Self {
        GameWindowSize::Windowed(1024, 640)
    }
}
