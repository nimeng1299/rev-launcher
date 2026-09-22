use gpui_kit::Global;

#[derive(Debug)]
pub struct AppData {}

impl Global for AppData {}

impl AppData {
    pub fn init() -> AppData {
        AppData {}
    }
}

/// 项目列表的修订号：磁盘上项目有增删时 +1。
///
/// 各个页面的项目缓存（下拉框、项目列表）只在渲染期对比这个数字，
/// 变了就重新扫描目录，不用互相发事件。
#[derive(Debug, Default)]
pub struct ProjectsRevision {
    revision: u64,
}

impl Global for ProjectsRevision {}

impl ProjectsRevision {
    pub fn init() -> Self {
        Self::default()
    }

    /// 当前修订号。
    pub fn get(&self) -> u64 {
        self.revision
    }

    /// 磁盘上的项目有变化时调用，通知各页面重扫。
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}
