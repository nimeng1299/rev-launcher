//! 任务模块。
//!
//! 提供下载任务 [`Task`] 及其构建器 [`TaskBuilder`]。

use crate::progress::Progress;
use crate::status::{DownloadFailure, DownloadStatus};
use atomig::Atomic;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 下载完成后对文件进行完整性校验的回调。
///
/// 回调接收已完整写入并同步到磁盘的临时文件路径：返回 `Ok(())` 表示校验通过，
/// 返回 `Err(reason)` 表示校验失败，失败原因会记录到任务中。
pub type DownloadValidator = Arc<dyn Fn(PathBuf) -> Result<(), DownloadFailure> + Send + Sync>;

/// 下载所需的配置，前置回调可以补全或替换其中的任意字段。
#[derive(Clone, Default)]
pub struct DownloadOptions {
    /// 下载地址。
    pub url: String,
    /// 附加到 URL 上的查询参数。
    pub query: Vec<(String, String)>,
    /// 附加的请求头。
    pub header: Vec<(String, String)>,
    /// 文件保存目录。
    pub path: PathBuf,
    /// 最终文件名。
    pub filename: String,
    /// 是否覆盖已存在的文件。
    pub overwrite: bool,
    /// 网络请求超时；不包含前置回调的执行时间。
    pub timeout: Option<Duration>,
    /// 下载完成后的完整性校验。
    pub validator: Option<DownloadValidator>,
}

/// 在 Worker 中执行一次的下载前置回调；只有返回 `Ok` 才继续下载。
pub type DownloadPreparation =
    Box<dyn FnOnce(DownloadOptions) -> Result<DownloadOptions, DownloadFailure> + Send>;

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
pub struct Task {
    /// 任务唯一 id。
    id: usize,
    /// 提交任务时的初始配置。
    options: DownloadOptions,
    /// 前置回调成功后一次性发布的最终配置。
    prepared_options: OnceLock<DownloadOptions>,
    /// 由领取任务的 Worker 取出并执行一次。
    before_download: Mutex<Option<DownloadPreparation>>,
    /// 下载过程中的临时文件名，下载完成后会被重命名为最终文件名。
    temp_filename: String,
    /// 下载状态（原子变量，可跨线程读写）。
    status: Atomic<DownloadStatus>,
    /// 下载失败原因（由 Worker 线程写入）。
    failed_reason: Mutex<DownloadFailure>,
    /// 下载进度。
    progress: Progress,
}

impl std::fmt::Debug for Task {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Task")
            .field("id", &self.id)
            .field("url", &self.url())
            .field("query", &self.query())
            .field("header", &self.header())
            .field("path", &self.path())
            .field("filename", &self.filename())
            .field("temp_filename", &self.temp_filename)
            .field("overwrite", &self.overwrite())
            .field("timeout", &self.timeout())
            .field("status", &self.status)
            .field("failed_reason", &self.failed_reason)
            .field("progress", &self.progress)
            .finish()
    }
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

        let temp_filename = format!("{}-{}.downloader", nanos, builder.options.filename);

        Self {
            id: builder.id,
            options: builder.options,
            prepared_options: OnceLock::new(),
            before_download: Mutex::new(builder.before_download),
            temp_filename,
            status: Atomic::new(DownloadStatus::Ready),
            failed_reason: Mutex::new(DownloadFailure::Unknown),
            progress: Progress::default(),
        }
    }

    /// 返回任务唯一 id。
    pub fn id(&self) -> usize {
        self.id
    }

    /// 返回当前配置：前置回调成功前为初始配置，成功后为最终配置。
    ///
    /// 需要同时读取多个字段时，可通过此方法取得同一份配置快照。
    /// 先前取得的初始配置引用仍然有效，但不会随前置回调自动更新。
    pub fn options(&self) -> &DownloadOptions {
        self.prepared_options.get().unwrap_or(&self.options)
    }

    /// 只由领取任务的 Worker 调用一次；执行用户回调时不持有互斥锁。
    pub(crate) fn prepare(&self) -> Result<(), DownloadFailure> {
        let prepare = self
            .before_download
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(prepare) = prepare {
            let options = prepare(self.options.clone())?;
            let _ = self.prepared_options.set(options);
        }
        Ok(())
    }

    /// 返回下载地址。
    pub fn url(&self) -> &str {
        &self.options().url
    }

    /// 返回附加的查询参数列表。
    pub fn query(&self) -> &[(String, String)] {
        &self.options().query
    }

    /// 返回附加的请求头列表。
    pub fn header(&self) -> &[(String, String)] {
        &self.options().header
    }

    /// 返回文件保存目录。
    pub fn path(&self) -> &PathBuf {
        &self.options().path
    }

    /// 返回最终文件名。
    pub fn filename(&self) -> &String {
        &self.options().filename
    }

    /// 返回下载过程中的临时文件名。
    pub fn temp_filename(&self) -> &String {
        &self.temp_filename
    }

    /// 返回是否覆盖已存在的文件。
    pub fn overwrite(&self) -> bool {
        self.options().overwrite
    }

    /// 返回网络请求超时时间。
    pub fn timeout(&self) -> &Option<Duration> {
        &self.options().timeout
    }

    /// 返回下载完成后的完整性校验回调。
    pub fn validator(&self) -> Option<&DownloadValidator> {
        self.options().validator.as_ref()
    }

    /// 把任务状态切换为「准备中」。
    pub fn change_preparing(&self) {
        self.status
            .store(DownloadStatus::Preparing, Ordering::SeqCst);
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
        *self.failed_reason() = reason;
        self.status.store(DownloadStatus::Failed, Ordering::SeqCst);
    }

    /// 读取失败原因；仅在状态为 `Failed` 时有意义。读取完应及时释放锁。
    pub fn failed_reason(&self) -> MutexGuard<'_, DownloadFailure> {
        self.failed_reason
            .lock()
            .unwrap_or_else(|error| error.into_inner())
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
    /// 下载配置。
    options: DownloadOptions,
    /// 下载前置回调。
    before_download: Option<DownloadPreparation>,
}

