use std::collections::HashMap;

use super::setting_trait::SettingTrait;

pub struct SettingManager {
    id: i32,
    value: HashMap<String, Box<dyn SettingTrait>>,
}
