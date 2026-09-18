use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JavaVersion {
    pub java_runtime: JavaRuntime,
    pub version: String,
    pub major_version: i32,
    pub path_buf: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum JavaRuntime {
    JDK,
    JRE,
}

impl fmt::Display for JavaRuntime {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::JDK => "JDK",
            Self::JRE => "JRE",
        };
        write!(f, "{}", s)
    }
}

impl JavaVersion {
    pub fn from_path(path: &PathBuf) -> Result<JavaVersion, crate::error::Error> {
        let output = Command::new(path)
            .arg("-version")
            .output()
            .map_err(|e| crate::error::Error::Io(e))?;

        if !output.status.success() {
            return Err(crate::error::Error::CommandRunFailed(format!(
                "执行 '{} -version' 失败",
                path.display()
            )));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);

        let first_line = stderr.lines().next().ok_or(crate::error::Error::CommandNoOutput)?;
        let version = extract_version_from_line(first_line).ok_or(crate::error::Error::UnknownVersion)?;

        let parent = path.parent().ok_or(crate::error::Error::NotJavaExecutableFile)?;
        let javac_name = if cfg!(target_os = "windows") {
            "javac.exe"
        } else {
            "javac"
        };
        let javac_path = parent.join(javac_name);
        let runtime = if std::fs::metadata(&javac_path).is_ok() {
            JavaRuntime::JDK
        } else {
            JavaRuntime::JRE
        };

        let major_version = parse_major_version(&version).ok_or(crate::error::Error::UnknownVersion)?;

        Ok(JavaVersion {
            java_runtime: runtime,
            version,
            major_version,
            path_buf: path.clone(),
        })
    }
}

/// 从类似 `java version "1.8.0_201"` 的行中提取双引号内的版本号。
fn extract_version_from_line(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let end = line[start + 1..].find('"')?;
    let version = &line[start + 1..start + 1 + end];
    Some(version.to_string())
}

/// 解析版本号中的主版本（如 "1.8" → 8， "11.0.2" → 11）。
fn parse_major_version(version: &str) -> Option<i32> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.is_empty() {
        return None;
    }

    let major_str = if parts[0] == "1" {
        // 对于 1.8.0，主版本是第二个数字
        parts.get(1).copied().unwrap_or("0")
    } else {
        parts[0]
    };

    major_str.parse::<i32>().ok()
}
