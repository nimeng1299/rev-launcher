//! 下载调度模块。
//!
//! [`Downloader`] 负责线程池与任务队列的管理；[`DownloadBuilder`] 用于
//! 配置线程数并构建下载器。

use crate::status::TaskStatus;
use crate::task::{Task, TaskBuilder};
use crate::worker::Worker;
use atomig::Atomic;
use lockfree::map::Map;
use lockfree::queue::Queue;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

/// 下载调度器。
///
/// 采用「生产者-消费者」模型工作：
/// - 调用 [`download()`](Downloader::download) 提交任务，任务进入内部就绪队列；
/// - 构建时启动的多个 [`Worker`] 线程会自动领取并下载任务；
/// - 任务状态与进度存放在共享容器中，外部线程可随时查询。
///
/// 调度器内部数据大多为原子类型或无锁容器，因此其方法可被多线程安全调用
/// （例如监控线程调用 [`is_finished()`](Downloader::is_finished) 判断是否全部完成）。
///
/// `Downloader` 被析构（`Drop`）时会自动调用 [`cancel()`](Downloader::cancel)
/// 通知所有 Worker 线程退出。
pub struct Downloader {
    /// 已分配的任务 id 计数器（每提交一个任务自增）。
    task_number: AtomicUsize,
    /// 线程池大小。
    pub thread_num: usize,
    /// 下载器运行状态，控制所有 Worker 线程的生命周期。
    status: Arc<Atomic<TaskStatus>>,
    /// 已提交的所有任务表：任务 id -> 任务。
    tasks: HashMap<usize, Arc<Task>>,
    /// 就绪队列：存放等待 Worker 领取的任务。
    ready_map: Arc<Queue<Arc<Task>>>,
    /// 进行中任务表：Worker 序号 -> 任务。
    progressing_map: Arc<Map<usize, Arc<Task>>>,
    /// 已完成任务表：任务 id -> 任务。
    finish_map: Arc<Map<usize, Arc<Task>>>,
    /// Worker 线程句柄集合：`(线程句柄, Worker 序号)`。
    threads: Vec<(JoinHandle<()>, usize)>,
}

impl Downloader {
    /// 由构建器生成下载器。
    ///
    /// 会根据构建器指定的线程数启动相应数量的 [`Worker`]
    /// 线程，并创建各共享容器。
    pub fn from_builder(builder: DownloadBuilder) -> Self {
        let thread_num = builder.thread_num;
        let mut threads = Vec::new();

        let status = Arc::new(Atomic::new(TaskStatus::Running));

        let tasks = HashMap::default();

        let ready_map = Arc::new(Queue::default());
        let progressing_map = Arc::new(Map::default());
        let finish_map = Arc::new(Map::default());

        for i in 0..thread_num {
            let status_clone = status.clone();

            let ready_map_clone = ready_map.clone();
            let progressing_map_clone = progressing_map.clone();
            let finish_map_clone = finish_map.clone();

            let handle = std::thread::spawn(move || {
                let mut worker = Worker::new(
                    i,
                    status_clone,
                    ready_map_clone,
                    progressing_map_clone,
                    finish_map_clone,
                );
                worker.run();
            });

            threads.push((handle, i));
        }

        Downloader {
            task_number: AtomicUsize::new(0),
            thread_num,
            status,
            tasks,
            ready_map,
            progressing_map,
            finish_map,
            threads,
        }
    }

    /// 取消下载：把运行状态置为「取消」。
    ///
    /// 所有 Worker 会在完成（或中断）当前任务后退出；
    /// 该任务不会立刻终止正在进行的网络请求。
    pub fn cancel(&self) {
        self.status.store(TaskStatus::Cancel, Ordering::SeqCst);
    }

    /// 把一个已构建好的任务加入下载队列，并登记到任务表中。
    ///
    /// 任务的 id 由调用方保证唯一（推荐通过 [`download()`](Downloader::download)
    /// 提交，由其自动分配 id）。
    pub fn add_task(&mut self, task: Task) {
        let task = Arc::new(task);
        self.ready_map.push(task.clone());
        self.tasks.insert(task.id(), task);
    }

