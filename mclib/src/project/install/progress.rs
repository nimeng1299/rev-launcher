use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use atomig::{Atom, Atomic};
use serde_json::Value;
use sha1::Digest;
use sharingan::downloader::{DownloadBuilder, Downloader};
use sharingan::status::DownloadFailure;
use crate::project::game_project::GameProject;
use crate::project::versions;

#[derive(Clone)]
pub struct InstallProgress {
    state: Arc<Atomic<InstallState>>,
    error: Arc<Mutex<Option<Arc<crate::error::Error>>>>,
    success: Arc<OnceLock<GameProject>>,
    /// 运行文件（客户端 jar）的下载器，进入 `InstallJar` 阶段后可查询。
    jar_downloader: Arc<OnceLock<Arc<Downloader>>>,
}

impl InstallProgress {
    fn new() -> Self {
        Self {
            state: Arc::new(Atomic::new(InstallState::Ready)),
            error: Arc::new(Mutex::new(None)),
            success: Arc::new(OnceLock::new()),
            jar_downloader: Arc::new(OnceLock::new()),
        }
    }

    /// 安装原版
    ///
    /// name: 名字
    /// path: 项目的父路径（项目将安装在`path.join(name)`）
    pub fn install_minecraft<P: AsRef<Path>>(
        name: &String,
        path: P,
        version: &versions::minecreft::Version,
    ) -> Self {
        let progress = Self::new();
        let path = path.as_ref().to_path_buf().join(name);
        let name = name.clone();
        let url = version.url.clone();

        let handle = progress.clone();
        std::thread::spawn(move || {
            if let Err(error) = handle.run_minecraft(&name, &path, &url) {
                handle.set_error(error);
            }
        });

        progress
    }

    /// 返回当前安装状态。
    pub fn state(&self) -> InstallState {
        self.state.load(Ordering::SeqCst)
    }

    /// 获取失败原因；只有发生错误时才返回 Some。
    pub fn error(&self) -> Option<Arc<crate::error::Error>> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 安装成功后返回创建的游戏项目；尚未完成时返回 `None`。
    pub fn success(&self) -> Option<GameProject> {
        self.success.get().cloned()
    }

    /// 获取客户端 jar 下载器的共享句柄；尚未创建时返回 `None`，不会等待初始化。
    ///
    /// 下载器创建后会一直保留，可通过它查询任务和进度。
    pub fn jar_downloader(&self) -> Option<Arc<Downloader>> {
        self.jar_downloader.get().cloned()
    }

    /// 记录错误并把状态切换为 `Failed`。
    fn set_error(&self, error: crate::error::Error) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(error));
        self.state.store(InstallState::Failed, Ordering::SeqCst);
    }

    fn run_minecraft(
        &self,
        name: &str,
        path: &Path,
        url: &str,
    ) -> Result<(), crate::error::Error> {
        std::fs::create_dir_all(path)?;

        // 1. 下载版本 JSON，写入 <path>/<name>.json。
        self.state.store(InstallState::DownloadVersionJson, Ordering::SeqCst);
        let json = prepare_manifest(name, download_json(url)?)?;
        std::fs::write(
            path.join(format!("{name}.json")),
            serde_json::to_string_pretty(&json)?,
        )?;

        // 2. 下载客户端运行 jar，写入 <path>/<name>.jar。
        self.state.store(InstallState::InstallJar, Ordering::SeqCst);
        download_jar(&self.jar_downloader, name, path, &json)?;

        // 3. 从 <name>.json 读取版本信息，生成并写入项目文件。
        let project = crate::project::game_project::get_game_project(&path.to_path_buf())?;
        let _ = self.success.set(project);
        self.state.store(InstallState::Success, Ordering::SeqCst);
        Ok(())
    }
}

impl std::fmt::Debug for InstallProgress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstallProgress")
            .field("state", &self.state())
            .field("error", &self.error())
            .field("success", &self.success)
            .finish_non_exhaustive()
    }
}

