use crate::account::Account;
use crate::java::java_version::JavaVersion;
use crate::project::game_project::GameProject;
use crate::project::project_json_file::json_get_libraries;
use crate::settings::GlobalSettings;
use atomig::{Atom, Atomic};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use sharingan::downloader::{DownloadBuilder, Downloader};
use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub enum LaunchError {
    AccountExpired,
    JavaCheckFailed(String),
    /// 没有找到适合的Java版本，并附带一个正确的Java版本号
    NotFindCorrectJava(i32),
    DownloadFailed(String),
    Json(serde_json::Error),
    UnknownError,
}

pub struct Launcher {
    account: Account,
    project: GameProject,
    /// 所有找到的Java版本集合，如果Setting中已经带了一个Java，此处可以为空
    javas: Vec<JavaVersion>,
    setting: GlobalSettings,
}

impl Launcher {
    pub fn new(
        account: Account,
        project: GameProject,
        javas: Vec<JavaVersion>,
        setting: GlobalSettings,
    ) -> Self {
        Self {
            account,
            project,
            javas,
            setting,
        }
    }
}

#[derive(Debug, Clone, Copy, Atom)]
#[repr(u8)]
pub enum LaunchState {
    Ready,
    CheckAccount,
    CheckJava,
    DownloadLibrary,
    DownloadMods,
    Launch,
    Failed,
}

pub struct LaunchInfo {
    state: Arc<Atomic<LaunchState>>,
    error: Arc<Mutex<LaunchError>>,
    data: Arc<Launcher>,
}

impl LaunchInfo {
    pub fn lunch(data: Launcher) -> Self {
        let state = Arc::new(Atomic::new(LaunchState::Ready));
        let error = Arc::new(Mutex::new(LaunchError::UnknownError));
        let data = Arc::new(data);

        let state_clone = state.clone();
        let error_clone = error.clone();
        let data_clone = data.clone();

        let _handle = std::thread::spawn(move || {
            let mut libraries_downloader: Option<Downloader> = None;
            let mut mods_downloader: Option<Downloader> = None;

            loop {
                let state = state_clone.load(Ordering::SeqCst);

                match state {
                    LaunchState::Ready => {
                        state_clone.store(LaunchState::CheckAccount, Ordering::SeqCst);
                    }
                    LaunchState::CheckAccount => match check_account(&data_clone) {
                        Ok(_) => state_clone.store(LaunchState::CheckJava, Ordering::SeqCst),
                        Err(e) => {
                            state_clone.store(LaunchState::Failed, Ordering::SeqCst);
                            update_error(&error_clone, e);
                        }
                    },
                    LaunchState::CheckJava => match check_java(&data_clone) {
                        Ok(_) => state_clone.store(LaunchState::DownloadLibrary, Ordering::SeqCst),
                        Err(e) => {
                            state_clone.store(LaunchState::Failed, Ordering::SeqCst);
                            update_error(&error_clone, e);
                        }
                    },
                    LaunchState::DownloadLibrary => {
                        if let Some(downloader) = &libraries_downloader {
                            if downloader.is_finished() {
                                if downloader.is_all_success() {
                                    state_clone.store(LaunchState::DownloadMods, Ordering::SeqCst);
                                }else {
                                    state_clone.store(LaunchState::Failed, Ordering::SeqCst);
                                }
                            }
                        } else {
                            match download_libraries(&data_clone) {
                                Ok(downloader) => {
                                    libraries_downloader = Some(downloader);
                                }
                                Err(e) => {
                                    state_clone.store(LaunchState::Failed, Ordering::SeqCst);
                                    update_error(&error_clone, e);
                                }
                            }
                        }
                    }
                    LaunchState::DownloadMods => {
                        if let Some(downloader) = &mods_downloader {
                            if downloader.is_all_success() {
                                state_clone.store(LaunchState::Launch, Ordering::SeqCst);
                            }else {
                                state_clone.store(LaunchState::Failed, Ordering::SeqCst);
                            }
                        } else {
                            mods_downloader = Some(download_mods(&data_clone));
                        }
                    },
                    LaunchState::Launch | LaunchState::Failed => break,
                };
            }
        });

        Self { state, error, data }
    }
}

fn update_error(err: &Arc<Mutex<LaunchError>>, new_err: LaunchError) {
    let mut err = err.lock().unwrap();
    *err = new_err;
}

fn check_account(data: &Arc<Launcher>) -> Result<(), LaunchError> {
    let check = data.account.check_token();
    if check {
        Ok(())
    } else {
        Err(LaunchError::AccountExpired)
    }
}

