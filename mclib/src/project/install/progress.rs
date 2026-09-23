use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use atomig::{Atom, Atomic};
use serde_json::{Map, Value};
use sha1::Digest;
use sharingan::downloader::{DownloadBuilder, Downloader};
use sharingan::status::DownloadFailure;
use crate::java::java_version::JavaVersion;
use crate::launch::arguments::{
    RuleContext, library_path, maven_path, relative_path, rules_allow,
};
use crate::project::game_project::GameProject;
use crate::project::versions;

/// 名称形式的 Forge 支持库默认按顺序尝试的 Maven 仓库。
const MAVEN_REPOSITORIES: &[&str] = &[
    "https://maven.minecraftforge.net",
    "https://libraries.minecraft.net",
    "https://repo.maven.apache.org/maven2",
];

#[derive(Clone)]
pub struct InstallProgress {
    state: Arc<Atomic<InstallState>>,
    error: Arc<Mutex<Option<Arc<crate::error::Error>>>>,
    success: Arc<OnceLock<GameProject>>,
    /// 运行文件（客户端 jar）的下载器，进入 `InstallJar` 阶段后可查询。
    jar_downloader: Arc<OnceLock<Arc<Downloader>>>,
    /// 安装器依赖的支持库下载器，进入 `DownloadLibraries` 阶段后可查询。
    libraries_downloader: Arc<OnceLock<Arc<Downloader>>>,
}

