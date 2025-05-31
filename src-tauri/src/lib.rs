use serde_json::Value;

use settings::setting::Setting;

mod api;
mod settings;

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
        .invoke_handler(tauri::generate_handler![get_setting_value])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
