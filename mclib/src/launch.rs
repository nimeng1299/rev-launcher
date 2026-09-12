use crate::account::Account;
use crate::java::java_version::JavaVersion;
use crate::project::game_project::GameProject;
use crate::settings::GlobalSettings;
use atomig::{Atom, Atomic};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sharingan::downloader::{DownloadBuilder, Downloader};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

mod arguments;
mod libraries;
mod loaders;
mod logging;
pub use arguments::LaunchCommand;
use logging::LaunchLog;
pub use logging::{LaunchLogEntry, LaunchLogSource};

#[derive(Debug)]
pub enum LaunchError {
    AccountExpired,
    JavaCheckFailed(String),
    /// 没有找到适合的Java版本，并附带一个正确的Java版本号
    NotFindCorrectJava(i32),
    DownloadFailed(String),
    LaunchFailed(String),
    LogFailed(String),
    ProcessExited(Option<i32>),
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

    /// 检查已安装版本并构建启动命令，不启动 Java，也不写入游戏目录。
    pub fn launch_command(&self) -> Result<LaunchCommand, LaunchError> {
        let java = check_java(self)?;
        let json = self
            .project
            .read_json()
            .map_err(|e| LaunchError::LaunchFailed(e.to_string()))?;
        LaunchCommand::build(self, &json, &java)
    }

    /// 校验并复用已安装实例的支持库，提交剩余下载任务；不启动 Java。
    /// 返回后应等待下载器完成，并检查每项任务是否成功。
    pub fn download_libraries(&self) -> Result<Downloader, LaunchError> {
        libraries::download(self, None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Atom)]
#[repr(u8)]
pub enum LaunchState {
    Ready,
    CheckAccount,
    CheckJava,
    DownloadLibrary,
    DownloadMods,
    Launch,
    Failed,
    /// Java 已启动，正在运行并持续记录日志。
    Running,
    /// Java 正常退出；异常退出使用 Failed。
    Exited,
}

/// 启动进度的共享句柄，可克隆后传给其他线程。
#[derive(Clone)]
pub struct LaunchInfo {
    state: Arc<Atomic<LaunchState>>,
    error: Arc<Mutex<Option<Arc<LaunchError>>>>,
    data: Arc<Launcher>,
    libraries_downloader: Arc<OnceLock<Arc<Downloader>>>,
    mods_downloader: Arc<OnceLock<Arc<Downloader>>>,
    log: Arc<OnceLock<LaunchLog>>,
    process_id: Arc<OnceLock<u32>>,
    exit_status: Arc<OnceLock<ExitStatus>>,
    failed_state: Arc<OnceLock<LaunchState>>,
}

impl LaunchInfo {
    /// 在后台执行启动流程，立即返回可跨线程查询的句柄。
    pub fn launch(data: Launcher) -> Self {
        let info = Self {
            state: Arc::new(Atomic::new(LaunchState::Ready)),
            error: Arc::new(Mutex::new(None)),
            data: Arc::new(data),
            libraries_downloader: Arc::new(OnceLock::new()),
            mods_downloader: Arc::new(OnceLock::new()),
            log: Arc::new(OnceLock::new()),
            process_id: Arc::new(OnceLock::new()),
            exit_status: Arc::new(OnceLock::new()),
            failed_state: Arc::new(OnceLock::new()),
        };
        let launch = info.clone();

        let _handle = std::thread::spawn(move || {
            if let Err(error) = launch.run() {
                launch.failed_state.get_or_init(|| launch.state());
                if let Some(log) = launch.log.get() {
                    let _ = log.write(LaunchLogSource::Launcher, &format!("启动失败：{error:?}"));
                }
                *launch.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(error));
                launch.state.store(LaunchState::Failed, Ordering::SeqCst);
            }
        });

        info
    }

