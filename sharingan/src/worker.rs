//! 下载工作线程模块。
//!
//! [`Worker`] 运行在调度器启动的独立线程中，负责从就绪队列领取任务、
//! 执行 HTTP 下载并把结果写回共享容器。

use crate::status::{DownloadFailure, TaskStatus};
use crate::task::Task;
use atomig::Atomic;
use lockfree::map::Map;
use lockfree::queue::Queue;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use ureq::Agent;

/// 下载工作线程。
///
/// 每个 Worker 在调度器启动的一个线程中运行 [`run()`](Worker::run) 循环：
/// 从就绪队列取出任务并下载，直到下载器状态被置为「取消」。
///
/// 各共享容器（就绪队列、进行中任务表、已完成任务表、运行状态）由
/// [`Downloader`](crate::downloader::Downloader) 创建，同一个下载器下的
/// 所有 Worker 共享同一批容器。
pub struct Worker {
    /// 该 Worker 的序号（同时作为进行中任务表的键）。
    id: usize,
    /// 下载器运行状态（与其他 Worker 及调度器共享）。
    status: Arc<Atomic<TaskStatus>>,
    /// 就绪任务队列（共享）。
    ready_map: Arc<Queue<Arc<Task>>>,
    /// 进行中任务表：Worker 序号 -> 任务（共享）。
    progressing_map: Arc<Map<usize, Arc<Task>>>,
    /// 已完成任务表：任务 id -> 任务（共享）。
    finish_map: Arc<Map<usize, Arc<Task>>>,
    /// 当前正在下载的任务。
    ongoing_tasks: Option<Arc<Task>>,
}

impl Worker {
    /// 创建一个新的 Worker。
    ///
    /// `id` 为该 Worker 的序号；其余参数为与调度器及其他 Worker 共享的
    /// 运行状态与任务容器。
    pub fn new(
        id: usize,
        status: Arc<Atomic<TaskStatus>>,
        ready_map: Arc<Queue<Arc<Task>>>,
        progressing_map: Arc<Map<usize, Arc<Task>>>,
        finish_map: Arc<Map<usize, Arc<Task>>>,
    ) -> Self {
        Self {
            id,
            status,
            ready_map,
            progressing_map,
            finish_map,
            ongoing_tasks: None,
        }
    }

