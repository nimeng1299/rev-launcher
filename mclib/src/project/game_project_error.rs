use std::fmt::{Display, Formatter};
use std::path::PathBuf;

#[derive(Debug)]
pub enum GameProjectError {
    Io(std::io::Error),
    Json(serde_json::Error),
    UnknownPath,
    NotFindSettingFile(PathBuf),
}

impl Display for GameProjectError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Json(e) => write!(f, "JSON error: {}", e),
            Self::UnknownPath => write!(f, "unknown path"),
            Self::NotFindSettingFile(path) => {
                write!(f, "not find setting file: {}", path.display())
            }
        }
    }
}
