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
    /// 2. 若存在已领取的任务，先执行前置回调，成功且未取消、未要求跳过时调用
    ///    内部 `download` 完成下载并清理临时文件；随后把任务移入完成表；
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
                match download_task(&task, &self.status) {
                    Ok(()) => task.change_success(),
                    Err(reason) => task.change_failure(reason),
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

/// 一个任务的前置处理、下载和按顺序尝试的备用配置。
/// 仅在整个流程结束后发布成功或失败，避免调用方在重试前误判任务已完成。
fn download_task(task: &Task, status: &Atomic<TaskStatus>) -> Result<(), DownloadFailure> {
    task.prepare()?;
    let mut fallbacks = task.options().fallbacks.iter().enumerate();
    loop {
        if status.load(Ordering::SeqCst) == TaskStatus::Cancel {
            return Err(DownloadFailure::UserCancel);
        }
        if task.options().skip_download {
            return Ok(());
        }
        task.change_downloading();
        let result = download(task);
        let _ = std::fs::remove_file(task.path().join(task.temp_filename()));
        let Err(mut reason) = result else {
            return Ok(());
        };
        loop {
            if status.load(Ordering::SeqCst) == TaskStatus::Cancel {
                return Err(DownloadFailure::UserCancel);
            }
            let Some((index, fallback)) = fallbacks.next() else {
                return Err(reason);
            };
            match task.prepare_fallback(index, fallback) {
                Ok(()) => break,
                Err(error) => reason = error,
            }
        }
    }
}

/// 执行一次下载并更新进度；校验通过后才覆盖最终文件。
fn download(task: &Task) -> Result<(), DownloadFailure> {
    let agent = Agent::config_builder()
        .timeout_global(*task.timeout())
        .build()
        .new_agent();
    let mut request = agent.get(task.url());
    for (key, value) in task.query() {
        request = request.query(key, value);
    }
    for (key, value) in task.header() {
        request = request.header(key, value);
    }
    let response = request.call().map_err(DownloadFailure::NetworkError)?;
    if !response.status().is_success() {
        return Err(DownloadFailure::HttpStatusCode(response.status()));
    }
    let total_size = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    task.progress().change_total(total_size);
    std::fs::create_dir_all(task.path()).map_err(DownloadFailure::IOError)?;
    let temp_filename = task.path().join(task.temp_filename());
    let mut file = File::create(&temp_filename).map_err(DownloadFailure::IOError)?;
    let mut reader = response.into_body().into_reader();
    let mut buffer = [0; 8192];
    let mut downloaded: u64 = 0;
    loop {
        let start = Instant::now();
        let bytes_read = reader.read(&mut buffer).map_err(DownloadFailure::IOError)?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])
            .map_err(DownloadFailure::IOError)?;
        downloaded += bytes_read as u64;
        let elapsed = start.elapsed().as_millis() as f64;
        let speed = if elapsed >= 10.0 {
            bytes_read as f64 / (elapsed / 1000.0)
        } else {
            task.progress().speed()
        };
        task.progress().update(downloaded, speed);
    }
    file.sync_all().map_err(DownloadFailure::IOError)?;
    drop(file);
    if let Some(validator) = task.validator() {
        validator(temp_filename.clone())?;
    }
    if task.overwrite() {
        let _ = std::fs::remove_file(task.path().join(task.filename()));
    }
    std::fs::rename(temp_filename, task.path().join(task.filename()))
        .map_err(DownloadFailure::IOError)
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

    #[test]
    fn preparation_can_skip_download_without_touching_files() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("cached.bin");
        std::fs::write(&target, b"cached").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/unused", listener.local_addr().unwrap());
        let path = directory.path().to_path_buf();
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(move |builder| {
            builder
                .url(url)
                .path(path)
                .filename("cached.bin".into())
                .overwrite(true)
                .before_download(|mut options| {
                    options.skip_download = true;
                    Ok(options)
                })
                .validator(|_| panic!("跳过下载时不应执行下载后校验"))
                .build()
        });
        wait_until_finished(&downloader);
        assert!(downloader.is_all_success());
        let task = &downloader.tasks()[&0];
        assert_eq!(task.progress().downloaded(), 0);
        assert!(!task.path().join(task.temp_filename()).exists());
        assert_eq!(std::fs::read(target).unwrap(), b"cached");
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
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
    fn fallback_keeps_task_pending_until_the_second_download_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first");
        let second_path = directory.path().join("second");
        let expected_path = second_path.clone();
        let (first_url, first_server) = serve_file();
        let (second_url, second_server) = serve_file();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(move |builder| {
            builder
                .url(first_url)
                .path(first_path)
                .filename("mod.jar".into())
                .timeout(Some(Duration::from_secs(5)))
                .validator(|_| Err(DownloadFailure::ValidationError))
                .before_download(move |mut options| {
                    options.add_fallback(move |mut options| {
                        entered_tx.send(()).unwrap();
                        release_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        options.url = second_url.clone();
                        options.path = second_path.clone();
                        options.validator = Some(Arc::new(|path| {
                            assert_eq!(std::fs::read(path).unwrap(), b"test");
                            Ok(())
                        }));
                        Ok(options)
                    });
                    Ok(options)
                })
                .build()
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let task = downloader.tasks()[&0].clone();
        assert!(matches!(task.status(), DownloadStatus::Preparing));
        assert!(!downloader.is_finished());
        assert_eq!(downloader.finish_map().iter().count(), 0);
        assert!(!task.path().join(task.temp_filename()).exists());
        release_tx.send(()).unwrap();
        wait_until_finished(&downloader);
        first_server.join().unwrap();
        second_server.join().unwrap();
        assert!(downloader.is_all_success());
        assert_eq!(downloader.tasks().len(), 1);
        assert!(Arc::ptr_eq(&task, &downloader.tasks()[&0]));
        assert_eq!(task.path(), &expected_path);
        assert_eq!(task.progress().downloaded(), 4);
        assert_eq!(task.progress().total(), Some(4));
        assert_eq!(
            std::fs::read(expected_path.join("mod.jar")).unwrap(),
            b"test"
        );
        assert!(!task.path().join(task.temp_filename()).exists());
    }

    #[test]
    fn multiple_fallbacks_continue_after_errors_and_stop_on_success() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        let (first_url, first_server) = serve_file();
        let (final_url, final_server) = serve_file();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut downloader = DownloadBuilder::new().build();
        let order_in_callbacks = order.clone();
        downloader.download(move |builder| {
            let first_order = order_in_callbacks.clone();
            let second_order = order_in_callbacks.clone();
            builder
                .url("invalid-primary".into())
                .path(path)
                .filename("mod.jar".into())
                .timeout(Some(Duration::from_secs(5)))
                .add_fallback(move |_| {
                    first_order.lock().unwrap().push(1);
                    Err(DownloadFailure::PreparationError(
                        "mirror unavailable".into(),
                    ))
                })
                .add_fallback(move |mut options| {
                    second_order.lock().unwrap().push(2);
                    options.url = first_url.clone();
                    options.validator = Some(Arc::new(|_| Err(DownloadFailure::ValidationError)));
                    Ok(options)
                })
                .before_download(move |mut options| {
                    // 动态追加的配置位于 Builder 已添加的配置之后。
                    options.add_fallback(move |mut options| {
                        order_in_callbacks.lock().unwrap().push(3);
                        started_tx.send(()).unwrap();
                        release_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        options.url = final_url.clone();
                        options.filename = "final.jar".into();
                        options.validator = None;
                        Ok(options)
                    });
                    options.add_fallback(|_| panic!("成功后不应尝试剩余配置"));
                    Ok(options)
                })
                .build()
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let task = downloader.tasks()[&0].clone();
        // 保留第二个备用配置的引用，下一次发布不会使旧引用失效。
        let previous = task.options();
        let previous_url = previous.url.clone();
        assert_eq!(previous.filename, "mod.jar");
        assert_eq!(task.progress().downloaded(), 4);
        assert!(!downloader.is_finished());
        assert!(matches!(task.status(), DownloadStatus::Preparing));
        assert!(!task.path().join(task.temp_filename()).exists());
        release_tx.send(()).unwrap();
        wait_until_finished(&downloader);
        first_server.join().unwrap();
        final_server.join().unwrap();
        assert_eq!(*order.lock().unwrap(), [1, 2, 3]);
        assert!(downloader.is_all_success());
        assert_eq!(downloader.tasks().len(), 1);
        assert_eq!(task.filename(), "final.jar");
        assert_eq!(task.progress().downloaded(), 4);
        assert_eq!(previous.url, previous_url);
        assert_eq!(previous.filename, "mod.jar");
        assert_eq!(
            std::fs::read(directory.path().join("final.jar")).unwrap(),
            b"test"
        );
        assert!(!directory.path().join("mod.jar").exists());
    }

    #[test]
    fn many_fallbacks_are_consumed_in_order_and_report_the_last_error() {
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let order_in_callbacks = order.clone();
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(move |mut builder| {
            for index in 0..128 {
                let order = order_in_callbacks.clone();
                builder = builder.add_fallback(move |mut options| {
                    order.lock().unwrap().push(index);
                    if index % 2 == 0 {
                        options.url = "invalid-mirror".into();
                        Ok(options)
                    } else {
                        Err(DownloadFailure::PreparationError(format!("mirror {index}")))
                    }
                });
            }
            builder.url("invalid-primary".into()).build()
        });
        wait_until_finished(&downloader);
        assert_eq!(*order.lock().unwrap(), (0..128).collect::<Vec<_>>());
        let task = &downloader.tasks()[&0];
        assert!(task.status().is_failed());
        assert!(
            matches!(&*task.failed_reason(), DownloadFailure::PreparationError(message)
            if message == "mirror 127")
        );
    }

    #[test]
    fn fallback_skip_stops_remaining_options() {
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(|builder| {
            builder
                .url("invalid-primary".into())
                .add_fallback(|_| Err(DownloadFailure::PreparationError("unavailable".into())))
                .add_fallback(|mut options| {
                    options.skip_download = true;
                    Ok(options)
                })
                .add_fallback(|_| panic!("跳过后不应尝试剩余配置"))
                .build()
        });
        wait_until_finished(&downloader);
        assert!(downloader.is_all_success());
        assert!(downloader.tasks()[&0].options().skip_download);
        assert_eq!(downloader.tasks()[&0].progress().downloaded(), 0);
    }

    #[test]
    fn fallback_runs_once_and_records_its_final_error() {
        for preparation_error in [false, true] {
            let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let attempts_in_callback = attempts.clone();
            let mut downloader = DownloadBuilder::new().build();
            downloader.download(move |builder| {
                builder
                    .url("invalid-url".into())
                    .before_download(move |mut options| {
                        options.add_fallback(move |mut options| {
                            attempts_in_callback.fetch_add(1, Ordering::SeqCst);
                            if preparation_error {
                                Err(DownloadFailure::PreparationError("no fallback".into()))
                            } else {
                                // 备用回调中新加的配置不会进入已固定的调度列表。
                                options.add_fallback(|_| panic!("不应动态重复入队"));
                                Ok(options)
                            }
                        });
                        Ok(options)
                    })
                    .build()
            });
            wait_until_finished(&downloader);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            let task = &downloader.tasks()[&0];
            assert!(task.status().is_failed());
            match &*task.failed_reason() {
                DownloadFailure::PreparationError(message) if preparation_error => {
                    assert_eq!(message, "no fallback");
                }
                DownloadFailure::NetworkError(_) if !preparation_error => {}
                reason => panic!("备用渠道错误未保留: {reason:?}"),
            }
        }
    }

    #[test]
    fn cancellation_during_fallback_stops_the_retry() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let mut downloader = DownloadBuilder::new().build();
        downloader.download(move |builder| {
            builder
                .url("invalid-url".into())
                .before_download(move |mut options| {
                    options.add_fallback(move |mut options| {
                        entered_tx.send(()).unwrap();
                        release_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        options.skip_download = true;
                        Ok(options)
                    });
                    options.add_fallback(|_| panic!("取消后不应尝试剩余配置"));
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
