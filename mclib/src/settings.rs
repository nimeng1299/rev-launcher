use crate::java::java_version::JavaVersion;
use optfield::optfield;
use serde::{Deserialize, Serialize};

#[optfield(pub ProjectSettings, attrs, doc, field_doc, merge_fn)]
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GlobalSettings {
    /// 选择的java版本，None则是自动选择
    pub java: Option<JavaVersion>,
    /// 分配的内存，单位M，None则是自动分配
    pub memory: Option<usize>,
    /// 游戏启动的窗口大小
    pub window_size: GameWindowSize,
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
        GameWindowSize::Windowed(860, 640)
    }
}