/// 把原版清单改造成启动器可识别的版本 JSON：
/// `patches` 第一项记录原始游戏版本（id 固定为 `game`，与加载器清单格式一致），
/// 顶层 `id` 改为项目名，使启动时按 `<name>.jar` 查找运行文件。
fn prepare_manifest(name: &str, manifest: Value) -> Result<Value, crate::error::Error> {
    let mut json = manifest;
    let game_version = json
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::Error::DownloadFailed("版本 JSON 缺少 id".into()))?
        .to_string();
    json["patches"] = serde_json::json!([{"id": "game", "version": game_version}]);
    json["id"] = Value::String(name.to_string());
    Ok(json)
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .user_agent("RevLauncher/0.1")
        .build()
        .new_agent()
}

/// 下载一个 URL 并解析为 JSON。
fn download_json(url: &str) -> Result<Value, crate::error::Error> {
    let mut resp = http_agent()
        .get(url)
        .header("accept", "application/json")
        .call()?;
    if !resp.status().is_success() {
        return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
    }
    Ok(serde_json::from_str(&resp.body_mut().read_to_string()?)?)
}

/// 用 sharingan 下载器把清单里的客户端 jar 下载为 `<path>/<name>.jar`。
///
/// 校验清单给出的 size/sha1；下载器存入 `slot` 供外部轮询进度。
fn download_jar(
    slot: &OnceLock<Arc<Downloader>>,
    name: &str,
    path: &Path,
    json: &Value,
) -> Result<(), crate::error::Error> {
    let client = json
        .pointer("/downloads/client")
        .ok_or_else(|| {
            crate::error::Error::DownloadFailed("版本 JSON 缺少 downloads.client".into())
        })?;
    let url = client
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            crate::error::Error::DownloadFailed("版本 JSON 缺少 downloads.client.url".into())
        })?
        .to_string();
    let sha1 = client
        .get("sha1")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let size = client.get("size").and_then(Value::as_u64);

    let mut downloader = DownloadBuilder::new().thread_num(1).build();
    let dir = path.to_path_buf();
    let filename = format!("{name}.jar");
    downloader.download(move |builder| {
        builder
            .url(url.clone())
            .path(dir.clone())
            .filename(filename.clone())
            .overwrite(true)
            .validator(move |jar| {
                let file =
                    std::fs::File::open(&jar).map_err(DownloadFailure::IOError)?;
                let metadata = file.metadata().map_err(DownloadFailure::IOError)?;
                if !metadata.is_file()
                    || metadata.len() == 0
                    || size.is_some_and(|size| size != metadata.len())
                {
                    return Err(DownloadFailure::ValidationError);
                }
                if let Some(sha1) = &sha1 {
                    let mut reader = std::io::BufReader::new(file);
                    let mut hasher = sha1::Sha1::new();
                    let mut buffer = [0; 64 * 1024];
                    loop {
                        match std::io::Read::read(&mut reader, &mut buffer) {
                            Ok(0) => break,
                            Ok(count) => hasher.update(&buffer[..count]),
                            Err(e) => return Err(DownloadFailure::IOError(e)),
                        }
                    }
                    if hex::encode(hasher.finalize()) != *sha1 {
                        return Err(DownloadFailure::ValidationError);
                    }
                }
                Ok(())
            })
            .build()
    });
    let downloader = slot.get_or_init(|| Arc::new(downloader));

    while !downloader.is_finished() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let failures: Vec<_> = downloader
        .tasks()
        .values()
        .filter(|task| !task.status().is_success())
        .map(|task| {
            let reason = task.failed_reason();
            format!("{}（{reason:?}）", task.filename())
        })
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(crate::error::Error::DownloadFailed(format!(
            "下载失败：{}",
            failures.join(", ")
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Atom)]
#[repr(u8)]
pub enum InstallState {
    Ready,
    DownloadVersionJson,
    InstallJar,
    Success,
    Failed,
}
