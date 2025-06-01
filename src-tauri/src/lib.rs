use std::path::PathBuf;

use rfd::FileDialog;
use serde_json::Value;

use settings::setting::Setting;

mod api;
mod settings;

///base
///------------------------
#[tauri::command]
fn file_dialog(filter: Vec<(String, Vec<String>)>, set_directory: String) -> Option<PathBuf> {
    let mut dialog = FileDialog::new();
    for i in 0..filter.len() {
        let (name, fts) = &filter[i];
        dialog = dialog.add_filter(name, fts);
    }
    dialog.set_directory(set_directory).pick_file()
}

///setting
///------------------------
#[tauri::command]
fn get_setting_value(id: i32, item_name: String) -> Result<Value, String> {
    let setting = Setting::instance();
    let setting = setting.read().unwrap();
    if let Some(setting_manager) = setting.get(id) {
        if let Ok(value) = setting_manager.get_setting().get(item_name) {
            return Ok(value.clone());
        }
    }
    Err("err get setting value".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![file_dialog, get_setting_value])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
