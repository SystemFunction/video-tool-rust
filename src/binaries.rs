//! Manages the tool's own copies of yt-dlp, ffmpeg/ffprobe and Deno.
//!
//! Mirrors the Python BinaryManager: HTTPS-only downloads, atomic writes,
//! and yt-dlp integrity verification against the release-signed SHA2-256SUMS.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::types::BinaryStatus;

const DOWNLOAD_CHUNK: usize = 1 << 17; // 128 KiB

pub struct Binaries {
    pub is_windows: bool,
    pub system: String,  // "Windows" | "Darwin" | "Linux"
    pub machine: String, // lowercased arch
    pub app_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub plugins_dir: PathBuf,
    pub ytdlp_local: PathBuf,
    pub ffmpeg_local: PathBuf,
    pub ffprobe_local: PathBuf,
    pub deno_local: PathBuf,
}

impl Binaries {
    pub fn new() -> Self {
        let system = detect_system();
        let machine = std::env::consts::ARCH.to_lowercase();
        let is_windows = system == "Windows";
        let app_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".video_tool_v3");
        let bin_dir = app_dir.join("bin");
        let plugins_dir = app_dir.join("plugins");
        let _ = fs::create_dir_all(&bin_dir);
        let _ = fs::create_dir_all(&plugins_dir);
        let exe = if is_windows { ".exe" } else { "" };
        let ytdlp_local = bin_dir.join(format!("yt-dlp{exe}"));
        let ffmpeg_local = bin_dir.join(format!("ffmpeg{exe}"));
        let ffprobe_local = bin_dir.join(format!("ffprobe{exe}"));
        let deno_local = bin_dir.join(format!("deno{exe}"));
        Binaries {
            is_windows,
            system,
            machine,
            app_dir,
            bin_dir,
            plugins_dir,
            ytdlp_local,
            ffmpeg_local,
            ffprobe_local,
            deno_local,
        }
    }

    // -- path resolution: prefer the local copy, else fall back to PATH --

    fn resolve(&self, name: &str, local: &PathBuf) -> String {
        if local.exists() {
            local.to_string_lossy().to_string()
        } else {
            name.to_string()
        }
    }

    pub fn ytdlp_path(&self) -> String {
        self.resolve("yt-dlp", &self.ytdlp_local)
    }
    pub fn ffmpeg_path(&self) -> String {
        self.resolve("ffmpeg", &self.ffmpeg_local)
    }
    pub fn ffprobe_path(&self) -> String {
        self.resolve("ffprobe", &self.ffprobe_local)
    }
    /// A Command with the console window hidden (Windows) and bin_dir on PATH.
    pub fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        self.apply_env(&mut cmd);
        no_window(&mut cmd);
        // A GUI app has no console to read from; leaving stdin inherited lets
        // a child block forever waiting on input that can never arrive.
        cmd.stdin(Stdio::null());
        cmd
    }

    /// Prepends bin_dir to PATH so a bundled Deno is found by yt-dlp.
    pub fn apply_env(&self, cmd: &mut Command) {
        let bin = self.bin_dir.to_string_lossy().to_string();
        let cur = std::env::var("PATH").unwrap_or_default();
        let sep = if self.is_windows { ';' } else { ':' };
        let already = cur.split(sep).any(|p| p == bin);
        if !already {
            let new = if cur.is_empty() {
                bin
            } else {
                format!("{bin}{sep}{cur}")
            };
            cmd.env("PATH", new);
        }
    }

    // -- checks --

    fn run_capture(&self, program: &str, args: &[&str]) -> Option<(bool, String)> {
        let out = self.command(program).args(args).output().ok()?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.trim().is_empty() {
            text = String::from_utf8_lossy(&out.stderr).to_string();
        }
        Some((out.status.success(), text))
    }

    pub fn check_ytdlp(&self) -> (bool, String) {
        match self.run_capture(&self.ytdlp_path(), &["--version"]) {
            Some((true, t)) => {
                let v = t.trim();
                (true, if v.is_empty() { "OK".into() } else { v.into() })
            }
            _ => (false, "Not found".into()),
        }
    }

    pub fn check_ffmpeg(&self) -> (bool, String) {
        match self.run_capture(&self.ffmpeg_path(), &["-version"]) {
            Some((true, t)) => {
                let ver = t
                    .split_whitespace()
                    .skip_while(|w| *w != "version")
                    .nth(1)
                    .unwrap_or("OK");
                (true, ver.to_string())
            }
            _ => (false, "Not found".into()),
        }
    }

    /// Returns ("deno"|"node"|"", version-string).
    pub fn js_runtime(&self) -> (String, String) {
        let mut candidates: Vec<(String, String)> = Vec::new();
        if self.deno_local.exists() {
            candidates.push(("deno".into(), self.deno_local.to_string_lossy().into()));
        }
        for rt in ["deno", "node"] {
            candidates.push((rt.into(), rt.into()));
        }
        for (name, exe) in candidates {
            if let Some((true, out)) = self.run_capture(&exe, &["--version"]) {
                let ver = out.lines().next().unwrap_or(&name).trim().to_string();
                let ver = if ver.is_empty() { name.clone() } else { ver };
                return (name, ver);
            }
        }
        (String::new(), String::new())
    }

    pub fn check_deno(&self) -> (bool, String) {
        let (name, ver) = self.js_runtime();
        if name.is_empty() {
            (false, "Not found".into())
        } else if !ver.is_empty() && ver.contains(&name) {
            (true, ver)
        } else {
            (true, format!("{name} {ver}"))
        }
    }

    pub fn detect_hw_encoder(&self) -> String {
        if let Some((true, out)) = self.run_capture(&self.ffmpeg_path(), &["-hide_banner", "-encoders"]) {
            let lo = out.to_lowercase();
            if lo.contains("h264_nvenc") {
                return "nvidia".into();
            }
            if lo.contains("h264_qsv") {
                return "intel".into();
            }
            if lo.contains("h264_amf") {
                return "amd".into();
            }
        }
        "cpu".into()
    }

    pub fn supports_impersonation(&self) -> bool {
        matches!(
            self.run_capture(&self.ytdlp_path(), &["--list-impersonate-targets"]),
            Some((true, _))
        )
    }

    /// Full status probe (blocking) used by the background worker.
    pub fn probe_status(&self) -> BinaryStatus {
        let (ytdlp_ok, ytdlp_version) = self.check_ytdlp();
        let (ffmpeg_ok, ffmpeg_version) = self.check_ffmpeg();
        let hw_backend = if ffmpeg_ok {
            self.detect_hw_encoder()
        } else {
            "cpu".into()
        };
        let impersonate_ok = if ytdlp_ok {
            self.supports_impersonation()
        } else {
            false
        };
        let (deno_ok, deno_version) = self.check_deno();
        let js_runtime = if deno_ok {
            deno_version.split_whitespace().next().unwrap_or("").to_string()
        } else {
            String::new()
        };
        BinaryStatus {
            ytdlp_ok,
            ytdlp_version,
            ffmpeg_ok,
            ffmpeg_version,
            deno_ok,
            deno_version,
            hw_backend,
            impersonate_ok,
            js_runtime,
        }
    }

    // -- download core --

    fn download_to_file(
        &self,
        url: &str,
        target: &PathBuf,
        on_progress: impl FnMut(u64, u64),
    ) -> Result<(), String> {
        download_to_file(url, target, on_progress)
    }

    fn sha256_file(path: &PathBuf) -> Result<String, String> {
        sha256_file(path)
    }
}

