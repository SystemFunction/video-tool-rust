//! The list of finished jobs shown in the History tab.
//!
//! Entries live in the same JSON config as every other setting, so the list
//! survives a restart without a second store to keep consistent. Only what
//! the tab can act on is kept - a path, the URL a download came from and the
//! preset it ran with; nothing about the media itself is cached, because the
//! file on disk is the authority and may well be gone by the next start.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::consts::HISTORY_LIMIT;

const HISTORY_KEY: &str = "history";

/// Which tab produced an entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Download,
    Convert,
}

impl Kind {
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Download => "⬇",
            Kind::Convert => "⚙",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub kind: Kind,
    /// File name, or the URL when the run never named a file.
    pub name: String,
    /// Absolute path, empty when unknown.
    pub path: String,
    /// Source URL for downloads, empty for conversions.
    pub url: String,
    /// Quality preset or codec, shown as the entry's second line.
    pub detail: String,
    /// Unix seconds; 0 for an entry written by a build without a clock.
    pub when: u64,
}

impl Default for Entry {
    fn default() -> Self {
        Entry {
            kind: Kind::Download,
            name: String::new(),
            path: String::new(),
            url: String::new(),
            detail: String::new(),
            when: 0,
        }
    }
}

impl Entry {
    pub fn exists(&self) -> bool {
        !self.path.is_empty() && Path::new(&self.path).exists()
    }

    /// The folder the file sits in, for "open containing folder".
    pub fn folder(&self) -> Option<String> {
        Path::new(&self.path)
            .parent()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().to_string())
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads the stored list. A malformed entry drops out rather than taking the
/// whole history with it - one bad record should not cost the user the rest.
pub fn load(config: &Config) -> Vec<Entry> {
    let Some(value) = config.get_value(HISTORY_KEY) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| serde_json::from_value::<Entry>(v.clone()).ok())
        .take(HISTORY_LIMIT)
        .collect()
}

pub fn store(config: &mut Config, entries: &[Entry]) {
    let trimmed = &entries[..entries.len().min(HISTORY_LIMIT)];
    if let Ok(value) = serde_json::to_value(trimmed) {
        config.set_value(HISTORY_KEY, value);
    }
}

/// Puts `entry` at the front, dropping an older record of the same file.
///
/// Converting the same source twice is a normal way to work, and a list that
/// filled up with repeats of one name would hide everything else.
pub fn push(entries: &mut Vec<Entry>, entry: Entry) {
    if !entry.path.is_empty() {
        entries.retain(|e| e.path != entry.path);
    }
    entries.insert(0, entry);
    entries.truncate(HISTORY_LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> Entry {
        Entry {
            name: path.to_string(),
            path: path.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn the_newest_run_of_a_file_is_the_one_that_is_kept() {
        let mut list = vec![entry("a.mp4"), entry("b.mp4")];
        push(&mut list, entry("a.mp4"));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].path, "a.mp4");
        assert_eq!(list[1].path, "b.mp4");
    }

    #[test]
    fn entries_without_a_path_are_all_kept() {
        let mut list = Vec::new();
        push(&mut list, Entry { name: "one".into(), ..Default::default() });
        push(&mut list, Entry { name: "two".into(), ..Default::default() });
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn the_list_never_grows_past_its_limit() {
        let mut list = Vec::new();
        for i in 0..HISTORY_LIMIT + 20 {
            push(&mut list, entry(&format!("{i}.mp4")));
        }
        assert_eq!(list.len(), HISTORY_LIMIT);
        // The most recent push is the one at the front.
        assert_eq!(list[0].path, format!("{}.mp4", HISTORY_LIMIT + 19));
    }
}
