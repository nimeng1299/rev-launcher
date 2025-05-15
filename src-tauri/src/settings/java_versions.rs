use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use which::which;

use crate::api::version::Version;

#[derive(Debug)]
struct JavaVersion {
    path: String,
    version: Version,
}

#[derive(Debug)]
pub struct JavaVersions {
    versions: Vec<JavaVersion>,
}

impl JavaVersions {
    pub fn new() -> Self {
        JavaVersions {
            versions: Vec::new(),
        }
    }

    pub fn add_version(&mut self, path: String, version: Version) {
        self.versions.push(JavaVersion { path, version });
    }

    pub fn load_versions() -> Self {
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
}
fn get_java_version(java_path: &PathBuf) -> Result<String, String> {
    // java -version 会将输出写到 stderr
    let output = Command::new(java_path)
        .arg("-version")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("Failed to execute `{}`: {}", java_path.display(), e))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    // 第一行通常形如：java version "1.8.0_281"
    stderr
        .lines()
        .next()
        .map(|s| s.to_string())
        .ok_or_else(|| "No output from java -version".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::version::Version;

    #[test]
    fn test_java_versions() {
        let java_versions = JavaVersions::load_versions();
        assert!(!java_versions.versions.is_empty());
        println!("{:#?}", java_versions);
    }
}
