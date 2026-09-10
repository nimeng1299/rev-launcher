//! 任务模块。
//!
//! 提供下载任务 [`Task`] 及其构建器 [`TaskBuilder`]。

use crate::progress::Progress;
use crate::status::{DownloadFailure, DownloadStatus};
use atomig::Atomic;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 一个下载任务。
///
/// 保存目标地址、请求参数、保存位置、超时、状态与进度等全部信息。
/// 任务通常通过 [`TaskBuilder`] 配置生成；若由调度器
/// [`Downloader::download()`](crate::downloader::Downloader::download) 提交，
/// 任务 id 会自动分配并保证唯一。
///
/// 任务构建完成后可被多个 Worker 线程安全共享：`status` 与进度均基于
/// 原子类型实现，外部线程可以随时通过 [`status()`](Task::status) 和
/// [`progress()`](Task::progress) 观察下载进展，无需额外加锁。
///
/// 两个任务相等的充要条件是 `id` 相同（见 `PartialEq` 实现）。
#[derive(Debug)]
pub struct Task {
    /// 任务唯一 id。
    id: usize,
    /// 下载地址。
    url: String,
    /// 附加到 URL 上的查询参数列表 `(键, 值)`。
    query: Vec<(String, String)>,
    /// 附加的请求头列表 `(键, 值)`。
    header: Vec<(String, String)>,
    /// 文件保存目录。
    path: PathBuf,
    /// 下载完成后的最终文件名。
    filename: String,
    /// 下载过程中的临时文件名，下载完成后会被重命名为最终文件名。
    temp_filename: String,
    /// 最终文件已存在时是否覆盖（覆盖前会先删除旧文件）。
    overwrite: bool,
    /// 网络请求超时时间；`None` 表示不设置全局超时。
    timeout: Option<Duration>,
    /// 下载状态（原子变量，可跨线程读写）。
    status: Atomic<DownloadStatus>,
    /// 下载失败原因（由 Worker 线程写入）。
    failed_reason: Mutex<DownloadFailure>,
    /// 下载进度。
    progress: Progress,
}

impl Task {
    /// 由构建器生成一个 [`Task`]。
    ///
    /// 构建时会根据当前系统时间生成临时文件名，并把任务状态初始化为
    /// [`DownloadStatus::Ready`]。
    pub fn from_builder(builder: TaskBuilder) -> Task {
        let now = SystemTime::now();

        let nanos = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::new(0721, 2770))
            .as_nanos();

        let temp_filename = format!("{}-{}.downloader", nanos, builder.filename);

        Self {
            id: builder.id,
            url: builder.url,
            query: builder.query,
            header: builder.header,
            path: builder.path,
            filename: builder.filename,
            temp_filename,
            overwrite: builder.overwrite,
            timeout: builder.timeout,
            status: Atomic::new(DownloadStatus::Ready),
            failed_reason: Mutex::new(DownloadFailure::Unknown),
            progress: Progress::default(),
        }
    }

    /// 返回任务唯一 id。
    pub fn id(&self) -> usize {
        self.id
    }

    /// 返回下载地址。
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 返回附加的查询参数列表。
    pub fn query(&self) -> &[(String, String)] {
        &self.query
    }

    /// 返回附加的请求头列表。
    pub fn header(&self) -> &[(String, String)] {
        &self.header
    }

    /// 返回文件保存目录。
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// 返回最终文件名。
    pub fn filename(&self) -> &String {
        &self.filename
    }

    /// 返回下载过程中的临时文件名。
    pub fn temp_filename(&self) -> &String {
        &self.temp_filename
    }

    /// 返回是否覆盖已存在的文件。
    pub fn overwrite(&self) -> bool {
        self.overwrite
    }

    /// 返回网络请求超时时间。
    pub fn timeout(&self) -> &Option<Duration> {
        &self.timeout
    }

    /// 把任务状态切换为「下载中」。
    ///
    /// 由 Worker 线程在真正开始下载前调用。
    pub fn change_downloading(&self) {
        self.status
            .store(DownloadStatus::Downloading, Ordering::SeqCst);
    }

    /// 把任务状态切换为「成功」。
    ///
    /// 由 Worker 线程在下载完成、临时文件落盘并重命名后调用。
    pub fn change_success(&self) {
        self.status
            .store(DownloadStatus::Succeeded, Ordering::SeqCst);
    }

    /// 把任务状态切换为「失败」，并记录失败原因。
    ///
    /// 由 Worker 线程在下载任意一步出错时调用。
    pub fn change_failure(&self, reason: DownloadFailure) {
        self.status.store(DownloadStatus::Failed, Ordering::SeqCst);
        if let Ok(mut guard) = self.failed_reason.lock() {
            *guard = reason;
        }
    }

    /// 返回当前下载状态（原子读取，可在任意线程调用）。
    pub fn status(&self) -> DownloadStatus {
        self.status.load(Ordering::SeqCst)
    }

    /// 返回任务进度的引用（原子读取，可在任意线程轮询）。
    pub fn progress(&self) -> &Progress {
        &self.progress
    }
}