impl InstallProgress {
    fn new() -> Self {
        Self {
            state: Arc::new(Atomic::new(InstallState::Ready)),
            error: Arc::new(Mutex::new(None)),
            success: Arc::new(OnceLock::new()),
            jar_downloader: Arc::new(OnceLock::new()),
            libraries_downloader: Arc::new(OnceLock::new()),
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
        // 只清理本次安装新建的目录，避免误删已存在的同名文件夹。
        let created = !path.exists();

        let handle = progress.clone();
        std::thread::spawn(move || {
            if let Err(error) = handle.run_minecraft(&name, &path, &url) {
                if created {
                    let _ = std::fs::remove_dir_all(&path);
                }
                handle.set_error(error);
            }
        });

        progress
    }

    /// 安装 Forge。
    ///
    /// name: 名字；path: 项目的父路径（项目将安装在 `path.join(name)`）；
    /// libraries_path: 支持库目录（安装所需的库和处理器产物都放在这里，
    /// 应使用与启动时一致的 `settings.libraries_path`）；
    /// java: 用于运行安装处理器的 Java；version: 要安装的 Forge 版本。
    ///
    /// 在 `<name>/temp` 中下载并解压安装器，解析 `install_profile.json`，
    /// 下载缺失的依赖库后依次执行 processors，最后把合并后的
    /// `version.json` 写成 `<name>.json`。
    pub fn install_forge<P: AsRef<Path>>(
        name: &String,
        path: P,
        libraries_path: P,
        java: JavaVersion,
        version: &versions::forge::Version,
    ) -> Self {
        let progress = Self::new();
        let path = path.as_ref().to_path_buf().join(name);
        let libraries_path = libraries_path.as_ref().to_path_buf();
        let name = name.clone();
        let version = version.clone();
        // 只清理本次安装新建的目录，避免误删已存在的同名文件夹。
        let created = !path.exists();

        let handle = progress.clone();
        std::thread::spawn(move || {
            if let Err(error) =
                handle.run_forge(&name, &path, &libraries_path, &java, &version)
            {
                if created {
                    let _ = std::fs::remove_dir_all(&path);
                }
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

    /// 获取 Forge 支持库下载器的共享句柄；尚未创建时返回 `None`，不会等待初始化。
    ///
    /// 下载器创建后会一直保留，可通过它查询任务和进度。
    pub fn libraries_downloader(&self) -> Option<Arc<Downloader>> {
        self.libraries_downloader.get().cloned()
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

    /// Forge 安装流程：
    /// `<name>/temp` 下载并解压安装器 → 解析 `install_profile.json` →
    /// 下载支持库 → 依次执行 processors → 写出 `<name>.json`。
    fn run_forge(
        &self,
        name: &str,
        path: &Path,
        libraries_path: &Path,
        java: &JavaVersion,
        version: &versions::forge::Version,
    ) -> Result<(), crate::error::Error> {
        std::fs::create_dir_all(path)?;
        // 安装器变量和处理器参数都使用绝对路径。
        let root = std::path::absolute(path)?;
        let path = root.as_path();
        let temp_dir = path.join("temp");
        let libraries_dir = std::path::absolute(libraries_path)?;
        let libraries_dir = libraries_dir.as_path();

        // 1. 在 temp 中下载并解压安装器 jar，解析 install_profile.json。
        self.state.store(InstallState::DownloadInstaller, Ordering::SeqCst);
        let installer = temp_dir.join("installer.jar");
        download_file(&version.download_url, &installer)?;
        let profile = read_zip_text(&installer, "install_profile.json")
            .ok_or_else(|| failed("Forge 安装器缺少 install_profile.json"))??;
        extract_zip(&installer, &temp_dir)?;
        // 新版安装器通过 profile["json"] 指明版本 JSON 在压缩包内的位置；
        // 旧版安装器把版本信息直接嵌在 install_profile.json 的 versionInfo 中。
        let forge_json = if let Some(inner) = profile.get("json").and_then(Value::as_str) {
            Some(
                read_zip_text(&installer, inner.trim_start_matches('/'))
                    .ok_or_else(|| failed(format!("Forge 安装器缺少 {inner}")))??,
            )
        } else {
            profile.get("versionInfo").cloned()
        }
        .ok_or_else(|| failed("Forge 安装器缺少 version.json 和 versionInfo"))?;

        // 2. 下载游戏版本清单和原版客户端 jar（处理器的 {MINECRAFT_JAR} 输入）。
        self.state
            .store(InstallState::DownloadVersionJson, Ordering::SeqCst);
        let manifest_list = versions::minecreft::MinecreftVersions::get_versions()?;
        let vanilla_url = manifest_list
            .versions
            .iter()
            .find(|entry| entry.id == version.id)
            .map(|entry| entry.url.clone())
            .ok_or_else(|| {
                failed(format!("找不到 Minecraft {} 的版本清单", version.id))
            })?;
        let vanilla_json = download_json(&vanilla_url)?;
        self.state.store(InstallState::InstallJar, Ordering::SeqCst);
        download_jar(&self.jar_downloader, name, path, &vanilla_json)?;

        // 3. 下载安装器和 version.json 声明的支持库到 libraries_path。
        //    注意 data/processors 里以 [坐标] 引用的产物（client-slim、
        //    forge-client、mappings.txt 等）由处理器在本地生成，
        //    不在任何 Maven 仓库上，不能提前下载。
        self.state
            .store(InstallState::DownloadLibraries, Ordering::SeqCst);
        let mut artifacts = collect_libraries(&profile)?;
        artifacts.extend(collect_libraries(&forge_json)?);
        download_forge_libraries(
            &self.libraries_downloader,
            &libraries_dir,
            artifacts,
        )?;

        // 4. 构建变量表并按顺序执行 processors。
        self.state.store(InstallState::RunProcessors, Ordering::SeqCst);
        let mut variables = collect_data(
            &profile,
            &temp_dir,
            &libraries_dir,
            &installer,
            &path.join(format!("{name}.jar")),
        )?;
        run_processors(&profile, java, &libraries_dir, &mut variables)?;

        // 5. 合并版本 JSON：原版清单提供资源/规则等字段，version.json 提供
        //    mainClass、arguments 和支持库；写成 <name>.json 并附加 patches。
        self.state
            .store(InstallState::WriteVersionJson, Ordering::SeqCst);
        let merged = merge_manifests(vanilla_json, &forge_json)?;
        let manifest = prepare_forge_manifest(name, &version.version, merged)?;
        std::fs::write(
            path.join(format!("{name}.json")),
            serde_json::to_string_pretty(&manifest)?,
        )?;

        // 6. 清理临时文件并生成项目信息。
        let _ = std::fs::remove_dir_all(&temp_dir);
        let project = crate::project::game_project::get_game_project(&root)?;
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
                let file = File::open(&jar).map_err(DownloadFailure::IOError)?;
                let metadata = file.metadata().map_err(DownloadFailure::IOError)?;
                if !metadata.is_file()
                    || metadata.len() == 0
                    || size.is_some_and(|size| size != metadata.len())
                {
                    return Err(DownloadFailure::ValidationError);
                }
                if let Some(sha1) = &sha1 {
                    let hash = file_sha1(&jar).map_err(DownloadFailure::IOError)?;
                    if !hash.eq_ignore_ascii_case(sha1) {
                        return Err(DownloadFailure::ValidationError);
                    }
                }
                Ok(())
            })
            .build()
    });
    let downloader = slot.get_or_init(|| Arc::new(downloader));

    wait_finished(downloader);
    let failures = download_failures(downloader);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failed(format!("下载失败：{}", failures.join(", "))))
    }
}

// ---------------------------------------------------------------------
// Forge 安装辅助
// ---------------------------------------------------------------------

fn failed(message: impl Into<String>) -> crate::error::Error {
    crate::error::Error::DownloadFailed(message.into())
}

/// 计算文件的 SHA-1，返回小写十六进制字符串。
fn file_sha1(path: &Path) -> Result<String, std::io::Error> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = sha1::Sha1::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 等待下载器的所有任务结束（不区分成败）。
fn wait_finished(downloader: &Downloader) {
    while !downloader.is_finished() {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 汇总下载器中所有失败任务的文件名和原因。
fn download_failures(downloader: &Downloader) -> Vec<String> {
    downloader
        .tasks()
        .values()
        .filter(|task| !task.status().is_success())
        .map(|task| {
            let reason = task.failed_reason();
            format!("{}（{reason:?}）", task.filename())
        })
        .collect()
}

/// 把一个 URL 流式下载为本地文件（用于单个安装器 jar）。
fn download_file(url: &str, target: &Path) -> Result<(), crate::error::Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent("RevLauncher/0.1")
        .build()
        .new_agent();
    let response = agent.get(url).call()?;
    if !response.status().is_success() {
        return Err(ureq::Error::StatusCode(response.status().as_u16()).into());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(target)?;
    let mut reader = response.into_body().into_reader();
    std::io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    Ok(())
}

/// 从 zip/jar 中读取一个文本文件并解析为 JSON；文件不存在时返回 `None`。
fn read_zip_text(archive: &Path, name: &str) -> Option<Result<Value, crate::error::Error>> {
    let file = File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| failed(format!("读取安装器失败：{e}")))
        .ok()?;
    let Ok(mut entry) = zip.by_name(name) else {
        return None;
    };
    Some((|| {
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        Ok(serde_json::from_str(&text)?)
    })())
}

/// 解压 zip/jar 到目录；拒绝符号链接与越界路径。
fn extract_zip(archive: &Path, target: &Path) -> Result<(), crate::error::Error> {
    let mut zip = zip::ZipArchive::new(BufReader::new(File::open(archive)?))
        .map_err(|e| failed(format!("解压安装器失败：{e}")))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| failed(format!("解压安装器失败：{e}")))?;
        if entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(failed("安装器压缩包包含符号链接"));
        }
        let name = entry
            .enclosed_name()
            .ok_or_else(|| failed("安装器压缩包包含越界路径"))?;
        let output = target.join(relative_path(&name.to_string_lossy())?);
        std::fs::create_dir_all(output.parent().unwrap())?;
        let mut file = File::create(&output)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

/// 一份需要下载的支持库：相对路径、按序尝试的下载地址和可选的完整性信息。
#[derive(Clone)]
struct ForgeLibrary {
    /// 相对 libraries 目录的路径（Maven 布局）。
    path: PathBuf,
    /// 按顺序尝试的下载地址。
    urls: Vec<String>,
    sha1: Option<String>,
    size: Option<u64>,
}

impl ForgeLibrary {
    /// 从一条 libraries 记录解析下载信息；返回 `None` 表示该条目无需下载
    /// （没有可用文件，或显式标记为本地安装产物）。
    fn from_json(library: &Value) -> Result<Option<Self>, crate::error::Error> {
        let artifact = library.pointer("/downloads/artifact");
        let path = if let Some(path) = artifact
            .and_then(|artifact| artifact.get("path"))
            .and_then(Value::as_str)
        {
            relative_path(path)?
        } else if library.get("downloads").is_some() {
            // 有 downloads 但没有 artifact 的条目没有文件可下载。
            return Ok(None);
        } else {
            let name = library
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| failed("支持库缺少 name 和 downloads.artifact.path"))?;
            maven_path(name)?
        };
        let meta = artifact.unwrap_or(library);
        let sha1 = meta
            .get("sha1")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let size = meta.get("size").and_then(Value::as_u64);
        let mut urls = Vec::new();
        if let Some(url) = meta.get("url").and_then(Value::as_str) {
            if url.is_empty() {
                // 空 URL 是安装器生成的本地产物，不下载。
                return Ok(None);
            }
            urls.push(url.to_string());
        } else {
            let relative = path.to_string_lossy().replace('\\', "/");
            let mut push_base = |base: &str| {
                let url = format!("{}/{relative}", base.trim_end_matches('/'));
                if !urls.contains(&url) {
                    urls.push(url);
                }
            };
            if let Some(base) = library.get("url").and_then(Value::as_str) {
                if base.is_empty() {
                    return Ok(None);
                }
                push_base(base);
            } else {
                for base in MAVEN_REPOSITORIES {
                    push_base(base);
                }
            }
        }
        Ok(Some(Self {
            path,
            urls,
            sha1,
            size,
        }))
    }

