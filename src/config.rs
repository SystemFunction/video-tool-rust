//! JSON-backed settings store (ported from ConfigManager).

use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};

pub struct Config {
    file: PathBuf,
    data: Map<String, Value>,
}

impl Config {
    pub fn load(app_dir: &PathBuf) -> Self {
        let _ = fs::create_dir_all(app_dir);
        let file = app_dir.join("config.json");
        let data = fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        Config { file, data }
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(self.data.clone())) {
            let tmp = self.file.with_extension("json.tmp");
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(&tmp, &self.file);
            }
        }
    }

    pub fn get_str(&self, key: &str, default: &str) -> String {
        match self.data.get(key) {
            Some(Value::String(s)) => s.clone(),
            _ => default.to_string(),
        }
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        match self.data.get(key) {
            Some(Value::Bool(b)) => *b,
            _ => default,
        }
    }

    pub fn set_str(&mut self, key: &str, value: &str) {
        self.data
            .insert(key.to_string(), Value::String(value.to_string()));
        self.save();
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.data.insert(key.to_string(), Value::Bool(value));
        self.save();
    }
}
