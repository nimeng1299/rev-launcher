use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum JavaError {
    Io(std::io::Error),
    CommandRunFailed(String),
    CommandNoOutput,
    UnknownVersion,
    NotJavaExecutableFile,
}

impl Display for JavaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::CommandRunFailed(e) => write!(f, "CommandRunFailed error: {}", e),
            Self::CommandNoOutput => write!(f, "Command No Output"),
            Self::UnknownVersion => write!(f, "Unknown Version"),
            Self::NotJavaExecutableFile => write!(f, "Not Java Executable File"),
        }
    }
}
