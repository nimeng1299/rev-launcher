//! mclib 统一的错误类型。
//!
//! 原先分散在各模块的 `LaunchError`、`JavaError`、`GameProjectError`
//! 已全部合并到这里，模块内只使用 `crate::error::Error`。

use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    // ------------------------------------------------------------------
    // 通用
    // ------------------------------------------------------------------
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("network error: {0}")]
    Network(#[from] ureq::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Deserialize failed，Paht: `{path}`，error: {message}")]
    Deserialize { path: String, message: String },

    #[error("unknown path")]
    UnknownPath,

    #[error("not find setting file: {}", .0.display())]
    NotFindSettingFile(PathBuf),

    // ------------------------------------------------------------------
    // 启动流程（原 LaunchError）
    // ------------------------------------------------------------------
    #[error("账号已过期")]
    AccountExpired,

    #[error("{0}")]
    JavaCheckFailed(String),

    /// 没有找到适合的Java版本，并附带一个正确的Java版本号
    #[error("未找到 Java {0}")]
    NotFindCorrectJava(i32),

    #[error("{0}")]
    DownloadFailed(String),

    #[error("{0}")]
    LaunchFailed(String),

    #[error("{0}")]
    LogFailed(String),

    #[error("游戏异常退出，退出码：{}", .0.map_or_else(|| "未知".to_string(), |code| code.to_string()))]
    ProcessExited(Option<i32>),

    #[error("启动失败，请查看启动日志")]
    UnknownError,

    // ------------------------------------------------------------------
    // Java 探测
    // ------------------------------------------------------------------
    #[error("CommandRunFailed error: {0}")]
    CommandRunFailed(String),

    #[error("Command No Output")]
    CommandNoOutput,

    #[error("Unknown Version")]
    UnknownVersion,

    #[error("Not Java Executable File")]
    NotJavaExecutableFile,
}
