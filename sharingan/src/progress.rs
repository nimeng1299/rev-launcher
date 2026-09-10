//! 下载进度模块。
//!
//! [`Progress`] 用原子类型保存已下载字节数、总大小与瞬时速度，
//! 可被 Worker 线程写入，同时被任意监控线程安全读取。

use atomic_float::AtomicF64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// 单个下载任务的进度信息。
///
/// 所有字段均为原子类型，因此无需额外加锁即可跨线程并发读写：
/// Worker 线程负责更新，外部监控线程负责轮询展示。
///
/// # 示例
///
/// ```
/// use sharingan::progress::Progress;
///
/// let progress = Progress::default_with_total(1_048_576);
/// progress.update(524_288, 1_048_576.0);
///
/// assert_eq!(progress.downloaded(), 524_288);
/// assert_eq!(progress.total(), Some(1_048_576));
/// assert_eq!(progress.speed(), 1_048_576.0);
/// ```
#[derive(Debug)]
pub struct Progress {
    /// 已下载的字节数。
    downloaded: AtomicU64,
    /// 是否已知文件总大小（例如从 HTTP 响应头 `Content-Length` 解析得到）。
    have_total: AtomicBool,
    /// 文件总字节数；仅当 `have_total` 为真时有效。
    total: AtomicU64,
    /// 瞬时下载速度，单位 B/s。
    speed: AtomicF64,
}

impl Progress {
    /// 使用给定的初始值创建进度对象。
    ///
    /// - `downloaded`：已下载的字节数；
    /// - `total`：文件总字节数，传 `None` 表示总大小未知（例如响应中没有 `Content-Length`）；
    /// - `speed`：初始下载速度（B/s）。
    pub fn new(downloaded: u64, total: Option<u64>, speed: f64) -> Self {
        let (have_total, total) = if let Some(total_) = total {
            (AtomicBool::new(true), AtomicU64::new(total_))
        } else {
            (AtomicBool::new(false), AtomicU64::new(0))
        };
        Self {
            downloaded: AtomicU64::new(downloaded),
            have_total,
            total,
            speed: AtomicF64::new(speed),
        }
    }

    /// 创建一个总大小未知、各数值为 0 的进度对象。
    pub fn default_no_total() -> Self {
        Self::new(0, None, 0f64)
    }

    /// 创建一个总大小为 `total`、已下载字节与速度均为 0 的进度对象。
    pub fn default_with_total(total: u64) -> Self {
        Self::new(0, Some(total), 0f64)
    }

    /// 更新已知的文件总大小。
    ///
    /// 传入 `Some(total)` 时记录总大小并标记为已知；
    /// 传入 `None` 时标记为总大小未知。
    pub fn change_total(&self, total: Option<u64>) {
        if let Some(total_) = total {
            self.have_total.store(true, Ordering::SeqCst);
            self.total.store(total_, Ordering::SeqCst);
        } else {
            self.have_total.store(false, Ordering::SeqCst);
        };
    }

    /// 更新已下载字节数与瞬时下载速度。
    ///
    /// 由 Worker 线程在下载过程中周期性调用。
    pub fn update(&self, downloaded: u64, speed: f64) {
        self.downloaded.store(downloaded, Ordering::SeqCst);
        self.speed.store(speed, Ordering::SeqCst);
    }

    /// 返回已下载的字节数。
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::SeqCst)
    }

    /// 返回文件总字节数；若总大小未知则返回 `None`。
    pub fn total(&self) -> Option<u64> {
        if self.have_total.load(Ordering::SeqCst) {
            Some(self.total.load(Ordering::SeqCst))
        } else {
            None
        }
    }

    /// 返回瞬时下载速度（B/s）。
    pub fn speed(&self) -> f64 {
        self.speed.load(Ordering::SeqCst)
    }
}

impl Default for Progress {
    /// 默认进度对象：总大小未知，已下载字节与速度均为 0。
    fn default() -> Self {
        Self::default_no_total()
    }
}
