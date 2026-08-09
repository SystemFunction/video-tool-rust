//! Checks GitHub Releases for a newer build and installs it in place.
//!
//! The download is verified against the `digest` GitHub reports for the
//! release asset. That digest arrives over the same TLS-authenticated API
//! response as the download URL, so no separate checksum file has to be
//! published alongside a release - and, as with the yt-dlp path, a missing
//! or mismatching digest is treated as a failure rather than waved through.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::binaries::{download_to_file, sha256_file};
use crate::consts::VERSION;
use crate::util;

const API_LATEST: &str =
    "https://api.github.com/repos/SystemFunction/video-tool-rust/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/SystemFunction/video-tool-rust/releases/latest";

/// Suffix of the binary the previous version was moved aside to.
const BACKUP_SUFFIX: &str = ".old";

/// How often the release query is attempted before giving up.
const CHECK_ATTEMPTS: u32 = 3;
/// Multiplied by the attempt number, so the waits are 1s and then 2s.
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct Release {
    /// Numeric version without the leading "v", e.g. "0.1.2".
    pub version: String,
    pub notes: String,
    pub page_url: String,
    /// None when the release has no asset for this platform.
    pub asset: Option<Asset>,
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    /// Lowercase hex sha-256, without the "sha256:" prefix.
    pub sha256: String,
}

/// Name of the release asset that carries the binary for this platform.
fn asset_name() -> &'static str {
    if cfg!(windows) {
        "video_tool.exe"
    } else {
        "video_tool"
    }
}

/// True when `remote` is a strictly higher version than `local`.
pub fn is_newer(remote: &str, local: &str) -> bool {
    let r = util::parse_version(remote);
    let l = util::parse_version(local);
    for i in 0..r.len().max(l.len()) {
        let (a, b) = (
            r.get(i).copied().unwrap_or(0),
            l.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    false
}

/// True for failures that another attempt can plausibly get past.
///
/// GitHub's API edge nodes hand out the occasional 502/503/504 that is gone
/// again seconds later; without a retry that momentary blip surfaces to the
/// user as a hard "update check failed".
fn is_transient(e: &ureq::Error) -> bool {
    match e {
        ureq::Error::Status(code, _) => matches!(code, 408 | 429 | 500..=599),
        // Timeouts, DNS hiccups, dropped connections.
        ureq::Error::Transport(_) => true,
    }
}

/// Short, self-explanatory rendering of a failed request.
///
/// ureq's own Display repeats the full API URL, which the UI then shows
/// verbatim behind an already-explanatory prefix.
fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => {
            format!("GitHub returned status {code} - please try again later")
        }
        ureq::Error::Transport(t) => format!("could not reach GitHub ({t})"),
    }
}

/// Fetches the latest-release document, retrying past transient failures.
fn fetch_latest() -> Result<serde_json::Value, String> {
    let mut last = String::new();
    for attempt in 0..CHECK_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RETRY_BACKOFF * attempt);
        }
        match ureq::get(API_LATEST)
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", &format!("video_tool/{VERSION}"))
            .timeout(Duration::from_secs(20))
            .call()
        {
            Ok(resp) => {
                return resp
                    .into_json()
                    .map_err(|e| format!("unexpected release data: {e}"))
            }
            Err(e) if is_transient(&e) => last = describe(&e),
            Err(e) => return Err(describe(&e)),
        }
    }
    Err(last)
}