    /// 运行下载主循环，直到下载器状态变为「取消」。
    ///
    /// 每轮循环依次执行：
    /// 1. 检查取消标记：若已取消，则删除正在下载任务的临时文件、把任务放入
    ///    完成表并退出循环；
    /// 2. 若存在已领取的任务，先执行前置回调，成功且未取消时调用内部 `download`
    ///    完成下载并清理临时文件；无论成功或失败，都把任务移入完成表；
    /// 3. 否则尝试从就绪队列领取一个新任务；若队列为空则休眠 100ms，避免忙等。
    pub fn run(&mut self) {
        loop {
            if self.status.load(Ordering::SeqCst) == TaskStatus::Cancel {
                if let Some(task) = self.ongoing_tasks.take() {
                    task.change_failure(DownloadFailure::UserCancel);

                    let filename = task.path().join(task.temp_filename());
                    let _ = std::fs::remove_file(&filename);
                    self.progressing_map.remove(&self.id);
                    self.finish_map.insert(task.id(), task);
                }
                break;
            }

            if let Some(task) = self.ongoing_tasks.take() {
                if let Err(reason) = task.prepare() {
                    task.change_failure(reason);
                } else if self.status.load(Ordering::SeqCst) == TaskStatus::Cancel {
                    task.change_failure(DownloadFailure::UserCancel);
                } else {
                    task.change_downloading();
                    download(task.clone());
                    // 不管有没有成功都要清除缓存
                    let filename = task.path().join(task.temp_filename());
                    let _ = std::fs::remove_file(&filename);
                }

                self.finish_map.insert(task.id(), task);
                self.progressing_map.remove(&self.id);
            } else {
                if let Some(new_task) = self.ready_map.pop() {
                    new_task.change_preparing();
                    self.progressing_map.insert(self.id, new_task.clone());
                    self.ongoing_tasks = Some(new_task);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// 实际执行一次 HTTP 下载，并更新任务状态与进度。
///
/// 流程：发起 GET 请求 → 校验 HTTP 状态码 → 解析 `Content-Length` 更新总大小 →
/// 创建保存目录与临时文件 → 边读边写、按读取间隔估算速度并更新进度 →
/// `sync_all` 落盘 → 校验临时文件 → 按需删除旧文件 → 把临时文件重命名为最终文件名。
///
/// 任意一步失败都会通过 `Task::change_failure` 记录失败原因；
/// 全部成功则通过 `Task::change_success` 标记任务完成。
fn download(task: Arc<Task>) {
    let agent = Agent::config_builder()
        .timeout_global(task.timeout().clone())
        .build()
        .new_agent();

    let mut request = agent.get(task.url());

    for (key, value) in task.query() {
        request = request.query(key, value);
    }

    for (key, value) in task.header() {
        request = request.header(key, value);
    }

    let response = match request.call() {
        Ok(response) => response,
        Err(e) => {
            task.change_failure(DownloadFailure::NetworkError(e));
            return;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        task.change_failure(DownloadFailure::HttpStatusCode(status));
        return;
    }

    let total_size = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    task.progress().change_total(total_size);

    if let Err(e) = std::fs::create_dir_all(task.path()) {
        task.change_failure(DownloadFailure::IOError(e));
        return;
    }
    let temp_filename = task.path().join(task.temp_filename());
    let mut file = match File::create(temp_filename) {
        Ok(file) => file,
        Err(e) => {
            task.change_failure(DownloadFailure::IOError(e));
            return;
        }
    };
    let mut reader = response.into_body().into_reader();
    let mut buffer = [0; 8192];
    let mut downloaded: u64 = 0;

    loop {
        let start = Instant::now();

        let bytes_read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(e) => {
                task.change_failure(DownloadFailure::IOError(e));
                return;
            }
        };
        if bytes_read == 0 {
            break;
        }
        match file.write_all(&buffer[..bytes_read]) {
            Ok(_) => {}
            Err(e) => {
                task.change_failure(DownloadFailure::IOError(e));
                return;
            }
        };
        downloaded += bytes_read as u64;

        let end = start.elapsed().as_millis() as f64;
        let speed = if end >= 10f64 {
            //防止间隔太短速度为+inf
            bytes_read as f64 / (end / 1000.0)
        } else {
            task.progress().speed()
        };
        task.progress().update(downloaded, speed);
    }

    match file.sync_all() {
        Ok(_) => {
            let temp_filename = task.path().join(task.temp_filename());
            if let Some(validator) = task.validator() {
                if let Err(reason) = validator(temp_filename.clone()) {
                    task.change_failure(reason);
                    let _ = std::fs::remove_file(temp_filename);
                    return;
                }
            }

            if task.overwrite() {
                let _ = std::fs::remove_file(&task.path().join(task.filename()));
            }

            match std::fs::rename(temp_filename, task.path().join(task.filename())) {
                Ok(_) => {
                    task.change_success();
                }
                Err(e) => {
                    task.change_failure(DownloadFailure::IOError(e));
                }
            }
        }
        Err(e) => {
            task.change_failure(DownloadFailure::IOError(e));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::DownloadBuilder;
    use crate::status::DownloadStatus;
    use crate::task::TaskBuilder;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn wait_until_finished(downloader: &crate::downloader::Downloader) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !downloader.is_finished()
            || downloader.finish_map().iter().count() != downloader.tasks().len()
        {
            assert!(Instant::now() < deadline, "等待任务结束超时");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn serve_file() -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/resolved", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "等待本地下载请求超时");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest")
                .unwrap();
            request
        });
        (url, server)
    }

    #[test]
    fn validation_errors_preserve_original_and_clean_temporary_file() {
        for io_error in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let target = directory.path().join("file.bin");
            std::fs::write(&target, b"original").unwrap();
            let (url, server) = serve_file();
            let path = directory.path().to_path_buf();
            let mut downloader = DownloadBuilder::new().build();
            downloader.download(move |builder| {
                builder
                    .url(url)
                    .path(path)
                    .filename("file.bin".into())
                    .overwrite(true)
                    .timeout(Some(Duration::from_secs(5)))
                    .validator(move |path| {
                        assert_eq!(std::fs::read(path).unwrap(), b"test");
                        if io_error {
                            Err(DownloadFailure::IOError(std::io::Error::other(
                                "validator error",
                            )))
                        } else {
                            Err(DownloadFailure::ValidationError)
                        }
                    })
                    .build()
            });
            wait_until_finished(&downloader);
            server.join().unwrap();
            let task = &downloader.tasks()[&0];
            assert!(task.status().is_failed());
            match &*task.failed_reason() {
                DownloadFailure::IOError(error) if io_error => {
                    assert_eq!(error.to_string(), "validator error");
                }
                DownloadFailure::ValidationError if !io_error => {}
                reason => panic!("校验错误未保留: {reason:?}"),
            }
            assert_eq!(std::fs::read(target).unwrap(), b"original");
            assert!(!task.path().join(task.temp_filename()).exists());
        }
    }

    #[test]
    fn preparation_resolves_download_options_in_worker() {
        let directory = tempfile::tempdir().unwrap();
        let (url, server) = serve_file();
        let caller = std::thread::current().id();
        let path = directory.path().join("resolved");
        let expected_url = url.clone();
        let expected_path = path.clone();
        let (validated_tx, validated_rx) = mpsc::channel();
        let mut downloader = DownloadBuilder::new().thread_num(1).build();
        downloader.download(move |builder| {
            builder
                .overwrite(true)
                .timeout(Some(Duration::from_secs(5)))
                .before_download(move |mut options| {
                    assert_ne!(std::thread::current().id(), caller);
                    assert!(options.overwrite);
                    assert_eq!(options.timeout, Some(Duration::from_secs(5)));
                    options.url = url;
                    options.path = path;
                    options.filename = "dynamic.bin".into();
                    options.query.push(("token".into(), "resolved".into()));
                    options.header.push(("x-prepared".into(), "yes".into()));
                    options.validator = Some(Arc::new(move |path| {
                        validated_tx.send(()).unwrap();
                        let content = std::fs::read(path).map_err(DownloadFailure::IOError)?;
                        if content != b"test" {
                            return Err(DownloadFailure::ValidationError);
                        }
                        Ok(())
                    }));
                    Ok(options)
                })
                .build()
        });
        let task = downloader.tasks()[&0].clone();
        wait_until_finished(&downloader);
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /resolved?token=resolved http/1.1\r\n"));
        assert!(request.contains("x-prepared: yes\r\n"));
        assert!(downloader.is_all_success());
        assert!(Arc::ptr_eq(&task, &downloader.tasks()[&0]));
        assert_eq!(task.id(), 0);
        assert_eq!(task.url(), expected_url);
        assert_eq!(task.path(), &expected_path);
        assert_eq!(task.filename(), "dynamic.bin");
        assert_eq!(task.progress().downloaded(), 4);
        assert_eq!(
            std::fs::read(expected_path.join("dynamic.bin")).unwrap(),
            b"test"
        );
        assert!(!expected_path.join(task.temp_filename()).exists());
        validated_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn failed_preparation_does_not_request_or_create_download_files() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/unused", listener.local_addr().unwrap());
        let path = directory.path().join("unused");
        let expected_path = path.clone();
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(move |builder| {
            builder
                .url(url)
                .path(path)
                .filename("unused.bin".into())
                .before_download(|_| {
                    Err(DownloadFailure::PreparationError("metadata failed".into()))
                })
                .build()
        });
        wait_until_finished(&downloader);
        let task = &downloader.tasks()[&0];
        assert!(task.status().is_failed());
        assert!(!downloader.is_all_success());
        assert!(
            matches!(&*task.failed_reason(), DownloadFailure::PreparationError(reason) if reason == "metadata failed")
        );
        assert_eq!(task.progress().downloaded(), 0);
        assert!(!expected_path.exists());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn preparations_run_concurrently_with_worker_limit() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let mut releases = Vec::new();
        let mut downloader = DownloadBuilder::new().thread_num(2).build();
        for id in 0..3 {
            let entered_tx = entered_tx.clone();
            let (release_tx, release_rx) = mpsc::channel();
            releases.push(release_tx);
            downloader.download(move |builder| {
                builder
                    .before_download(move |_| {
                        entered_tx.send((id, std::thread::current().id())).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        Err(DownloadFailure::PreparationError(format!("task {id}")))
                    })
                    .build()
            });
        }
        drop(entered_tx);
        let first = entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(first.0, second.0);
        assert_ne!(first.1, second.1);
        assert_ne!(first.1, std::thread::current().id());
        assert_ne!(second.1, std::thread::current().id());
        assert!(!downloader.is_finished());
        for id in [first.0, second.0] {
            assert!(matches!(
                downloader.tasks()[&id].status(),
                DownloadStatus::Preparing
            ));
        }
        assert!(matches!(
            entered_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        releases[first.0].send(()).unwrap();
        releases[second.0].send(()).unwrap();
        let third = entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(third.0, first.0);
        assert_ne!(third.0, second.0);
        releases[third.0].send(()).unwrap();
        wait_until_finished(&downloader);
        assert!(
            downloader
                .tasks()
                .values()
                .all(|task| task.status().is_failed())
        );
        assert!(matches!(
            entered_rx.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn cancellation_during_preparation_skips_download() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/cancelled", listener.local_addr().unwrap());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(move |builder| {
            builder
                .before_download(move |mut options| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    options.url = url;
                    Ok(options)
                })
                .build()
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        downloader.cancel();
        release_tx.send(()).unwrap();
        wait_until_finished(&downloader);
        let task = &downloader.tasks()[&0];
        assert!(task.status().is_failed());
        assert!(matches!(*task.failed_reason(), DownloadFailure::UserCancel));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn active_download_reports_downloading_until_body_completes() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "等待本地下载请求超时");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                assert!(request.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
                .unwrap();
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            stream.write_all(b"test").unwrap();
        });
        let path = directory.path().to_path_buf();
        let mut downloader = DownloadBuilder::new().thread_num(1).build();
        downloader.download(move |builder| {
            builder
                .url(url.clone())
                .path(path.clone())
                .filename("file.bin".into())
                .timeout(Some(Duration::from_secs(5)))
                .build()
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            downloader.tasks()[&0].status(),
            DownloadStatus::Downloading
        ));
        assert!(!downloader.is_finished());
        release_tx.send(()).unwrap();
        server.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !downloader.is_finished() {
            assert!(Instant::now() < deadline, "下载结束超时");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(downloader.is_all_success());
        assert_eq!(downloader.tasks()[&0].progress().downloaded(), 4);
        assert_eq!(downloader.tasks()[&0].progress().total(), Some(4));
        assert_eq!(
            std::fs::read(directory.path().join("file.bin")).unwrap(),
            b"test"
        );
        downloader.cancel();
    }

    #[test]
    fn cancelled_claimed_task_is_finished_instead_of_downloading() {
        let directory = tempfile::tempdir().unwrap();
        let task = Arc::new(
            TaskBuilder::new(0)
                .path(directory.path().to_path_buf())
                .filename("cancelled.bin".into())
                .build(),
        );
        task.change_downloading();
        let mut worker = Worker::new(
            0,
            Arc::new(Atomic::new(TaskStatus::Cancel)),
            Arc::new(Queue::default()),
            Arc::new(Map::default()),
            Arc::new(Map::default()),
        );
        worker.ongoing_tasks = Some(task.clone());
        worker.run();
        assert!(task.status().is_failed());
        assert!(task.status().is_finished());
    }
}