impl PartialEq for Task {
    /// 两个任务仅在 id 相同时视为相等。
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// 下载任务构建器。
///
/// 采用 Builder 模式逐步配置任务字段，最后通过 [`build()`](TaskBuilder::build)
/// 生成 [`Task`]。多数场景下无需直接构造：使用
/// [`Downloader::download()`](crate::downloader::Downloader::download) 提交任务时，
/// 调度器会传入一个已经分配好 id 的 [`TaskBuilder`]。
pub struct TaskBuilder {
    /// 任务 id。
    id: usize,
    /// 下载地址。
    url: String,
    /// 附加的查询参数列表 `(键, 值)`。
    query: Vec<(String, String)>,
    /// 附加的请求头列表 `(键, 值)`。
    header: Vec<(String, String)>,
    /// 文件保存目录。
    path: PathBuf,
    /// 最终文件名。
    filename: String,
    /// 是否覆盖已存在的文件。
    overwrite: bool,
    /// 网络请求超时时间。
    timeout: Option<Duration>,
}

impl TaskBuilder {
    /// 创建一个指定任务 id 的空构建器。
    pub fn new(id: usize) -> TaskBuilder {
        Self {
            id,
            url: String::new(),
            query: vec![],
            header: vec![],
            path: PathBuf::new(),
            filename: String::new(),
            overwrite: false,
            timeout: None,
        }
    }

    /// 设置下载地址。
    pub fn url(mut self, url: String) -> Self {
        self.url = url;
        self
    }

    /// 整体替换查询参数列表。
    pub fn query(mut self, query: Vec<(String, String)>) -> Self {
        self.query = query;
        self
    }

    /// 追加一个查询参数 `(key, value)`。
    pub fn add_query(mut self, key: String, value: String) -> Self {
        self.query.push((key, value));
        self
    }

    /// 清空所有查询参数。
    pub fn clear_query(mut self) -> Self {
        self.query.clear();
        self
    }

    /// 整体替换请求头列表。
    pub fn header(mut self, header: Vec<(String, String)>) -> Self {
        self.header = header;
        self
    }

    /// 追加一个请求头 `(key, value)`。
    pub fn add_header(mut self, key: String, value: String) -> Self {
        self.header.push((key, value));
        self
    }

    /// 清空所有请求头。
    pub fn clear_header(mut self) -> Self {
        self.header.clear();
        self
    }

    /// 设置文件保存目录。
    pub fn path(mut self, path: PathBuf) -> Self {
        self.path = path;
        self
    }

    /// 设置最终文件名。
    pub fn filename(mut self, filename: String) -> Self {
        self.filename = filename;
        self
    }

    /// 设置是否覆盖已存在的文件。
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// 设置网络请求超时时间；`None` 表示不设置。
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// 完成配置并生成 [`Task`]。
    pub fn build(self) -> Task {
        Task::from_builder(self)
    }
}