/// Queries the latest release. `Ok(None)` means "nothing newer than us".
pub fn check_latest() -> Result<Option<Release>, String> {
    let body = fetch_latest()?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or("release has no tag_name")?
        .to_string();
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, VERSION) {
        return Ok(None);
    }

    let wanted = asset_name();
    let asset = body["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(wanted))
        .and_then(|a| {
            // Without a usable digest we deliberately produce no asset, which
            // downgrades the offer to "open the release page" rather than
            // installing something unverified.
            let digest = a["digest"].as_str()?.strip_prefix("sha256:")?.to_lowercase();
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            Some(Asset {
                url: a["browser_download_url"].as_str()?.to_string(),
                size: a["size"].as_u64().unwrap_or(0),
                sha256: digest,
            })
        });

    Ok(Some(Release {
        version,
        notes: body["body"].as_str().unwrap_or_default().trim().to_string(),
        page_url: body["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
        asset,
    }))
}

/// Downloads `asset` and swaps it in for the running executable.
///
/// Returns the path the previous binary was kept at. Windows allows renaming
/// a running .exe but not overwriting it, so the old file is moved aside and
/// restored if anything goes wrong; it is cleaned up on the next start.
pub fn install(asset: &Asset, on_progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate the running exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or("the running exe has no parent directory")?;

    // Staged next to the target so the final swap is a same-volume rename.
    let staged = dir.join("video_tool.update-staging");
    let _ = fs::remove_file(&staged);
    download_to_file(&asset.url, &staged, on_progress)?;

    let discard = |msg: String| -> String {
        let _ = fs::remove_file(&staged);
        msg
    };

    if asset.size > 0 {
        let got = fs::metadata(&staged).map(|m| m.len()).unwrap_or(0);
        if got != asset.size {
            return Err(discard(format!(
                "size mismatch - update discarded (expected {} bytes, got {got})",
                asset.size
            )));
        }
    }
    let actual = sha256_file(&staged).map_err(discard)?;
    if actual != asset.sha256 {
        return Err(discard(format!(
            "checksum mismatch - update discarded (expected {}..., got {}...)",
            &asset.sha256[..12],
            &actual[..actual.len().min(12)]
        )));
    }

    let backup = with_suffix(&exe, BACKUP_SUFFIX);
    let _ = fs::remove_file(&backup);
    fs::rename(&exe, &backup)
        .map_err(|e| discard(format!("could not move the current version aside: {e}")))?;
    if let Err(e) = fs::rename(&staged, &exe) {
        // Put the working binary back before giving up.
        let _ = fs::rename(&backup, &exe);
        return Err(discard(format!("could not install the update: {e}")));
    }
    Ok(backup)
}

/// Relaunches the (already replaced) executable and asks the caller to quit.
pub fn restart() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    crate::binaries::no_window(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Removes the binary left behind by a previous update, if it is still there.
pub fn cleanup_backup() {
    if let Ok(exe) = std::env::current_exe() {
        let backup = with_suffix(&exe, BACKUP_SUFFIX);
        if backup.exists() {
            let _ = fs::remove_file(backup);
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_are_detected() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn same_or_older_versions_are_not_offered() {
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.9.9", "1.0.0"));
    }

    #[test]
    fn differing_component_counts_compare_by_position() {
        // "0.2" is newer than "0.1.9"; "0.1" is not newer than "0.1.0".
        assert!(is_newer("0.2", "0.1.9"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(is_newer("0.1.0.1", "0.1.0"));
    }

    #[test]
    fn a_leading_v_in_the_tag_is_ignored() {
        assert!(!is_newer("v0.1.1".trim_start_matches('v'), "0.1.1"));
    }

    /// Builds a `ureq::Error::Status` the way a failed request would.
    fn status_err(code: u16) -> ureq::Error {
        let resp = ureq::Response::new(code, "", "").unwrap();
        ureq::Error::Status(code, resp)
    }

    #[test]
    fn server_side_failures_are_retried() {
        for code in [408, 429, 500, 502, 503, 504] {
            assert!(is_transient(&status_err(code)), "{code} should be retried");
        }
    }

    #[test]
    fn client_side_failures_are_not_retried() {
        for code in [400, 401, 403, 404] {
            assert!(
                !is_transient(&status_err(code)),
                "{code} should not be retried"
            );
        }
    }

    #[test]
    fn the_message_names_the_status_without_repeating_the_url() {
        let msg = describe(&status_err(504));
        assert!(msg.contains("504"), "{msg}");
        assert!(!msg.contains("api.github.com"), "{msg}");
    }

    #[test]
    fn backup_path_keeps_the_extension_before_the_suffix() {
        let p = with_suffix(Path::new("C:/x/video_tool.exe"), ".old");
        assert_eq!(p.file_name().unwrap(), "video_tool.exe.old");
    }
}