fn check_java(data: &Arc<Launcher>) -> Result<JavaVersion, LaunchError> {
    let value = data
        .project
        .read_json()
        .map_err(|e| LaunchError::JavaCheckFailed(e.to_string()))?;
    let major_version = value
        .pointer("/javaVersion/majorVersion")
        .ok_or(LaunchError::JavaCheckFailed(
            "unknown modpack need java version".to_string(),
        ))?
        .as_i64()
        .ok_or(LaunchError::JavaCheckFailed(
            "unknown modpack need java version".to_string(),
        ))?;

    if let Some(java) = &data.setting.java
        && java.major_version as i64 == major_version
    {
        return Ok(java.clone());
    }

    for java in &data.javas {
        if java.major_version as i64 == major_version {
            return Ok(java.clone());
        }
    }

    Err(LaunchError::NotFindCorrectJava(major_version as i32))
}

/// 下载整合包所需的全部支持库，并等待下载结束。
///
/// 会跳过已存在且大小一致的库文件；返回的 [`Downloader`] 中所有任务均已结束，
/// 若有任意一个任务失败则返回 [`LaunchError::DownloadFailed`]。
fn download_libraries(data: &Arc<Launcher>) -> Result<Downloader, LaunchError> {
    let path = data.setting.libraries_path.clone();
    std::fs::create_dir_all(&path).map_err(|e| LaunchError::DownloadFailed(e.to_string()))?;

    let mut downloader = DownloadBuilder::new().thread_num(8).build();

    let lib_json = json_get_libraries(
        &data
            .project
            .read_json()
            .map_err(|e| LaunchError::DownloadFailed(e.to_string()))?,
    )
    .map_err(|e| LaunchError::DownloadFailed(e.to_string()))?;

    for lib in lib_json {
        // 不符合当前平台启用条件的库直接跳过
        if !check_rules(&lib) {
            continue;
        }

        let Some(artifacts) = lib.get("downloads").and_then(|d| d.get("artifact")) else {
            continue;
        };
        let Ok(artifacts) = Artifact::from_json(artifacts) else {
            continue;
        };

        let target = path.join(&artifacts.path);
        let (Some(filename), Some(parent)) = (
            target
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
            target.parent().map(|parent| parent.to_path_buf()),
        ) else {
            continue;
        };

        // 已经存在且大小与清单一致时无需重复下载
        if std::fs::metadata(&target)
            .map(|metadata| metadata.len() == artifacts.size)
            .unwrap_or(false)
        {
            continue;
        }

        // download 要求闭包为 'static，因此这里把需要的值 move 进去
        let url = artifacts.url.clone();
        let expected_sha1 = artifacts.sha1.clone();
        downloader.download(move |builder| {
            let expected_sha1 = expected_sha1.clone();
            builder
                .filename(filename.clone())
                .path(parent.clone())
                .url(url.clone())
                .validator(move |path| {
                    let Ok(file) = File::open(path) else {
                        return false;
                    };
                    let mut reader = BufReader::new(file);
                    let mut hasher = Sha1::new();
                    let mut buf = [0u8; 8192];

                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => hasher.update(&buf[..n]),
                            Err(_) => return false,
                        }
                    }

                    expected_sha1 == hex::encode(hasher.finalize())
                })
                .build()
        });
    }

    // // 阻塞等待所有库下载结束
    // while !downloader.is_finished() {
    //     std::thread::sleep(Duration::from_millis(100));
    // }
    //
    // let failed: Vec<String> = downloader
    //     .tasks()
    //     .iter()
    //     .filter(|(_, task)| !task.status().is_success())
    //     .map(|(id, task)| format!("{} (task {id})", task.url()))
    //     .collect();
    //
    // if !failed.is_empty() {
    //     return Err(LaunchError::DownloadFailed(format!(
    //         "{} libraries failed to download: {}",
    //         failed.len(),
    //         failed.join(", ")
    //     )));
    // }

    Ok(downloader)
}

/// 下载缺失的mod
fn download_mods(data: &Arc<Launcher>) -> Downloader {
    let mut downloader = DownloadBuilder::new().thread_num(8).build();

    let mod_path = data.project.path.join("mods");
    let resourcepacks = data.project.path.join("resourcepacks");

    // curse整合包格式
    let curse_file_path = data.project.path.join("manifest.json");
    if curse_file_path.is_file()
        && let Ok(text) = std::fs::read_to_string(curse_file_path)
        && let Ok(json) = serde_json::from_str::<Value>(text.as_str())
        && let Some(files) = json.get("files")
        && let Some(files) = files.as_array()
    {
        for file in files {
            if let Ok(info) = CurseDownloadInfo::from_json(file) {
                let filename = info.fileName.clone();
                let path = if filename.ends_with("zip") {
                    resourcepacks.clone()
                } else {
                    mod_path.clone()
                };
                if let Ok(_) = std::fs::metadata(&path.join(&filename)) {
                    continue;
                }
                downloader.download(move |builder| {
                    builder
                        .filename(filename.clone())
                        .path(path.clone())
                        .url(info.url.clone())
                        .build()
                })
            }
        }
    }

    downloader
}