    /// 判断已存在的文件是否可复用；逻辑与启动阶段的支持库校验一致。
    fn valid(&self, target: &Path) -> bool {
        let Ok(file) = File::open(target) else {
            return false;
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        if !metadata.is_file()
            || metadata.len() == 0
            || self.size.is_some_and(|size| size != metadata.len())
        {
            return false;
        }
        if let Some(expected) = &self.sha1 {
            return file_sha1(target).is_ok_and(|hash| hash == *expected);
        }
        if self
            .path
            .extension()
            .is_some_and(|ext| ext == "jar" || ext == "zip")
        {
            return zip::ZipArchive::new(file).is_ok_and(|archive| !archive.is_empty());
        }
        true
    }
}

/// 从清单的 `libraries` 数组收集需要下载的支持库。
fn collect_libraries(json: &Value) -> Result<Vec<ForgeLibrary>, crate::error::Error> {
    let mut result = Vec::new();
    let Some(libraries) = json.get("libraries").and_then(Value::as_array) else {
        return Ok(result);
    };
    let context = RuleContext::current(false);
    for library in libraries {
        if !rules_allow(library, &context)? {
            continue;
        }
        // 先调用 library_path 校验条目合法性，再解析下载信息。
        if library_path(library)?.is_none() {
            continue;
        }
        if let Some(library) = ForgeLibrary::from_json(library)? {
            result.push(library);
        }
    }
    Ok(result)
}

/// 多线程下载支持库到 `libraries_dir`；每个任务自带备用仓库地址，
/// 主地址失败后按顺序自动切换。已存在且校验通过的文件不会重复下载。
fn download_forge_libraries(
    slot: &OnceLock<Arc<Downloader>>,
    libraries_dir: &Path,
    artifacts: Vec<ForgeLibrary>,
) -> Result<(), crate::error::Error> {
    let mut pending = Vec::new();
    let mut paths = HashSet::new();
    for artifact in artifacts {
        if !paths.insert(artifact.path.clone()) {
            continue;
        }
        if artifact.valid(&libraries_dir.join(&artifact.path)) {
            continue;
        }
        if artifact.urls.is_empty() {
            return Err(failed(format!(
                "支持库 {} 没有可用的下载地址",
                artifact.path.display()
            )));
        }
        pending.push(artifact);
    }
    let mut downloader = DownloadBuilder::new().thread_num(8).build();
    for artifact in pending {
        let target = libraries_dir.join(&artifact.path);
        let filename = target
            .file_name()
            .ok_or_else(|| failed("支持库路径缺少文件名"))?
            .to_string_lossy()
            .into_owned();
        let parent = target
            .parent()
            .ok_or_else(|| failed("支持库路径缺少父目录"))?
            .to_path_buf();
        downloader.download(move |builder| {
            let mut builder = builder
                .filename(filename)
                .path(parent)
                .url(artifact.urls[0].clone())
                .overwrite(true)
                .timeout(Some(Duration::from_secs(120)));
            // 其余地址作为备用配置，下载或校验失败后按顺序重试。
            for url in artifact.urls.iter().skip(1) {
                let url = url.clone();
                builder = builder.add_fallback(move |mut options| {
                    options.url = url.clone();
                    Ok(options)
                });
            }
            let validator = artifact.clone();
            builder
                .validator(move |file| {
                    if validator.valid(&file) {
                        Ok(())
                    } else {
                        Err(DownloadFailure::ValidationError)
                    }
                })
                .build()
        });
    }
    let downloader = slot.get_or_init(|| Arc::new(downloader));
    wait_finished(downloader);
    let failures = download_failures(downloader);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failed(format!(
            "Forge 支持库下载失败：{}",
            failures.join(", ")
        )))
    }
}