    fn run(&self) -> Result<(), LaunchError> {
        let log = LaunchLog::new(&self.data.project.path, &self.data.account.token)?;
        let log = self.log.get_or_init(|| log);
        let mut java = None;
        let mut previous = None;
        loop {
            let state = self.state();
            if previous != Some(state) {
                log.write(LaunchLogSource::Launcher, &format!("启动阶段：{state:?}"))?;
                previous = Some(state);
            }
            let next = match state {
                LaunchState::Ready => LaunchState::CheckAccount,
                LaunchState::CheckAccount => {
                    check_account(&self.data)?;
                    LaunchState::CheckJava
                }
                LaunchState::CheckJava => {
                    java = Some(check_java(&self.data)?);
                    LaunchState::DownloadLibrary
                }
                LaunchState::DownloadLibrary => {
                    if let Some(downloader) = self.libraries_downloader.get() {
                        if downloads_finished(downloader)? {
                            LaunchState::DownloadMods
                        } else {
                            state
                        }
                    } else {
                        let downloader = libraries::download(&self.data, Some(log))?;
                        self.libraries_downloader
                            .get_or_init(|| Arc::new(downloader));
                        state
                    }
                }
                LaunchState::DownloadMods => {
                    if let Some(downloader) = self.mods_downloader.get() {
                        if downloads_finished(downloader)? {
                            LaunchState::Launch
                        } else {
                            state
                        }
                    } else {
                        self.mods_downloader
                            .get_or_init(|| Arc::new(download_mods(&self.data)));
                        state
                    }
                }
                LaunchState::Launch => {
                    let java = java
                        .as_ref()
                        .ok_or_else(|| LaunchError::LaunchFailed("尚未选择 Java".into()))?;
                    let json = self
                        .data
                        .project
                        .read_json()
                        .map_err(|e| LaunchError::LaunchFailed(e.to_string()))?;
                    let command = LaunchCommand::build(&self.data, &json, java)?;
                    command.prepare_natives()?;
                    logging::run_process(self, &command)?;
                    LaunchState::Exited
                }
                LaunchState::Running | LaunchState::Exited | LaunchState::Failed => return Ok(()),
            };
            if next == LaunchState::Exited {
                log.write(LaunchLogSource::Launcher, "启动阶段：Exited")?;
                self.state.store(next, Ordering::SeqCst);
                return Ok(());
            }
            self.state.store(next, Ordering::SeqCst);
            if next == state {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// 保留旧接口，等价于 [`Self::launch`]。
    pub fn lunch(data: Launcher) -> Self {
        Self::launch(data)
    }

    /// 返回当前启动状态。
    pub fn state(&self) -> LaunchState {
        self.state.load(Ordering::SeqCst)
    }

    /// 获取失败原因；只有发生错误时才返回 Some。
    pub fn error(&self) -> Option<Arc<LaunchError>> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 返回发生错误时的步骤，即使界面错过了中间状态也能准确标记失败位置。
    pub fn failed_state(&self) -> Option<LaunchState> {
        self.failed_state.get().copied()
    }

    /// 本次启动的完整日志文件，初始化日志后即可查询。
    pub fn log_path(&self) -> Option<PathBuf> {
        self.log.get().map(|log| log.path.clone())
    }

    /// 最近 2,000 条日志的快照，可由界面线程轮询；完整日志保存在文件中。
    pub fn logs(&self) -> Vec<LaunchLogEntry> {
        self.log.get().map(LaunchLog::entries).unwrap_or_default()
    }

    pub fn process_id(&self) -> Option<u32> {
        self.process_id.get().copied()
    }

    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exit_status.get().copied()
    }

    /// 获取支持库下载器的共享句柄；尚未创建时返回 `None`，不会等待初始化。
    ///
    /// 下载器创建后会一直保留，可通过它查询任务和进度。
    pub fn libraries_downloader(&self) -> Option<Arc<Downloader>> {
        self.libraries_downloader.get().cloned()
    }

    /// 获取 mod 下载器的共享句柄；尚未创建时返回 `None`，不会等待初始化。
    ///
    /// 下载器创建后会一直保留，可通过它查询任务和进度。
    pub fn mods_downloader(&self) -> Option<Arc<Downloader>> {
        self.mods_downloader.get().cloned()
    }
}

fn downloads_finished(downloader: &Downloader) -> Result<bool, LaunchError> {
    if !downloader.is_finished() {
        return Ok(false);
    }
    let failures: Vec<_> = downloader
        .tasks()
        .values()
        .filter(|task| !task.status().is_success())
        .map(|task| task.filename().to_string())
        .collect();
    if failures.is_empty() {
        Ok(true)
    } else {
        Err(LaunchError::DownloadFailed(format!(
            "下载失败：{}",
            failures.join(", ")
        )))
    }
}

fn check_account(data: &Arc<Launcher>) -> Result<(), LaunchError> {
    let check = data.account.check_token();
    if check {
        Ok(())
    } else {
        Err(LaunchError::AccountExpired)
    }
}

fn check_java(data: &Launcher) -> Result<JavaVersion, LaunchError> {
    let value = data
        .project
        .read_json()
        .map_err(|e| LaunchError::JavaCheckFailed(e.to_string()))?;
    // 旧版 Minecraft 清单未声明 javaVersion，使用 Java 8。
    let major_version = match value.pointer("/javaVersion/majorVersion") {
        Some(version) => version
            .as_i64()
            .filter(|version| (1..=i32::MAX as i64).contains(version))
            .ok_or_else(|| {
                LaunchError::JavaCheckFailed("无效的 javaVersion.majorVersion".into())
            })?,
        None => 8,
    };

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
#[cfg(test)]
fn check_rules(json: &Value) -> bool {
    arguments::rules_allow(json, &arguments::RuleContext::current(false)).unwrap_or(false)
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
    use crate::java::java_version::JavaRuntime;
    use crate::project::game_project::ModLoader;
    use sharingan::status::TaskStatus;
    use std::path::PathBuf;
    use std::time::Instant;

    pub(super) fn write_jar(path: &std::path::Path) {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut jar = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        jar.start_file("test.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        jar.write_all(b"fixture jar").unwrap();
        jar.finish().unwrap();
    }

    pub(super) struct TestProject {
        pub(super) path: PathBuf,
        _temp: tempfile::TempDir,
    }

    impl TestProject {
        pub(super) fn new() -> Self {
            let temp = tempfile::Builder::new()
                .prefix("rev-launcher 中文 ")
                .tempdir()
                .unwrap();
            let path = temp.path().to_path_buf();
            std::fs::create_dir_all(path.join("instance")).unwrap();
            std::fs::write(
                path.join("instance/instance.json"),
                r#"{"javaVersion":{"majorVersion":21},"libraries":[]}"#,
            )
            .unwrap();
            Self { path, _temp: temp }
        }

        pub(super) fn launcher(&self) -> Launcher {
            Launcher::new(
                Account::new_offline("test"),
                GameProject {
                    name: "instance".to_string(),
                    version: String::new(),
                    path: self.path.join("instance"),
                    game_version: "1.21".to_string(),
                    loader: ModLoader::Minecraft,
                    loader_version: "1.21".to_string(),
                },
                Vec::new(),
                GlobalSettings {
                    java: Some(JavaVersion {
                        java_runtime: JavaRuntime::JRE,
                        version: "21".to_string(),
                        major_version: 21,
                        path_buf: PathBuf::new(),
                    }),
                    libraries_path: self.path.join("libraries"),
                    ..GlobalSettings::default()
                },
            )
        }
    }

    fn wait_for_launch(launch: &LaunchInfo) -> LaunchState {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = launch.state();
            if matches!(state, LaunchState::Exited | LaunchState::Failed) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "启动流程超时，当前状态：{state:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn launch_downloaders_are_shared_across_threads() {
        let project = TestProject::new();
        let launch = LaunchInfo::launch(project.launcher());
        let observer = launch.clone();
        let (libraries, mods) = std::thread::spawn(move || {
            // 此夹具没有 Java/客户端文件，验证启动失败后下载器仍能跨线程访问。
            assert!(matches!(wait_for_launch(&observer), LaunchState::Failed));
            (
                observer.libraries_downloader().unwrap(),
                observer.mods_downloader().unwrap(),
            )
        })
        .join()
        .unwrap();

        assert!(Arc::ptr_eq(
            &libraries,
            &launch.libraries_downloader().unwrap()
        ));
        assert!(Arc::ptr_eq(&mods, &launch.mods_downloader().unwrap()));
        assert!(!Arc::ptr_eq(&libraries, &mods));

        drop(launch);
        assert!(libraries.is_all_success());
        assert!(mods.is_all_success());
        assert_eq!(libraries.status(), TaskStatus::Running);
        assert_eq!(mods.status(), TaskStatus::Running);
    }

    #[test]
    fn launch_failure_does_not_create_downloaders() {
        let project = TestProject::new();
        std::fs::remove_file(project.path.join("instance/instance.json")).unwrap();
        let launch = LaunchInfo::lunch(project.launcher());

        assert!(matches!(wait_for_launch(&launch), LaunchState::Failed));
        assert!(launch.libraries_downloader().is_none());
        assert!(launch.mods_downloader().is_none());
        assert!(matches!(
            launch.error().as_deref(),
            Some(LaunchError::JavaCheckFailed(_))
        ));
        assert!(launch.log_path().unwrap().is_file());
        assert!(
            launch
                .logs()
                .iter()
                .any(|entry| entry.message.contains("启动失败"))
        );
        assert!(launch.process_id().is_none());
        assert_eq!(launch.failed_state(), Some(LaunchState::CheckJava));
    }

    #[test]
    fn launch_runs_selected_java_once_and_records_exit() {
        let project = TestProject::new();
        let mut data = project.launcher();
        let java_path = crate::java::find_java_paths()
            .into_iter()
            .next()
            .expect("测试需要 PATH 中可用的 Java");
        let java = JavaVersion::from_path(&java_path).unwrap();
        std::fs::write(project.path.join("instance/instance.jar"), []).unwrap();
        // -version 在 JVM 内直接退出，无需启动 Minecraft 或加载任何客户端代码。
        let json = serde_json::json!({"id":"instance", "javaVersion":{"majorVersion":java.major_version},
            "mainClass":"unused.Main", "libraries":[], "arguments":{"jvm":["-version"],"game":[]}});
        std::fs::write(
            project.path.join("instance/instance.json"),
            json.to_string(),
        )
        .unwrap();
        data.setting.java = None;
        data.setting.memory = Some(512);
        data.javas = vec![java];
        let launch = LaunchInfo::launch(data);
        assert_eq!(
            wait_for_launch(&launch),
            LaunchState::Exited,
            "{:?}",
            launch.error()
        );
        assert!(launch.error().is_none());
        assert!(launch.exit_status().unwrap().success());
        assert!(launch.process_id().is_some());
        let log = std::fs::read_to_string(launch.log_path().unwrap()).unwrap();
        assert_eq!(log.matches("启动 Java：").count(), 1);
        assert!(
            launch
                .logs()
                .iter()
                .any(|line| line.source == LaunchLogSource::Stderr)
        );
    }

    #[test]
    fn legacy_manifest_selects_java_eight() {
        let project = TestProject::new();
        std::fs::write(
            project.path.join("instance/instance.json"),
            r#"{"libraries":[],"minecraftArguments":"--username ${auth_player_name}"}"#,
        )
        .unwrap();
        let mut data = project.launcher();
        let mut java = data.setting.java.clone().unwrap();
        java.major_version = 8;
        data.javas.push(java);
        assert_eq!(check_java(&data).unwrap().major_version, 8);
        data.javas.clear();
        assert!(matches!(
            check_java(&data),
            Err(LaunchError::NotFindCorrectJava(8))
        ));
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
