//! 状态定义模块。
//!
//! 提供下载器运行状态 [`TaskStatus`]、单个任务下载状态 [`DownloadStatus`]
//! 以及失败原因 [`DownloadFailure`]，供调度器、Worker 与外部监控代码使用。
//!
//! 相关枚举均基于 `atomig::Atom` 派生，可放入原子变量进行跨线程读写。

use atomig::Atom;

/// 下载器的整体运行状态，用于控制所有 [`Worker`](crate::worker::Worker) 线程的生命周期。
///
/// 该状态由调度器维护，Worker 线程在每次循环开始时读取。
#[derive(Atom, PartialEq, Debug)]
#[repr(u8)]
pub enum TaskStatus {
    /// 运行中：Worker 持续从就绪队列领取任务并下载。
    Running,
    /// 已取消：Worker 清理当前任务后退出。
    Cancel,
}

/// 单个下载任务的状态。
///
/// 状态由 Worker 线程写入（见 [`Task`](crate::task::Task) 的 `change_*` 系列方法），
/// 外部线程可通过 [`Task::status()`](crate::task::Task::status) 随时读取。
#[derive(Atom, Debug)]
#[repr(u8)]
pub enum DownloadStatus {
    /// 已就绪，等待 Worker 领取。
    Ready,
    /// 正在下载中。
    Downloading,
    /// 下载成功，临时文件已重命名为最终文件名。
    Succeeded,
    /// 下载失败，失败原因见 [`DownloadFailure`]。
    Failed,
}

impl DownloadStatus {
    /// 判断任务是否已成功完成。
    ///
    /// 只有状态为 [`DownloadStatus::Succeeded`] 时返回 `true`。
    pub fn is_success(&self) -> bool {
        match self {
            DownloadStatus::Succeeded => true,
            _ => false,
        }
    }

    /// 判断任务是否已成功完成。
    ///
    /// 只有状态为 [`DownloadStatus::Failed`] 时返回 `true`。
    pub fn is_failed(&self) -> bool {
        match self {
            DownloadStatus::Failed => true,
            _ => false,
        }
    }

    /// 判断任务是否已经结束（无论成功或失败）。
    ///
    /// 状态为 [`DownloadStatus::Succeeded`] 或 [`DownloadStatus::Failed`] 时返回 `true`，
    /// 仍在排队或下载中时返回 `false`。
    pub fn is_finished(&self) -> bool {
        match self {
            DownloadStatus::Succeeded => true,
            DownloadStatus::Failed => true,
            _ => false,
        }
    }
}

/// 下载失败的具体原因。
///
/// 由 Worker 线程在下载出错时通过
/// [`Task::change_failure()`](crate::task::Task::change_failure) 记录到任务中。
#[derive(Debug)]
pub enum DownloadFailure {
    /// 用户主动取消。
    UserCancel,
    /// 网络请求失败（连接失败、超时、TLS 错误等）。
    NetworkError(ureq::Error),
    /// 服务器返回了非成功的 HTTP 状态码（如 404、500）。
    HttpStatusCode(ureq::http::StatusCode),
    /// 本地 IO 失败（创建目录、创建/读写文件等）。
    IOError(std::io::Error),
    /// 下载完成后的完整性校验失败。
    ValidationError,
    /// 未知原因。
    Unknown,
}