impl TaskBuilder {
    /// 创建一个指定任务 id 的空构建器。
    pub fn new(id: usize) -> TaskBuilder {
        Self {
            id,
            options: DownloadOptions::default(),
            before_download: None,
        }
    }

    /// 设置下载地址。
    pub fn url(mut self, url: String) -> Self {
        self.options.url = url;
        self
    }

    /// 整体替换查询参数列表。
    pub fn query(mut self, query: Vec<(String, String)>) -> Self {
        self.options.query = query;
        self
    }

    /// 追加一个查询参数 `(key, value)`。
    pub fn add_query(mut self, key: String, value: String) -> Self {
        self.options.query.push((key, value));
        self
    }

    /// 清空所有查询参数。
    pub fn clear_query(mut self) -> Self {
        self.options.query.clear();
        self
    }

    /// 整体替换请求头列表。
    pub fn header(mut self, header: Vec<(String, String)>) -> Self {
        self.options.header = header;
        self
    }

    /// 追加一个请求头 `(key, value)`。
    pub fn add_header(mut self, key: String, value: String) -> Self {
        self.options.header.push((key, value));
        self
    }

    /// 清空所有请求头。
    pub fn clear_header(mut self) -> Self {
        self.options.header.clear();
        self
    }

    /// 设置文件保存目录。
    pub fn path(mut self, path: PathBuf) -> Self {
        self.options.path = path;
        self
    }

    /// 设置最终文件名。
    pub fn filename(mut self, filename: String) -> Self {
        self.options.filename = filename;
        self
    }

    /// 设置是否覆盖已存在的文件。
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.options.overwrite = overwrite;
        self
    }

    /// 设置网络请求超时时间；`None` 表示不设置。
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.options.timeout = timeout;
        self
    }

    /// 设置下载完成后的完整性校验回调。
    ///
    /// 回调接收已完整写入并同步到磁盘的临时文件路径。返回 `Ok(())` 才会将
    /// 临时文件重命名为最终文件；返回 `Err(reason)` 时记录该原因，清理临时文件，
    /// 并保留已存在的目标文件。内容不匹配可返回 [`DownloadFailure::ValidationError`]。
    ///
    /// ```
    /// use sharingan::status::DownloadFailure;
    /// use sharingan::task::TaskBuilder;
    ///
    /// let task = TaskBuilder::new(0).validator(|path| {
    ///     let metadata = std::fs::metadata(path).map_err(DownloadFailure::IOError)?;
    ///     if metadata.len() == 0 {
    ///         return Err(DownloadFailure::ValidationError);
    ///     }
    ///     Ok(())
    /// }).build();
    /// ```
    pub fn validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(PathBuf) -> Result<(), DownloadFailure> + Send + Sync + 'static,
    {
        self.options.validator = Some(Arc::new(validator));
        self
    }

    /// 设置下载前置回调；重复设置会替换旧回调。
    ///
    /// 回调在 Worker 线程中执行一次，接收构建器的完整配置，可在此查询元数据、
    /// 确定 URL、文件名、请求头等。返回 `Ok(options)` 才会发起下载请求；
    /// 返回 `Err(reason)` 则记录失败原因并结束任务，不创建下载文件。
    /// 多个任务的前置回调与下载共用线程池，并发上限为下载器的 `thread_num`。
    /// 回调执行期间状态为 [`DownloadStatus::Preparing`]，不计入网络请求超时。
    /// 取消不会强行中断回调，但回调返回后会检查取消状态，避免继续下载。
    ///
    /// ```no_run
    /// use sharingan::downloader::DownloadBuilder;
    /// use sharingan::status::DownloadFailure;
    ///
    /// let mut downloader = DownloadBuilder::new().thread_num(4).build();
    /// downloader.download(|builder| {
    ///     builder
    ///         .path("downloads".into())
    ///         .before_download(|mut options| {
    ///             // 也可以在这里请求接口，动态获取下载地址和文件名。
    ///             options.url = std::fs::read_to_string("download-url.txt")
    ///                 .map_err(|e| DownloadFailure::PreparationError(e.to_string()))?
    ///                 .trim().to_owned();
    ///             options.filename = "file.bin".into();
    ///             Ok(options)
    ///         })
    ///         .build()
    /// });
    /// ```
    pub fn before_download<F>(mut self, prepare: F) -> Self
    where
        F: FnOnce(DownloadOptions) -> Result<DownloadOptions, DownloadFailure> + Send + 'static,
    {
        self.before_download = Some(Box::new(prepare));
        self
    }

    /// 完成配置并生成 [`Task`]。
    pub fn build(self) -> Task {
        Task::from_builder(self)
    }
}
