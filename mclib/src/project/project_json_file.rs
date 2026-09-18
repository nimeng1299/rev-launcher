use serde::de::Error;
use serde_json::Value;

fn json_get_vec(json: &Value, pointer: &str) -> Result<Vec<Value>, crate::error::Error> {
    json.pointer(pointer)
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or(crate::error::Error::Json(serde_json::Error::custom(format!(
            "missing field: {pointer}"
        ))))
}

pub fn json_get_libraries(json: &Value) -> Result<Vec<Value>, crate::error::Error> {
    json_get_vec(json, "/libraries")
}

pub fn json_get_patches(json: &Value) -> Result<Vec<Value>, crate::error::Error> {
    json_get_vec(json, "/patches")
}