/// Streams an HTTPS download into a file (atomic tmp -> rename).
pub fn download_to_file(
    url: &str,
    target: &PathBuf,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    if !url.to_lowercase().starts_with("https://") {
        return Err(format!("Refusing non-HTTPS download URL: {url}"));
    }
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(180))
        .call()
        .map_err(|e| format!("request failed: {e}"))?;
    // Redirects are followed automatically, so the URL we vetted above is
    // not necessarily the one that served the bytes. Re-check the final
    // hop; otherwise a redirect to http:// would silently downgrade the
    // transport for an executable we are about to run.
    if !resp.get_url().to_lowercase().starts_with("https://") {
        return Err(format!(
            "Refusing redirect to a non-HTTPS URL: {}",
            resp.get_url()
        ));
    }
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let tmp = {
        let mut p = target.clone();
        let ext = p
            .extension()
            .map(|e| format!("{}.part", e.to_string_lossy()))
            .unwrap_or_else(|| "part".to_string());
        p.set_extension(ext);
        p
    };
    let mut reader = resp.into_reader();
    let mut written: u64 = 0;
    {
        let mut f = File::create(&tmp).map_err(|e| format!("create tmp: {e}"))?;
        let mut buf = vec![0u8; DOWNLOAD_CHUNK];
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("read: {e}"))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| format!("write: {e}"))?;
            written += n as u64;
            on_progress(written, total);
        }
        f.flush().ok();
    }
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("rename: {e}")
    })?;
    Ok(())
}

