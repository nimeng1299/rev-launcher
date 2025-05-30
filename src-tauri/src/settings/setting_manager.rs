use std::collections::HashMap;

use super::{java_versions::JavaVersions, setting_trait::SettingTrait};

pub struct SettingManager {
    id: i32,
    value: HashMap<String, Box<dyn SettingTrait>>,
}

impl SettingManager {
    pub fn new(id: i32) -> Self {
        let mut value: HashMap<String, Box<dyn SettingTrait>> = HashMap::new();
        value.insert(
            "java".to_string(),
            JavaVersions::read_from_file(None).unwrap(),
        );
        SettingManager { id, value }
    }
}
