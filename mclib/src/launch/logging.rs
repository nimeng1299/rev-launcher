use super::{LaunchCommand, LaunchError, LaunchInfo, LaunchState};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchLogSource {
    Launcher,
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub struct LaunchLogEntry {
    pub time: SystemTime,
    pub source: LaunchLogSource,
    pub message: String,
}

struct LogData {
    file: File,
    recent: VecDeque<LaunchLogEntry>,
}

pub(super) struct LaunchLog {
    pub path: PathBuf,
    token: String,
    data: Mutex<LogData>,
}

impl LaunchLog {
    pub fn new(game_directory: &Path, token: &str) -> Result<Self, LaunchError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let directory = std::path::absolute(game_directory)
            .map_err(|e| LaunchError::LogFailed(e.to_string()))?
            .join(".rev_launcher/logs");
        std::fs::create_dir_all(&directory).map_err(|e| LaunchError::LogFailed(e.to_string()))?;
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = directory.join(format!(
            "launch-{time}-{}-{}.log",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|e| LaunchError::LogFailed(e.to_string()))?;
        Ok(Self {
            path,
            token: token.into(),
            data: Mutex::new(LogData {
                file,
                recent: VecDeque::new(),
            }),
        })
    }

    pub fn redact(&self, message: &str) -> String {
        if self.token.is_empty() {
            message.into()
        } else {
            message.replace(&self.token, "<redacted>")
        }
    }

    pub fn write(&self, source: LaunchLogSource, message: &str) -> Result<(), LaunchError> {
        let time = SystemTime::now();
        let timestamp = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let message = self.redact(message);
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(
            data.file,
            "[{}.{:03}] [{source:?}] {message}",
            timestamp.as_secs(),
            timestamp.subsec_millis()
        )
        .and_then(|_| data.file.flush())
        .map_err(|e| LaunchError::LogFailed(e.to_string()))?;
        // 文件保留完整日志，供界面轮询的内存快照只保留最近 2,000 条。
        if data.recent.len() == 2000 {
            data.recent.pop_front();
        }
        data.recent.push_back(LaunchLogEntry {
            time,
            source,
            message,
        });
        Ok(())
    }

    pub fn entries(&self) -> Vec<LaunchLogEntry> {
        self.data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recent
            .iter()
            .cloned()
            .collect()
    }
}

fn copy_output(
    reader: impl Read,
    log: &LaunchLog,
    source: LaunchLogSource,
) -> Result<(), LaunchError> {
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    let mut log_error = None;
    loop {
        bytes.clear();
        let count = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|e| LaunchError::LogFailed(format!("读取 {source:?} 失败：{e}")))?;
        if count == 0 {
            break;
        }
        let message = String::from_utf8_lossy(&bytes);
        if let Err(error) = log.write(source, message.trim_end_matches(['\r', '\n'])) {
            // 即使磁盘写入失败也继续消费管道，避免 Java 因管道写满而无法退出。
            log_error.get_or_insert(error);
        }
    }
    match log_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(super) fn run_process(info: &LaunchInfo, launch: &LaunchCommand) -> Result<(), LaunchError> {
    let log = info
        .log
        .get()
        .ok_or_else(|| LaunchError::LogFailed("启动日志尚未初始化".into()))?;
    log.write(
        LaunchLogSource::Launcher,
        &format!("工作目录：{}", launch.working_directory.display()),
    )?;
    // 在格式化之前脱敏，避免 Debug 转义后留下原始凭据。
    let arguments: Vec<_> = launch.arguments.iter().map(|arg| log.redact(arg)).collect();
    log.write(
        LaunchLogSource::Launcher,
        &format!("启动 Java：{:?}\n参数：{arguments:?}", launch.java),
    )?;
    let mut command = Command::new(&launch.java);
    command
        .args(&launch.arguments)
        .current_dir(&launch.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 仅隐藏 java.exe 的控制台，不影响游戏自己的窗口。
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| {
        LaunchError::LaunchFailed(format!("无法启动 {}：{e}", launch.java.display()))
    })?;
    info.process_id.get_or_init(|| child.id());
    info.state.store(LaunchState::Running, Ordering::SeqCst);
    let started_log = log.write(
        LaunchLogSource::Launcher,
        &format!("Java 已启动，PID：{}，状态：Running", child.id()),
    );
    let stdout = child.stdout.take().expect("stdout 已配置为管道");
    let stderr = child.stderr.take().expect("stderr 已配置为管道");
    let (status, stdout_result, stderr_result) = std::thread::scope(|scope| {
        let out = scope.spawn(|| copy_output(stdout, log, LaunchLogSource::Stdout));
        let err = scope.spawn(|| copy_output(stderr, log, LaunchLogSource::Stderr));
        let status = child.wait();
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        (status, out.join(), err.join())
    });
    let status =
        status.map_err(|e| LaunchError::LaunchFailed(format!("等待 Java 退出失败：{e}")))?;
    info.exit_status.get_or_init(|| status);
    started_log?;
    stdout_result.map_err(|_| LaunchError::LogFailed("stdout 日志线程异常退出".into()))??;
    stderr_result.map_err(|_| LaunchError::LogFailed("stderr 日志线程异常退出".into()))??;
    log.write(
        LaunchLogSource::Launcher,
        &format!("Java 进程 {} 已退出：{status}", child.id()),
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(LaunchError::ProcessExited(status.code()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::tests::TestProject;
    use atomig::Atomic;
    use serde_json::json;
    use std::sync::{Arc, OnceLock};

    fn fixture(helper: &str) -> (TestProject, LaunchInfo, LaunchCommand) {
        let project = TestProject::new();
        let mut data = project.launcher();
        data.account.token = "test-secret-token".into();
        data.setting.java.as_mut().unwrap().path_buf = std::env::current_exe().unwrap();
        data.setting.memory = Some(512);
        std::fs::write(project.path.join("instance/instance.jar"), []).unwrap();
        let mut command = LaunchCommand::build(&data, &json!({"id":"instance", "libraries":[], "mainClass":"unused", "arguments":{"game":[]}}), data.setting.java.as_ref().unwrap()).unwrap();
        command.arguments = vec![
            "--exact".into(),
            format!("launch::logging::tests::{helper}"),
            "--ignored".into(),
            "--nocapture".into(),
        ];
        let log = LaunchLog::new(&data.project.path, &data.account.token).unwrap();
        let info = LaunchInfo {
            state: Arc::new(Atomic::new(LaunchState::Launch)),
            error: Arc::new(Mutex::new(None)),
            data: Arc::new(data),
            libraries_downloader: Arc::new(OnceLock::new()),
            mods_downloader: Arc::new(OnceLock::new()),
            log: Arc::new(OnceLock::new()),
            process_id: Arc::new(OnceLock::new()),
            exit_status: Arc::new(OnceLock::new()),
            failed_state: Arc::new(OnceLock::new()),
        };
        info.log.get_or_init(|| log);
        (project, info, command)
    }

    #[test]
    fn process_drains_both_pipes_redacts_and_bounds_memory() {
        let (_project, info, command) = fixture("emit_output");
        run_process(&info, &command).unwrap();
        assert!(info.process_id().is_some());
        assert!(info.exit_status().unwrap().success());
        let log = std::fs::read_to_string(info.log_path().unwrap()).unwrap();
        assert!(log.contains("STDOUT-END") && log.contains("STDERR-END"));
        assert!(!log.contains("test-secret-token"));
        assert!(log.contains("<redacted>"));
        assert_eq!(log.matches("OUTPUT-LINE").count(), 6000);
        assert_eq!(info.logs().len(), 2000);
        assert!(
            info.logs()
                .iter()
                .all(|line| !line.message.contains("test-secret-token"))
        );
    }

    #[test]
    fn process_reports_nonzero_exit_and_spawn_errors() {
        let (_project, info, mut command) = fixture("exit_unsuccessfully");
        assert!(matches!(
            run_process(&info, &command),
            Err(LaunchError::ProcessExited(Some(7)))
        ));
        assert_eq!(info.exit_status().unwrap().code(), Some(7));
        assert!(
            info.logs()
                .iter()
                .any(|line| line.source == LaunchLogSource::Stderr
                    && line.message.contains("failure"))
        );
        command.java = command.working_directory.join("missing-java-executable");
        assert!(matches!(
            run_process(&info, &command),
            Err(LaunchError::LaunchFailed(_))
        ));
    }

    #[test]
    fn non_utf8_output_is_retained_lossily() {
        let project = TestProject::new();
        let log = LaunchLog::new(&project.path, "").unwrap();
        copy_output(
            &b"first\ninvalid-\xff\nlast"[..],
            &log,
            LaunchLogSource::Stdout,
        )
        .unwrap();
        assert_eq!(log.entries().len(), 3);
        assert_eq!(log.entries()[1].message, "invalid-\u{fffd}");
        assert_eq!(log.entries()[2].message, "last");
    }

    #[test]
    #[ignore = "仅由 process_drains_both_pipes_redacts_and_bounds_memory 的子进程调用"]
    fn emit_output() {
        for _ in 0..3000 {
            println!("OUTPUT-LINE {} test-secret-token", "x".repeat(128));
            eprintln!("OUTPUT-LINE {} test-secret-token", "y".repeat(128));
        }
        println!("STDOUT-END");
        eprintln!("STDERR-END");
    }

    #[test]
    #[ignore = "仅由 process_reports_nonzero_exit_and_spawn_errors 的子进程调用"]
    fn exit_unsuccessfully() {
        eprintln!("expected failure");
        std::process::exit(7);
    }
}