pub fn sha256_file(path: &PathBuf) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; DOWNLOAD_CHUNK];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

impl Binaries {
    /// Verifies a freshly downloaded yt-dlp binary against SHA2-256SUMS.
    ///
    /// Fails closed. This binary is executed with the user's privileges on
    /// every download, so "could not verify" has to be treated exactly like
    /// "did not match": anything able to disrupt the checksum request would
    /// otherwise be enough to get an unverified executable installed.
    fn verify_ytdlp_checksum(&self, target: &PathBuf, asset_name: &str) -> Result<(), String> {
        let discard = |msg: String| -> String {
            let _ = fs::remove_file(target);
            msg
        };

        let sums_url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";
        let sums_text = ureq::get(sums_url)
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(|e| discard(format!("could not fetch SHA2-256SUMS ({e}) - yt-dlp download discarded, please retry")))?
            .into_string()
            .map_err(|e| discard(format!("could not read SHA2-256SUMS ({e}) - yt-dlp download discarded, please retry")))?;

        let expected = sums_text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let hash = parts.next()?;
                let name = parts.next()?.trim_start_matches('*');
                (name == asset_name && parts.next().is_none()).then(|| hash.to_lowercase())
            })
            .find(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| {
                discard(format!(
                    "no valid SHA2-256SUMS entry for '{asset_name}' - yt-dlp download discarded"
                ))
            })?;

        let actual = Self::sha256_file(target).map_err(discard)?;
        if actual != expected {
            return Err(discard(format!(
                "yt-dlp checksum mismatch - download discarded (expected {}..., got {}...).",
                short_hash(&expected),
                short_hash(&actual)
            )));
        }
        Ok(())
    }

    fn ytdlp_url(&self) -> (String, String) {
        let base = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";
        let asset = if self.system == "Windows" {
            "yt-dlp.exe"
        } else if self.system == "Darwin" {
            "yt-dlp_macos"
        } else if self.machine.contains("aarch64") || self.machine.contains("arm64") {
            "yt-dlp_linux_aarch64"
        } else {
            "yt-dlp"
        };
        (format!("{base}{asset}"), asset.to_string())
    }

    fn ffmpeg_platform_key(&self) -> &'static str {
        if self.system == "Windows" {
            "windows-64"
        } else if self.system == "Darwin" {
            "osx-64"
        } else if self.machine.contains("aarch64") || self.machine.contains("arm64") {
            "linux-arm64"
        } else {
            "linux-64"
        }
    }

    fn deno_url(&self) -> String {
        let base = "https://github.com/denoland/deno/releases/latest/download/";
        let asset = if self.system == "Windows" {
            "deno-x86_64-pc-windows-msvc.zip"
        } else if self.system == "Darwin" {
            if self.machine.contains("arm64") || self.machine.contains("aarch64") {
                "deno-aarch64-apple-darwin.zip"
            } else {
                "deno-x86_64-apple-darwin.zip"
            }
        } else if self.machine.contains("aarch64") || self.machine.contains("arm64") {
            "deno-aarch64-unknown-linux-gnu.zip"
        } else {
            "deno-x86_64-unknown-linux-gnu.zip"
        };
        format!("{base}{asset}")
    }

    pub fn install_ytdlp(&self, on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
        let (url, asset) = self.ytdlp_url();
        self.download_to_file(&url, &self.ytdlp_local, on_progress)?;
        self.verify_ytdlp_checksum(&self.ytdlp_local, &asset)?;
        Ok(())
    }

    pub fn install_ffmpeg(
        &self,
        mut on_progress: impl FnMut(&str, u64, u64),
    ) -> Result<(), String> {
        let key = self.ffmpeg_platform_key();
        let data: serde_json::Value = ureq::get("https://ffbinaries.com/api/v1/version/latest")
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(|e| format!("ffbinaries request failed: {e}"))?
            .into_json()
            .map_err(|e| format!("ffbinaries json: {e}"))?;
        let binaries = &data["bin"][key];
        if !binaries.is_object() {
            return Err(format!("No ffmpeg binaries available for {key}."));
        }
        for (name, target) in [("ffmpeg", &self.ffmpeg_local), ("ffprobe", &self.ffprobe_local)] {
            let url = match binaries[name].as_str() {
                Some(u) => u.to_string(),
                None => continue,
            };
            let zip_path = self.bin_dir.join(format!("{name}.zip"));
            let nm = name.to_string();
            let res = self
                .download_to_file(&url, &zip_path, |w, t| on_progress(&nm, w, t))
                .and_then(|_| extract_named(&zip_path, target, name));
            let _ = fs::remove_file(&zip_path);
            res?;
        }
        Ok(())
    }

    pub fn install_deno(&self, on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
        let url = self.deno_url();
        let zip_path = self.bin_dir.join("deno.zip");
        let res = self
            .download_to_file(&url, &zip_path, on_progress)
            .and_then(|_| extract_named(&zip_path, &self.deno_local, "deno"));
        let _ = fs::remove_file(&zip_path);
        res
    }

    /// Uses yt-dlp's own --update-to to switch release channel.
    pub fn update_channel(&self, channel: &str) -> Result<String, String> {
        let out = self
            .command(&self.ytdlp_path())
            .args(["--update-to", channel])
            .output()
            .map_err(|e| e.to_string())?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        let text = text.trim().to_string();
        if !out.status.success() {
            return Err(if text.is_empty() {
                format!("yt-dlp --update-to {channel} failed")
            } else {
                text
            });
        }
        Ok(text)
    }
}

/// Truncates a hex digest for display without ever splitting a character.
fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// Extracts the member named `name`(.exe) from a zip into `target`.
///
/// Only the member's base name is ever considered and the destination is a
/// fixed path, so a crafted archive cannot steer the write elsewhere.
fn extract_named(zip_path: &PathBuf, target: &PathBuf, name: &str) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let wanted = [name.to_string(), format!("{name}.exe")];
    for i in 0..archive.len() {
        let mut member = archive.by_index(i).map_err(|e| e.to_string())?;
        if member.is_dir() {
            continue;
        }
        let base = member
            .name()
            .to_lowercase()
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_string();
        if wanted.contains(&base) {
            let mut out = File::create(target).map_err(|e| e.to_string())?;
            std::io::copy(&mut member, &mut out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("{name} not found in the ZIP"))
}

fn detect_system() -> String {
    match std::env::consts::OS {
        "windows" => "Windows".into(),
        "macos" => "Darwin".into(),
        _ => "Linux".into(),
    }
}

/// Hides the console window on Windows for spawned processes.
#[cfg(windows)]
pub fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
pub fn no_window(_cmd: &mut Command) {}
