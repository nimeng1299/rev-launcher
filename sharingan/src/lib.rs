//! sharingan —— 一个多线程、基于原子操作的轻量 HTTP 下载库。
//!
//! 本库采用「生产者-消费者」模型实现并发下载：
//!
//! - [`downloader::Downloader`] 调度器：管理线程池、任务队列与运行状态；
//! - [`worker::Worker`] 工作线程：从就绪队列领取任务并执行真实下载；
//! - [`task::Task`] 任务：描述单个下载的地址、请求参数、保存位置与状态；
//! - [`progress::Progress`] 进度：用原子类型保存字节数与瞬时速度，支持跨线程轮询；
//! - [`status::DownloadStatus`]、[`status::TaskStatus`]：任务与调度器的状态；
//! - [`status::DownloadFailure`]：下载失败原因。
//!
//! 调度器内部共享数据均使用原子类型或无锁容器传递，因此调用方可以在任意
//! 线程安全地提交任务、查询状态与读取进度，无需额外加锁。
//!
//! # 快速上手
//!
//! ```no_run
//! use sharingan::downloader::DownloadBuilder;
//! use std::thread;
//! use std::time::Duration;
//!
//! // 构建一个 4 线程的下载器，线程池随之启动。
//! let mut downloader = DownloadBuilder::default().thread_num(4).build();
//!
//! // 提交任务：闭包接收 TaskBuilder，返回配置完成的 Task，id 自动分配。
//! downloader.download(|builder| {
//!     builder
//!         .url("https://example.com/file.bin".to_string())
//!         .path("/tmp/download".into())
//!         .filename("file.bin".to_string())
//!         .overwrite(true)
//!         .build()
//! });
//!
//! // 轮询直到所有任务结束。
//! while !downloader.is_finished() {
//!     thread::sleep(Duration::from_millis(100));
//! }
//!
//! // 读取每个任务的最终状态。
//! for (id, task) in downloader.tasks().iter() {
//!     println!("task {id}: {:?}", task.status());
//! }
//! ```
//!
//! 更完整的示例参见 `examples/simple_download.rs`。

pub mod downloader;
pub mod progress;
pub mod status;
pub mod task;
pub mod worker;

#[cfg(test)]
mod test {
    use crate::downloader::DownloadBuilder;

    #[test]
    fn download_single_test() {
        let mut downloader = DownloadBuilder::default().thread_num(4).build();
        downloader.download(|builder| {
            builder
                .url("https://testfile.to/dl/1mb".to_string())
                .path(std::env::current_dir().unwrap().join("test"))
                .filename("test.bin".to_string())
                .overwrite(true)
                .build()
        });

        loop {
            if downloader.is_finished() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        // for guard in downloader.finish_map().iter() {
        //     let id = guard.key();
        //     let task = guard.val();
        //     eprintln!("id: {id} status: {:?}", task.status());
        // }

        assert_eq!(0, 0);
    }

    #[test]
    fn download_2file_test() {
        let mut downloader = DownloadBuilder::default().thread_num(4).build();
        downloader.download(|builder| {
            builder
                .url("https://testfile.to/dl/10mb".to_string())
                .path(std::env::current_dir().unwrap().join("test"))
                .filename("test10mb.bin".to_string())
                .overwrite(true)
                .build()
        });

        std::thread::sleep(std::time::Duration::from_secs(3));

        downloader.download(|builder| {
            builder
                .url("https://testfile.to/dl/1mb".to_string())
                .path(std::env::current_dir().unwrap().join("test"))
                .filename("test.bin".to_string())
                .overwrite(true)
                .build()
        });

        loop {
            if downloader.is_finished() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        // for guard in downloader.finish_map().iter() {
        //     let id = guard.key();
        //     let task = guard.val();
        //     eprintln!("id: {id} status: {:?}", task.status());
        // }

        assert_eq!(0, 0);
    }
}
