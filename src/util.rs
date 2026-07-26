//! Small pure helpers ported from the Python original.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

/// Parses an "H:M:S(.frac)" timestamp into seconds.
pub fn parse_hms_to_seconds(value: &str) -> Option<f64> {
    let value = value.trim().replace(',', ".");
    if value.is_empty() {
        return None;
    }
    let re = Regex::new(r"^(\d+):(\d+):(\d+(?:\.\d+)?)$").ok()?;
    let m = re.captures(&value)?;
    let h: f64 = m.get(1)?.as_str().parse().ok()?;
    let mn: f64 = m.get(2)?.as_str().parse().ok()?;
    let s: f64 = m.get(3)?.as_str().parse().ok()?;
    Some(h * 3600.0 + mn * 60.0 + s)
}

pub fn format_eta(seconds: Option<f64>) -> String {
    match seconds {
        Some(s) if s >= 0.0 => {
            let total = s as i64;
            let (mm, ss) = (total / 60, total % 60);
            let (hh, mm) = (mm / 60, mm % 60);
            if hh > 0 {
                format!("{:02}:{:02}:{:02}", hh, mm, ss)
            } else {
                format!("{:02}:{:02}", mm, ss)
            }
        }
        _ => "--:--".to_string(),
    }
}

pub fn format_elapsed(seconds: f64) -> String {
    if seconds < 0.0 {
        return "0:00:00.00".to_string();
    }
    let total = seconds as i64;
    let frac = ((seconds - total as f64) * 100.0) as i64;
    let (hh, rem) = (total / 3600, total % 3600);
    let (mm, ss) = (rem / 60, rem % 60);
    format!("{}:{:02}:{:02}.{:02}", hh, mm, ss, frac)
}

/// Parses an ffmpeg-style speed value ("1.23x") into a float.
pub fn parse_speed_value(speed_text: &str) -> Option<f64> {
    if speed_text.is_empty() {
        return None;
    }
    let cleaned = speed_text.to_lowercase().replace('x', "");
    let cleaned = cleaned.trim();
    match cleaned.parse::<f64>() {
        Ok(v) if v > 0.0 => Some(v),
        _ => None,
    }
}

const ILLEGAL_FS_CHARS: &str = "<>:\"/\\|?*";

/// Makes a user-typed name safe for the filesystem.
pub fn sanitize_filename(name: &str) -> String {
    let mut cleaned: String = name
        .chars()
        .map(|ch| {
            if ILLEGAL_FS_CHARS.contains(ch) || (ch as u32) < 32 {
                '_'
            } else {
                ch
            }
        })
        .collect();
    cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    cleaned.chars().take(200).collect()
}

/// Returns `path` if free, otherwise the first free "name (N).ext" variant.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    for i in 1..1000 {
        let name = match ext {
            Some(e) => format!("{} ({}).{}", stem, i, e),
            None => format!("{} ({})", stem, i),
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = match ext {
        Some(e) => format!("{} ({}).{}", stem, ts, e),
        None => format!("{} ({})", stem, ts),
    };
    path.with_file_name(name)
}

/// Escapes a literal path for use as a yt-dlp output template.
pub fn escape_outtmpl(path: &str) -> String {
    path.replace('%', "%%")
}

/// Turns a concrete target path into a yt-dlp output template (ext left to yt-dlp).
pub fn outtmpl_for(path: &Path) -> String {
    let without_ext = path.with_extension("");
    format!("{}.%(ext)s", escape_outtmpl(&without_ext.to_string_lossy()))
}

const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "ts", "mp3", "m4a", "wav",
    "opus", "flac", "aac", "ogg", "oga",
];

/// Drops a media extension the user typed, so the real container decides it.
pub fn strip_media_ext(name: &str) -> String {
    let p = Path::new(name);
    let suffix = p
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());
    match suffix {
        Some(s) if MEDIA_EXTS.contains(&s.as_str()) => p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name)
            .to_string(),
        _ => name.to_string(),
    }
}

/// Parses a version string into a tuple of the integers it contains.
pub fn parse_version(text: &str) -> Vec<u64> {
    let re = Regex::new(r"\d+").unwrap();
    let nums: Vec<u64> = re
        .find_iter(text)
        .filter_map(|m| m.as_str().parse().ok())
        .collect();
    if nums.is_empty() {
        vec![0]
    } else {
        nums
    }
}

/// True when `installed` is a parseable yt-dlp CalVer older than `required`.
pub fn ytdlp_older_than(installed: &str, required: &str) -> bool {
    let inst = parse_version(installed);
    if inst.len() < 3 {
        return false;
    }
    let req = parse_version(required);
    inst[..3] < req[..3.min(req.len())]
}

/// Audio-only download quality keys mapped to their final extension.
pub fn audio_ext_by_quality(quality: &str) -> Option<&'static str> {
    match quality {
        "audio" => Some("mp3"),
        "audio_wav" => Some("wav"),
        "audio_opus" => Some("opus"),
        _ => None,
    }
}

/// Convert-tab audio codec keys mapped to their forced output extension.
pub fn convert_audio_ext(codec_key: &str) -> Option<&'static str> {
    match codec_key {
        "audio_mp3" => Some("mp3"),
        "audio_wav" => Some("wav"),
        _ => None,
    }
}
