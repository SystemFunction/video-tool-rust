//! JSON-backed settings store (ported from ConfigManager).
//!
//! Setters only mark the store dirty; the actual write is coalesced by
//! `flush`. Text fields report a change on every keystroke, and rewriting
//! plus renaming the whole file that often is both wasteful and a needless
//! window for a torn config.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

/// Shortest gap between two unforced writes.
const FLUSH_INTERVAL: Duration = Duration::from_millis(1500);

pub struct Config {
    file: PathBuf,
    data: Map<String, Value>,
    dirty: bool,
    last_write: Instant,
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
        Config {
            file,
            data,
            dirty: false,
            last_write: Instant::now(),
        }
    }

    /// Writes pending changes. `force` ignores the coalescing interval and is
    /// what shutdown and pre-run paths use so nothing is lost.
    pub fn flush(&mut self, force: bool) {
        if !self.dirty || (!force && self.last_write.elapsed() < FLUSH_INTERVAL) {
            return;
        }
        self.last_write = Instant::now();
        self.dirty = false;
        let Ok(text) = serde_json::to_string_pretty(&Value::Object(self.data.clone())) else {
            return;
        };
        let tmp = self.file.with_extension("json.tmp");
        if fs::write(&tmp, text).is_ok() && fs::rename(&tmp, &self.file).is_err() {
            let _ = fs::remove_file(&tmp);
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
        if matches!(self.data.get(key), Some(Value::String(s)) if s == value) {
            return;
        }
        self.data
            .insert(key.to_string(), Value::String(value.to_string()));
        self.dirty = true;
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        if matches!(self.data.get(key), Some(Value::Bool(b)) if *b == value) {
            return;
        }
        self.data.insert(key.to_string(), Value::Bool(value));
        self.dirty = true;
    }
}
