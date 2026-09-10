use gpui_kit::Global;

#[derive(Debug)]
pub struct AppData {}

impl Global for AppData {}

impl AppData {
    pub fn init() -> AppData {
        AppData {}
    }
}