/// 解析 install_profile.json 的 `data` 段为处理器变量表。
///
/// 值格式：客户端使用 `client` 字段；以 `'` 开头的是字符串字面量；
/// 以 `/` 开头的是安装器压缩包内已解压到 `installer_dir` 的文件；
/// `[坐标]` 解析为 libraries 目录下的文件；其余 `{变量}` 引用其他键。
fn collect_data(
    profile: &Value,
    installer_dir: &Path,
    libraries_dir: &Path,
    installer_file: &Path,
    minecraft_jar: &Path,
) -> Result<HashMap<String, String>, crate::error::Error> {
    let mut variables = HashMap::from([
        ("SIDE".to_string(), "client".to_string()),
        // ROOT 等价于官方安装器的游戏根目录，取其下的 libraries 子目录
        // 就是传入的支持库目录，保证 `{ROOT}/libraries/...` 输出落在同一位置。
        (
            "ROOT".to_string(),
            libraries_dir
                .parent()
                .unwrap_or(libraries_dir)
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "INSTALLER".to_string(),
            installer_file.to_string_lossy().into_owned(),
        ),
        (
            "LIBRARY_DIR".to_string(),
            libraries_dir.to_string_lossy().into_owned(),
        ),
    ]);
    if let Some(data) = profile.get("data").and_then(Value::as_object) {
        for (key, entry) in data {
            let Some(raw) = entry
                .get("client")
                .and_then(Value::as_str)
                .or_else(|| entry.as_str())
            else {
                continue;
            };
            let value = if let Some(literal) = raw.strip_prefix('\'') {
                literal.to_string()
            } else if let Some(inner) = raw.strip_prefix('/') {
                installer_dir.join(inner).to_string_lossy().into_owned()
            } else if raw.starts_with('[') && raw.ends_with(']') {
                let coordinate = &raw[1..raw.len() - 1];
                libraries_dir
                    .join(maven_path(coordinate)?)
                    .to_string_lossy()
                    .into_owned()
            } else {
                expand_tokens(raw, &variables, false)?
            };
            variables.insert(key.clone(), value);
        }
    }
    // 无论清单如何声明，处理器的输入始终是刚下载的原版客户端 jar。
    variables.insert(
        "MINECRAFT_JAR".to_string(),
        minecraft_jar.to_string_lossy().into_owned(),
    );
    // data 值之间可以互相引用（如 BINPATCH 依赖 ROOT），迭代展开直到稳定。
    for _ in 0..8 {
        let snapshot = variables.clone();
        let mut changed = false;
        for value in variables.values_mut() {
            let expanded = expand_tokens(value, &snapshot, false)?;
            if expanded != *value {
                *value = expanded;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(variables)
}

/// 展开字符串中的 `{变量}` 占位符。
/// `strict` 为 true 时未知变量报错，否则原样保留等待下一轮展开。
fn expand_tokens(
    text: &str,
    variables: &HashMap<String, String>,
    strict: bool,
) -> Result<String, crate::error::Error> {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            return Err(failed(format!("未闭合的占位符：{text}")));
        };
        let key = &rest[start + 1..start + end];
        result.push_str(&rest[..start]);
        match variables.get(key) {
            Some(value) => result.push_str(value),
            None if strict => return Err(failed(format!("未知的安装变量：{key}"))),
            None => {
                result.push('{');
                result.push_str(key);
                result.push('}');
            }
        }
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

/// 展开一条处理器参数：`{变量}` 查表替换，`[坐标]` 解析为支持库文件路径。
fn expand_argument(
    argument: &str,
    variables: &HashMap<String, String>,
    libraries_dir: &Path,
) -> Result<String, crate::error::Error> {
    let text = expand_tokens(argument, variables, true)?;
    let mut result = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            return Err(failed(format!("未闭合的坐标引用：{argument}")));
        };
        let coordinate = &rest[start + 1..start + end];
        result.push_str(&rest[..start]);
        if coordinate.matches(':').count() >= 2
            && !coordinate.contains(['{', '}', '[', ']', '/', '\\', ' '])
        {
            result.push_str(
                &libraries_dir
                    .join(maven_path(coordinate)?)
                    .to_string_lossy(),
            );
        } else {
            result.push('[');
            result.push_str(coordinate);
            result.push(']');
        }
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

/// 按顺序执行 install_profile.json 声明的 processors。
///
/// 每个处理器是一个 Java 程序：`java -cp <classpath 列表 + jar> <mainClass> <args>`，
/// 只在 `sides` 包含 client（或未声明 sides）时执行。
fn run_processors(
    profile: &Value,
    java: &JavaVersion,
    libraries_dir: &Path,
    variables: &mut HashMap<String, String>,
) -> Result<(), crate::error::Error> {
    let Some(processors) = profile.get("processors").and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, processor) in processors.iter().enumerate() {
        if let Some(sides) = processor.get("sides").and_then(Value::as_array) {
            let client = sides
                .iter()
                .filter_map(Value::as_str)
                .any(|side| side == "client");
            if !client {
                continue;
            }
        }
        let jar = processor
            .get("jar")
            .and_then(Value::as_str)
            .ok_or_else(|| failed(format!("第 {} 个处理器缺少 jar", index + 1)))?;
        let jar = libraries_dir.join(maven_path(jar)?);
        if !jar.is_file() {
            return Err(failed(format!("处理器 jar 不存在：{}", jar.display())));
        }
        let mut classpath: Vec<PathBuf> = vec![jar.clone()];
        for entry in processor
            .get("classpath")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let entry = libraries_dir.join(maven_path(entry)?);
            if !classpath.contains(&entry) {
                classpath.push(entry);
            }
        }
        let classpath = std::env::join_paths(&classpath)
            .map_err(|e| failed(format!("无效的处理器 classpath：{e}")))?
            .into_string()
            .map_err(|_| failed("处理器 classpath 不是有效的 Unicode 路径"))?;
        // 清单一般不写 mainClass，主类从处理器 jar 的 MANIFEST.MF 里读，
        // 和官方安装器的行为一致。
        let main_class = match processor.get("mainClass").and_then(Value::as_str) {
            Some(main_class) => main_class.to_string(),
            None => jar_main_class(&jar)?,
        };
        let mut command = Command::new(&java.path_buf);
        command.arg("-cp").arg(classpath).arg(&main_class);
        let mut arguments = Vec::new();
        for value in processor
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            arguments.push(expand_argument(value, variables, libraries_dir)?);
        }
        let output = command.args(&arguments).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(failed(format!(
                "Forge 安装处理器 {main_class} 执行失败（{}）：{}",
                output.status,
                stderr.trim().lines().last().unwrap_or("")
            )));
        }
        check_outputs(processor, variables, libraries_dir)?;
    }
    Ok(())
}

