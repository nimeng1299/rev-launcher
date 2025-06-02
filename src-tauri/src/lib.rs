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
/// id: -1 is globle
#[tauri::command]
fn get_setting_value(id: i32, item_name: String) -> Result<Value, String> {
    let setting = Setting::instance();
    let setting = setting.read().unwrap();

    let globle_setting = setting.get_globle().get_setting();
    if id == -1 {
        if let Ok(value) = globle_setting.get(item_name) {
            return Ok(value.clone());
        }
        return Err("err get globle setting value".to_string());
    } else {
        if let Some(modpack_setting_manager) = setting.get(id) {
            if let Ok(value) = modpack_setting_manager
                .get_setting()
                .get(item_name, &globle_setting)
            {
                return Ok(value.clone());
            }
        }
    }
    Err("err get setting value".to_string())
}

#[tauri::command]
fn change_setting_value(id: i32, item_name: String, value: Vec<String>) {
    let rusult = Setting::instance()
        .write()
        .unwrap()
        .change(id, item_name, value);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            file_dialog,
            get_setting_value,
            change_setting_value
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
