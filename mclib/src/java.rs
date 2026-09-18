pub mod java_version;

use crate::java::java_version::JavaVersion;
use std::path::PathBuf;
use std::process::Command;

pub fn find_java_paths() -> Vec<PathBuf> {
    let mut results = Vec::new();

    #[cfg(target_os = "windows")]
    let output = Command::new("where").arg("java").output();

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    let output = {
        // 尝试 `command -v`
        let cmd = Command::new("command").args(&["-v", "java"]).output();

        // 如果失败，则回退到 `which -a` 列出所有
        if cmd.is_err() || !cmd.as_ref().unwrap().status.success() {
            Command::new("which").args(&["-a", "java"]).output()
        } else {
            cmd.unwrap()
        }
    };

    if let Ok(out) = output
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let path = PathBuf::from(line.trim());
            if path.exists() {
                results.push(path);
            }
        }
    }

    results
}

pub fn find_javas() -> Vec<JavaVersion> {
    find_java_paths()
        .iter()
        .map(|path| JavaVersion::from_path(path))
        .filter_map(Result::ok)
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::java::find_java_paths;

    #[test]
    fn find_java_paths_test() {
        let paths = find_java_paths();
        assert!(paths.len() > 0);
    }
}