/// 从 jar 的 META-INF/MANIFEST.MF 里读取 Main-Class 属性。
///
/// 清单属性值过长时会折行，续行以空格开头，先展开再逐行查找。
fn jar_main_class(jar: &Path) -> Result<String, crate::error::Error> {
    let mut zip = zip::ZipArchive::new(BufReader::new(File::open(jar)?))
        .map_err(|e| failed(format!("读取处理器 jar 失败：{e}")))?;
    let mut entry = zip
        .by_name("META-INF/MANIFEST.MF")
        .map_err(|_| failed(format!("处理器 {} 缺少 MANIFEST.MF", jar.display())))?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    let mut unfolded = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            unfolded.push_str(rest);
        } else {
            if !unfolded.is_empty() {
                unfolded.push('\n');
            }
            unfolded.push_str(line);
        }
    }
    unfolded
        .lines()
        .find_map(|line| {
            line.strip_prefix("Main-Class:")
                .map(|value| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| failed(format!("处理器 {} 缺少 Main-Class", jar.display())))
}

/// 校验处理器声明的 outputs：键是 `{变量}` 形式的输出路径占位符，
/// 值是期望的 SHA-1（通常是 `{XXX_SHA}` 变量或哈希字面量）。
/// 文件不存在或校验失败时报错；通过校验后把路径写回变量表，
/// 供后续处理器以 `{变量}` 引用。
fn check_outputs(
    processor: &Value,
    variables: &mut HashMap<String, String>,
    libraries_dir: &Path,
) -> Result<(), crate::error::Error> {
    let Some(outputs) = processor.get("outputs").and_then(Value::as_object) else {
        return Ok(());
    };
    for (key, expected) in outputs {
        let Some(expected) = expected.as_str() else {
            continue;
        };
        let output = expand_argument(key, variables, libraries_dir)?;
        let file = PathBuf::from(&output);
        if !file.is_file() || file.metadata()?.len() == 0 {
            return Err(failed(format!("处理器输出缺失：{key} = {output}")));
        }
        let expected = expand_argument(expected, variables, libraries_dir)?;
        // 40 位十六进制的期望值按 SHA-1 校验文件内容。
        if expected.len() == 40
            && expected.bytes().all(|b| b.is_ascii_hexdigit())
            && file_sha1(&file).is_ok_and(|hash| !hash.eq_ignore_ascii_case(&expected))
        {
            return Err(failed(format!("处理器输出校验失败：{key} = {output}")));
        }
        // 输出变量名去掉外层花括号，让后续处理器可以用 {VAR} 引用。
        let name = key
            .strip_prefix('{')
            .and_then(|key| key.strip_suffix('}'))
            .unwrap_or(key);
        variables.insert(name.to_string(), output);
    }
    Ok(())
}

/// 合并原版清单与 Forge 的 version.json：以原版为底，
/// Forge 的字段覆盖同名字段；libraries 按 name 合并，arguments 拼接，
/// 并去掉 `inheritsFrom` 使启动阶段可以直接读取。
fn merge_manifests(vanilla: Value, forge: &Value) -> Result<Value, crate::error::Error> {
    let mut merged = vanilla;
    let Some(object) = forge.as_object() else {
        return Err(failed("Forge version.json 不是对象"));
    };
    for (key, value) in object {
        match key.as_str() {
            "libraries" => merge_libraries(&mut merged, value),
            "arguments" => merge_arguments(&mut merged, value)?,
            "id" | "inheritsFrom" => {}
            _ => {
                merged[key] = value.clone();
            }
        }
    }
    Ok(merged)
}

/// Forge 声明的支持库追加到原版列表；坐标相同的条目用 Forge 版本替换。
fn merge_libraries(merged: &mut Value, forge: &Value) {
    let Some(forge_libraries) = forge.as_array() else {
        return;
    };
    let base = merged
        .as_object_mut()
        .expect("merged 是对象")
        .entry("libraries".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(base_libraries) = base.as_array_mut() else {
        return;
    };
    for library in forge_libraries {
        let name = library.get("name").and_then(Value::as_str);
        if let Some(name) = name
            && let Some(existing) = base_libraries
                .iter_mut()
                .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        {
            *existing = library.clone();
            continue;
        }
        base_libraries.push(library.clone());
    }
}

/// 拼接 arguments：jvm 段 Forge 在前（其 -p/-DignoreList 等参数必须生效），
/// game 段原版在前（Forge 只追加 --launchTarget 等加载器参数）。
fn merge_arguments(merged: &mut Value, forge: &Value) -> Result<(), crate::error::Error> {
    let Some(forge_arguments) = forge.as_object() else {
        return Ok(());
    };
    let base = merged
        .as_object_mut()
        .expect("merged 是对象")
        .entry("arguments".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(base_arguments) = base.as_object_mut() else {
        return Ok(());
    };
    for (key, value) in forge_arguments {
        let Some(additions) = value.as_array() else {
            base_arguments.insert(key.clone(), value.clone());
            continue;
        };
        let slot = base_arguments
            .entry(key.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(list) = slot.as_array_mut() else {
            *slot = value.clone();
            continue;
        };
        if key == "jvm" {
            let mut combined = additions.clone();
            combined.extend(list.drain(..));
            *list = combined;
        } else {
            list.extend(additions.iter().cloned());
        }
    }
    Ok(())
}

/// 把合并后的清单改造成启动器可识别的 Forge 版本 JSON：
/// `patches` 依次记录游戏版本（`game`）和 Forge 版本（`forge`），
/// 顶层 `id` 改为项目名，使启动时按 `<name>.jar` 查找运行文件。
fn prepare_forge_manifest(
    name: &str,
    forge_version: &str,
    manifest: Value,
) -> Result<Value, crate::error::Error> {
    let mut json = manifest;
    let game_version = json
        .get("inheritsFrom")
        .or_else(|| json.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| failed("版本 JSON 缺少游戏版本"))?
        .to_string();
    json["patches"] = serde_json::json!([
        {"id": "game", "version": game_version},
        {"id": "forge", "version": forge_version},
    ]);
    json["id"] = Value::String(name.to_string());
    Ok(json)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Atom)]
#[repr(u8)]
pub enum InstallState {
    Ready,
    DownloadInstaller,
    DownloadVersionJson,
    DownloadLibraries,
    InstallJar,
    RunProcessors,
    WriteVersionJson,
    Success,
    Failed,
}
