use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use which::which;

use crate::api::dirs;
use crate::api::version::Version;

use super::setting_trait::SettingTrait;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct JavaVersion {
    path: String,
    version: Version,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JavaVersions {
    versions: Vec<JavaVersion>,
    select: u8,
}

impl JavaVersions {
    pub fn new() -> Self {
        JavaVersions {
            versions: Vec::new(),
            select: 0,
        }
    }

    pub fn add_version(&mut self, path: String, version: Version) {
        self.versions.push(JavaVersion { path, version });
    }

    //获取环境变量中java的版本
    pub fn load_path_versions() -> Self {
        let mut versions = JavaVersions::new();
        let v = which("java");
        if v.is_ok() {
            let path = v.unwrap();
            let version = get_java_version(&path);
            if version.is_err() {
                return versions;
            }
            let version = Version::from_string(&version.unwrap(), Some(&vec!['\"']));
            if version.is_err() {
                return versions;
            }
            versions.add_version(path.to_string_lossy().to_string(), version.unwrap());
        }
        versions
    }

    fn load_file_version() -> Result<Self> {
        let config_dir = dirs::get_config_dirs()?;

        let config_file_path = config_dir.join("java_versions.json");
        if config_file_path.exists() {
            let mut file = File::open(config_file_path)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            let java_versions: JavaVersions = serde_json::from_str(&contents)?;
            return Ok(java_versions);
        } else {
            return Ok(JavaVersions::load_path_versions());
        }
    }
}

impl SettingTrait for JavaVersions {
    fn read(json: Option<Value>) -> Result<Self> {
        match json {
            Some(value) => {
                let java_versions: JavaVersions = serde_json::from_value(value)?;
                Ok(java_versions)
            }
            None => JavaVersions::load_file_version(),
        }
    }
    fn write(&self) -> Result<Value> {
        serde_json::to_value(self)
            .map_err(|e| anyhow::anyhow!("Failed to serialize JavaVersions: {}", e))
    }
    fn send(&self) -> Result<serde_json::Value> {
        let json = serde_json::to_value(self)?;
        Ok(json)
    }
    fn receive(&mut self, value: Vec<String>) -> Result<()> {
        todo!();
        Ok(())
    }
}

fn get_java_version(java_path: &PathBuf) -> Result<String> {
    // java -version 会将输出写到 stderr
    let output = Command::new(java_path)
        .arg("-version")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to execute `{}`: {}", java_path.display(), e))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    // 第一行通常形如：java version "1.8.0_281"
    stderr
        .lines()
        .next()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("No output from java -version"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::version::Version;

    #[test]
    fn test_java_versions() {
        let java_versions = JavaVersions::load_path_versions();
        assert!(!java_versions.versions.is_empty());
        println!("{:#?}", java_versions);
    }
}