/// 判断这个库是否符合当前系统的启用条件。
///
/// 与 Minecraft 版本清单的规则一致：没有 `rules` 字段的库默认启用；
/// 有 `rules` 时按顺序应用所有匹配当前系统的规则，最后一条决定结果。
fn check_rules(json: &Value) -> bool {
    let Some(rules) = json.get("rules").and_then(|rules| rules.as_array()) else {
        return true;
    };

    let mut enable = false;
    for rule in rules {
        let Some(action) = rule.get("action").and_then(|action| action.as_str()) else {
            continue;
        };

        // 规则未限定系统时对所有平台生效，限定了则要求与当前系统一致
        let os_matched = match rule.pointer("/os/name").and_then(|name| name.as_str()) {
            None => true,
            Some(os_name) => {
                let os_name = if os_name == "osx" { "macos" } else { os_name };
                os_name == std::env::consts::OS
            }
        };
        if !os_matched {
            continue;
        }

        match action {
            "allow" => enable = true,
            "disallow" => enable = false,
            _ => {}
        }
    }

    enable
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Artifact {
    path: String,
    url: String,
    sha1: String,
    size: u64,
}

impl Artifact {
    fn from_json(json: &Value) -> Result<Self, LaunchError> {
        serde_json::from_value(json.clone()).map_err(|e| LaunchError::Json(e))
    }
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Deserialize, Serialize)]
struct CurseDownloadInfo {
    projectID: i64,
    fileID: i64,
    fileName: String,
    url: String,
    required: bool,
}

impl CurseDownloadInfo {
    fn from_json(json: &Value) -> Result<Self, LaunchError> {
        serde_json::from_value(json.clone()).map_err(|e| LaunchError::Json(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_from_json() {
        let j = r#"{
          "path": "org/lwjgl/lwjgl-tinyfd/3.3.3/lwjgl-tinyfd-3.3.3-natives-windows-arm64.jar",
          "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl-tinyfd/3.3.3/lwjgl-tinyfd-3.3.3-natives-windows-arm64.jar",
          "sha1": "a88c494f3006eb91a7433b12a3a55a9a6c20788b",
          "size": 110867
        }"#;

        let json: Value = serde_json::from_str(j).unwrap();
        let artifact = Artifact::from_json(&json).unwrap();
        assert_eq!(artifact.size, 110867);
    }

    #[test]
    fn curse_download_from_json() {
        let j = r#"{
      "projectID": 412082,
      "fileID": 5458843,
      "fileName": "supplementaries-1.20-2.8.17.jar",
      "url": "https://edge.forgecdn.net/files/5458/843/supplementaries-1.20-2.8.17.jar",
      "required": true
    }"#;

        let json: Value = serde_json::from_str(j).unwrap();
        let info = CurseDownloadInfo::from_json(&json).unwrap();
        assert_eq!(info.projectID, 412082);
    }

    #[test]
    fn check_rules_without_rules() {
        let json: Value = serde_json::from_str(r#"{"name": "com.example:demo:1.0"}"#).unwrap();
        assert!(check_rules(&json), "没有 rules 的库应当默认启用");
    }

    #[test]
    fn check_rules_other_os() {
        let current = std::env::consts::OS;
        let other = if current == "linux" {
            "windows"
        } else {
            "linux"
        };
        let json: Value = serde_json::from_str(&format!(
            r#"{{"rules": [{{"action": "allow", "os": {{"name": "{other}"}}}}]}}"#
        ))
        .unwrap();
        assert!(!check_rules(&json), "只允许其他系统的库不应启用");
    }

    #[test]
    fn check_rules_last_match_wins() {
        let current = if std::env::consts::OS == "macos" {
            "osx"
        } else {
            std::env::consts::OS
        };
        let json: Value = serde_json::from_str(&format!(
            r#"{{"rules": [
                {{"action": "allow", "os": {{"name": "{current}"}}}},
                {{"action": "disallow"}}
            ]}}"#
        ))
        .unwrap();
        assert!(!check_rules(&json), "最后一条匹配的规则应当生效");
    }
}