    /// 提交一个下载任务。
    ///
    /// 闭包 `task_builder` 接收一个 [`TaskBuilder`] 并返回配置完成的
    /// [`Task`]；任务 id 由调度器自动分配。任务提交后会被 Worker 线程
    /// 自动领取下载。
    /// 此闭包在提交线程立即执行；耗时前置工作应放入
    /// [`TaskBuilder::before_download`]，由 Worker 并发执行。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use sharingan::downloader::DownloadBuilder;
    ///
    /// let mut downloader = DownloadBuilder::default().thread_num(4).build();
    ///
    /// downloader.download(|builder| {
    ///     builder
    ///         .url("https://example.com/file.bin".to_string())
    ///         .path("/tmp".into())
    ///         .filename("file.bin".to_string())
    ///         .overwrite(true)
    ///         .build()
    /// });
    ///
    /// downloader.cancel();
    /// ```
    pub fn download<F>(&mut self, task_builder: F)
    where
        F: FnOnce(TaskBuilder) -> Task + 'static,
    {
        let task = task_builder(TaskBuilder::new(
            self.task_number.fetch_add(1, Ordering::SeqCst),
        ));
        self.add_task(task);
    }

    /// 返回下载器当前运行状态。
    pub fn status(&self) -> TaskStatus {
        self.status.load(Ordering::SeqCst)
    }

    /// 返回已提交的所有任务表（任务 id -> 任务）。
    pub fn tasks(&self) -> &HashMap<usize, Arc<Task>> {
        &self.tasks
    }

    /// 返回就绪队列句柄的克隆，可用于观察等待下载的任务。
    pub fn ready_map(&self) -> Arc<Queue<Arc<Task>>> {
        self.ready_map.clone()
    }

    /// 返回进行中任务表句柄的克隆（Worker 序号 -> 任务）。
    pub fn progressing_map(&self) -> Arc<Map<usize, Arc<Task>>> {
        self.progressing_map.clone()
    }

    /// 返回已完成任务表句柄的克隆（任务 id -> 任务）。
    pub fn finish_map(&self) -> Arc<Map<usize, Arc<Task>>> {
        self.finish_map.clone()
    }

    /// 判断所有已提交任务是否均已结束（成功或失败）。
    pub fn is_finished(&self) -> bool {
        for (_, task) in self.tasks.iter() {
            if !task.status().is_finished() {
                return false;
            }
        }
        true
    }

    /// 判断所有已提交任务是否均已成功（注意有未完成的任务也会返回false）。
    pub fn is_all_success(&self) -> bool {
        for (_, task) in self.tasks.iter() {
            if !task.status().is_success() {
                return false;
            }
        }
        true
    }

    /// 返回 Worker 线程句柄列表 `(线程句柄, Worker 序号)`。
    ///
    /// 注意：线程句柄由 `Downloader` 持有，取消后线程自行退出，
    /// `Downloader` 不会主动 join 它们。
    pub fn threads(&self) -> &Vec<(JoinHandle<()>, usize)> {
        &self.threads
    }
}

impl Drop for Downloader {
    /// 析构时取消所有下载任务并通知 Worker 退出。
    fn drop(&mut self) {
        self.cancel();
    }
}

/// 下载器构建器。
///
/// 采用 Builder 模式配置参数（目前仅线程数），通过 [`build()`](DownloadBuilder::build)
/// 生成 [`Downloader`] 并启动线程池。
///
/// # 示例
///
/// ```
/// use sharingan::downloader::DownloadBuilder;
///
/// // 等价写法：DownloadBuilder::new().thread_num(8).build()
/// let downloader = DownloadBuilder::default().thread_num(8).build();
/// ```
pub struct DownloadBuilder {
    /// 线程池大小（即同时下载的任务数）。
    pub thread_num: usize,
}

impl DownloadBuilder {
    /// 创建默认构建器：线程数为 1。
    pub fn new() -> DownloadBuilder {
        DownloadBuilder { thread_num: 1 }
    }

    /// 完成配置并生成 [`Downloader`]，同时启动线程池。
    pub fn build(self) -> Downloader {
        Downloader::from_builder(self)
    }

    /// 设置线程池大小。
    pub fn thread_num(mut self, thread_num: usize) -> Self {
        self.thread_num = thread_num;
        self
    }
}

impl Default for DownloadBuilder {
    /// 默认构建器，等价于 [`DownloadBuilder::new()`]。
    fn default() -> DownloadBuilder {
        DownloadBuilder::new()
    }
}
