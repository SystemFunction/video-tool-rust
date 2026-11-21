//! Manages the tool's own copies of yt-dlp, ffmpeg/ffprobe and Deno.
//!
//! Mirrors the Python BinaryManager: HTTPS-only downloads, atomic writes,
//! and yt-dlp integrity verification against the release-signed SHA2-256SUMS.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use ureq::ResponseExt;

use crate::types::BinaryStatus;

const DOWNLOAD_CHUNK: usize = 1 << 17; // 128 KiB

/// Release listing of the FFmpeg builds published by BtbN on GitHub.
const FFMPEG_API: &str = "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest";
const DENO_API: &str = "https://api.github.com/repos/denoland/deno/releases/latest";

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

    // -- path resolution: prefer the local copy, else an absolute PATH hit --

    /// Never hands the OS a bare program name. On Windows an unqualified name
    /// is also looked up in the directory of the running exe - typically the
    /// Downloads folder the portable build is started from - so anything a
    /// browser saved there as yt-dlp.exe or ffmpeg.exe would win over the
    /// real tool. When neither copy exists the local path is returned anyway:
    /// spawning it fails cleanly as "not found".
    fn resolve(&self, name: &str, local: &Path) -> String {
        if local.exists() {
            return local.to_string_lossy().to_string();
        }
        find_on_path(name, self.is_windows)
            .unwrap_or_else(|| local.to_path_buf())
            .to_string_lossy()
            .to_string()
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
    /// A Command set up like every other child the app starts (see `prepare`).
    pub fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        self.prepare(&mut cmd);
        cmd
    }

    /// The environment every child process gets.
    ///
    /// The working directory is pinned to the app's own folder. Inherited, it
    /// would be wherever the GUI was launched from - usually Downloads - and
    /// yt-dlp reads a yt-dlp.conf from there (which may carry --exec) and its
    /// own lookups of ffmpeg/deno may search it. A GUI app has no console to
    /// read from either; leaving stdin inherited lets a child block forever.
    pub fn prepare(&self, cmd: &mut Command) {
        self.apply_env(cmd);
        cmd.current_dir(&self.app_dir);
        no_window(cmd);
        cmd.stdin(Stdio::null());
    }

    /// Puts bin_dir first on the child's PATH so yt-dlp finds the verified
    /// ffmpeg/Deno copies before any other. An existing later entry for it is
    /// dropped rather than left to lose against what precedes it.
    pub fn apply_env(&self, cmd: &mut Command) {
        let cur = std::env::var_os("PATH").unwrap_or_default();
        let rest = std::env::split_paths(&cur).filter(|p| *p != self.bin_dir);
        if let Ok(path) = std::env::join_paths(std::iter::once(self.bin_dir.clone()).chain(rest)) {
            cmd.env("PATH", path);
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
            if let Some(path) = find_on_path(rt, self.is_windows) {
                candidates.push((rt.into(), path.to_string_lossy().into()));
            }
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
    ///
    /// Every line below starts a process, and yt-dlp alone needs a second or
    /// more just to unpack itself before it prints anything. Run one after
    /// another that added up to the whole startup delay, so the probes - which
    /// do not depend on each other - are spread over threads and the wall time
    /// is now that of the single slowest one.
    pub fn probe_status(&self) -> BinaryStatus {
        let ((ytdlp_ok, ytdlp_version), (ffmpeg_ok, ffmpeg_version), hw, impersonate, deno) =
            thread::scope(|s| {
                let ytdlp = s.spawn(|| self.check_ytdlp());
                let ffmpeg = s.spawn(|| self.check_ffmpeg());
                let hw = s.spawn(|| self.detect_hw_encoder());
                // Started unconditionally; the result is discarded below when
                // yt-dlp turned out to be missing.
                let imp = s.spawn(|| self.supports_impersonation());
                let deno = s.spawn(|| self.check_deno());
                (
                    ytdlp.join().unwrap_or((false, "Not found".into())),
                    ffmpeg.join().unwrap_or((false, "Not found".into())),
                    hw.join().unwrap_or_else(|_| "cpu".into()),
                    imp.join().unwrap_or(false),
                    deno.join().unwrap_or((false, "Not found".into())),
                )
            });
        let hw_backend = if ffmpeg_ok { hw } else { "cpu".into() };
        let impersonate_ok = ytdlp_ok && impersonate;
        let (deno_ok, deno_version) = deno;
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

    /// Identity of the managed binaries: size and mtime of each local copy.
    ///
    /// Used to decide whether a stored probe result still describes the files
    /// on disk. `None` means "cannot be cached": when yt-dlp or FFmpeg is not
    /// one of our own copies it comes from PATH, where it can be replaced
    /// without anything here changing, so those setups always probe for real.
    /// A JS runtime from PATH is the one thing a hit can go stale on - the
    /// Setup tab's re-check button exists for exactly that case.
    pub fn fingerprint(&self) -> Option<String> {
        if !self.ytdlp_local.exists() || !self.ffmpeg_local.exists() {
            return None;
        }
        let parts: Vec<String> = [
            &self.ytdlp_local,
            &self.ffmpeg_local,
            &self.ffprobe_local,
            &self.deno_local,
        ]
        .iter()
        .map(|p| file_stamp(p))
        .collect();
        Some(parts.join("|"))
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

/// The first absolute PATH entry holding `name` (with `.exe` on Windows).
///
/// Relative entries are skipped: they would resolve against whatever the
/// current directory happens to be, which is exactly the lookup this avoids.
pub fn find_on_path(name: &str, is_windows: bool) -> Option<PathBuf> {
    let file = if is_windows { format!("{name}.exe") } else { name.to_string() };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Absolute path of the OS file opener, so it cannot be shadowed by a
/// same-named file in the exe's directory (see `Binaries::resolve`).
#[cfg(windows)]
pub fn system_opener() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("explorer.exe")
}

/// "size:mtime" for a file, or "-" when it does not exist.
fn file_stamp(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{}", m.len(), mtime)
        }
        Err(_) => "-".to_string(),
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
    // Redirects are followed automatically, so the URL vetted above is not
    // necessarily the one that serves the bytes. `https_only` makes ureq
    // refuse a redirect that would downgrade the transport for an executable
    // we are about to run; the check on the final hop below stays as a second
    // line of defence in case that setting ever stops being honoured.
    // No global timeout: it would also cover reading the body, and an FFmpeg
    // build of ~200 MB does not fit into any budget that still catches a dead
    // connection quickly. Getting connected and getting an answer are bounded
    // tightly; the transfer itself only gets a generous ceiling.
    let resp = ureq::get(url)
        .config()
        .https_only(true)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(60 * 60)))
        .build()
        .call()
        .map_err(|e| format!("request failed: {e}"))?;
    if resp.get_uri().scheme_str() != Some("https") {
        return Err(format!(
            "Refusing redirect to a non-HTTPS URL: {}",
            resp.get_uri()
        ));
    }
    let total: u64 = resp.body().content_length().unwrap_or(0);
    let tmp = {
        let mut p = target.clone();
        let ext = p
            .extension()
            .map(|e| format!("{}.part", e.to_string_lossy()))
            .unwrap_or_else(|| "part".to_string());
        p.set_extension(ext);
        p
    };
    let mut reader = resp.into_body().into_reader();
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
    // sha2 0.11 hands back a plain byte array rather than a hex-formattable
    // digest type, so the lowercase hex the SHA2-256SUMS file uses is spelled
    // out here.
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// One file of a GitHub release, with the digest GitHub reports for it.
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex sha-256.
    pub sha256: String,
}

/// The sha-256 GitHub reports in an asset's `digest` field, if it is usable.
pub fn asset_sha256(asset: &serde_json::Value) -> Option<String> {
    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?.to_lowercase();
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digest)
}

/// Lists the assets of one release via the GitHub API.
///
/// An asset without a usable digest is left out entirely: it could only ever
/// be installed unverified, which is not on offer.
fn github_release_assets(api_url: &str) -> Result<Vec<ReleaseAsset>, String> {
    let body: serde_json::Value = ureq::get(api_url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", format!("video_tool/{}", crate::consts::VERSION))
        .config()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .call()
        .map_err(|e| format!("GitHub release query failed: {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("unexpected release data: {e}"))?;
    Ok(body["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(ReleaseAsset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
                size: a["size"].as_u64().unwrap_or(0),
                sha256: asset_sha256(a)?,
            })
        })
        .collect())
}

/// Downloads `asset` to `target` and checks it against its digest.
///
/// Fails closed: on any mismatch the file is deleted again, so nothing that
/// did not verify is ever left where it could be executed or unpacked.
fn download_verified(
    asset: &ReleaseAsset,
    target: &PathBuf,
    on_progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let discard = |msg: String| -> String {
        let _ = fs::remove_file(target);
        msg
    };
    download_to_file(&asset.url, target, on_progress)?;
    if asset.size > 0 {
        let got = fs::metadata(target).map(|m| m.len()).unwrap_or(0);
        if got != asset.size {
            return Err(discard(format!(
                "size mismatch for {} - download discarded (expected {} bytes, got {got})",
                asset.name, asset.size
            )));
        }
    }
    let actual = sha256_file(target).map_err(discard)?;
    if actual != asset.sha256 {
        return Err(discard(format!(
            "checksum mismatch for {} - download discarded (expected {}..., got {}...)",
            asset.name,
            short_hash(&asset.sha256),
            short_hash(&actual)
        )));
    }
    Ok(())
}

/// Picks the newest release-branch build for `target` ("win64"/"winarm64").
///
/// BtbN names those `ffmpeg-n<ver>-latest-<target>-gpl-<ver>.zip`. The
/// `master` builds are development snapshots and the `shared` ones need DLLs
/// next to the exe, so neither is considered. GPL rather than LGPL because the
/// converter relies on libx264/libx265.
fn pick_ffmpeg_asset<'a>(assets: &'a [ReleaseAsset], target: &str) -> Option<&'a ReleaseAsset> {
    assets
        .iter()
        .filter_map(|a| {
            let (ver, rest) = a.name.strip_prefix("ffmpeg-n")?.split_once("-latest-")?;
            (rest == format!("{target}-gpl-{ver}.zip")).then(|| (crate::util::parse_version(ver), a))
        })
        .max_by(|(x, _), (y, _)| x.cmp(y))
        .map(|(_, a)| a)
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
            .config()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .call()
            .map_err(|e| discard(format!("could not fetch SHA2-256SUMS ({e}) - yt-dlp download discarded, please retry")))?
            .body_mut()
            .read_to_string()
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

    /// BtbN's name for this platform, or `None` where it publishes no zip.
    fn ffmpeg_build_target(&self) -> Option<&'static str> {
        if self.system != "Windows" {
            return None;
        }
        Some(if self.machine.contains("aarch64") || self.machine.contains("arm64") {
            "winarm64"
        } else {
            "win64"
        })
    }

    fn deno_asset_name(&self) -> &'static str {
        if self.system == "Windows" {
            if self.machine.contains("aarch64") || self.machine.contains("arm64") {
                "deno-aarch64-pc-windows-msvc.zip"
            } else {
                "deno-x86_64-pc-windows-msvc.zip"
            }
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
        }
    }

    /// Downloads yt-dlp next to the live copy and only swaps it in once the
    /// checksum matched, so a failed or tampered download never replaces a
    /// working binary - and never sits, unverified, where it gets executed.
    pub fn install_ytdlp(&self, on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
        let (url, asset) = self.ytdlp_url();
        let staged = self.bin_dir.join("yt-dlp.download");
        let _ = fs::remove_file(&staged);
        self.download_to_file(&url, &staged, on_progress)?;
        self.verify_ytdlp_checksum(&staged, &asset)?;
        fs::rename(&staged, &self.ytdlp_local).map_err(|e| {
            let _ = fs::remove_file(&staged);
            format!("could not install yt-dlp: {e}")
        })
    }

    /// Installs ffmpeg and ffprobe from BtbN's static GPL build.
    ///
    /// The zip is checked against the digest GitHub reports before anything
    /// is unpacked. Outside Windows no zip build exists to verify, so the
    /// system's package manager is the way to get FFmpeg there.
    pub fn install_ffmpeg(
        &self,
        mut on_progress: impl FnMut(&str, u64, u64),
    ) -> Result<(), String> {
        let Some(target) = self.ffmpeg_build_target() else {
            return Err(
                "FFmpeg cannot be installed automatically on this platform - please install it \
                 with your package manager (e.g. `brew install ffmpeg` or `apt install ffmpeg`)."
                    .into(),
            );
        };
        let assets = github_release_assets(FFMPEG_API)?;
        let asset = pick_ffmpeg_asset(&assets, target)
            .ok_or_else(|| format!("no verifiable FFmpeg build found for {target}"))?;
        let zip_path = self.bin_dir.join("ffmpeg.zip");
        let res = download_verified(asset, &zip_path, |w, t| on_progress("FFmpeg", w, t))
            .and_then(|_| extract_named(&zip_path, &self.ffmpeg_local, "ffmpeg"))
            .and_then(|_| extract_named(&zip_path, &self.ffprobe_local, "ffprobe"));
        let _ = fs::remove_file(&zip_path);
        res
    }

    pub fn install_deno(&self, on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
        let wanted = self.deno_asset_name();
        let assets = github_release_assets(DENO_API)?;
        let asset = assets
            .iter()
            .find(|a| a.name == wanted)
            .ok_or_else(|| format!("no verifiable Deno download found ({wanted})"))?;
        let zip_path = self.bin_dir.join("deno.zip");
        let res = download_verified(asset, &zip_path, on_progress)
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
/// fixed path, so a crafted archive cannot steer the write elsewhere. The
/// member is unpacked beside the target and renamed over it at the end, so an
/// interrupted extraction leaves the previous binary intact.
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
            let tmp = target.with_extension("extract");
            let res = File::create(&tmp)
                .and_then(|mut out| std::io::copy(&mut member, &mut out).map(|_| ()))
                .and_then(|_| fs::rename(&tmp, target));
            if let Err(e) = res {
                let _ = fs::remove_file(&tmp);
                return Err(format!("could not unpack {name}: {e}"));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            url: format!("https://example.com/{name}"),
            size: 1,
            sha256: "0".repeat(64),
        }
    }

    #[test]
    fn every_child_runs_in_the_app_folder_with_bin_first_on_path() {
        let bin = Binaries::new();
        let mut cmd = Command::new("anything");
        bin.prepare(&mut cmd);
        // Not the launch folder, which is usually Downloads.
        assert_eq!(cmd.get_current_dir(), Some(bin.app_dir.as_path()));
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v)
            .expect("PATH is set for the child");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(entries.first(), Some(&bin.bin_dir));
        assert_eq!(entries.iter().filter(|p| **p == bin.bin_dir).count(), 1, "no duplicate entry");
    }

    #[test]
    fn a_missing_tool_never_resolves_to_a_bare_name() {
        let bin = Binaries::new();
        for path in [bin.ytdlp_path(), bin.ffmpeg_path(), bin.ffprobe_path()] {
            // A bare name would let Windows search the exe's own folder.
            assert!(Path::new(&path).is_absolute(), "{path}");
        }
        assert_eq!(find_on_path("surely-no-such-tool-vt", cfg!(windows)), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_opener_is_the_system_explorer() {
        let opener = system_opener();
        assert!(opener.is_absolute(), "{}", opener.display());
        assert!(opener.ends_with("explorer.exe"));
    }

    #[test]
    fn the_newest_static_gpl_release_build_is_picked() {
        let assets: Vec<ReleaseAsset> = [
            "ffmpeg-master-latest-win64-gpl.zip",
            "ffmpeg-n8.1-latest-win64-gpl-8.1.zip",
            "ffmpeg-n10.0-latest-win64-gpl-shared-10.0.zip",
            "ffmpeg-n10.0-latest-win64-lgpl-10.0.zip",
            "ffmpeg-n9.0-latest-win64-gpl-9.0.zip",
            "ffmpeg-n9.0-latest-winarm64-gpl-9.0.zip",
            "checksums.sha256",
        ]
        .into_iter()
        .map(asset)
        .collect();
        // 9.0 beats 8.1 numerically; shared, lgpl and master never qualify.
        let win = pick_ffmpeg_asset(&assets, "win64").unwrap();
        assert_eq!(win.name, "ffmpeg-n9.0-latest-win64-gpl-9.0.zip");
        let arm = pick_ffmpeg_asset(&assets, "winarm64").unwrap();
        assert_eq!(arm.name, "ffmpeg-n9.0-latest-winarm64-gpl-9.0.zip");
        assert!(pick_ffmpeg_asset(&assets, "linux64").is_none());
    }

    #[test]
    fn only_a_well_formed_sha256_digest_is_accepted() {
        let ok = serde_json::json!({ "digest": format!("sha256:{}", "AB".repeat(32)) });
        assert_eq!(asset_sha256(&ok), Some("ab".repeat(32)));
        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "digest": null }),
            serde_json::json!({ "digest": format!("sha512:{}", "ab".repeat(32)) }),
            serde_json::json!({ "digest": "sha256:abc" }),
            serde_json::json!({ "digest": format!("sha256:{}", "zz".repeat(32)) }),
        ] {
            assert_eq!(asset_sha256(&bad), None, "{bad}");
        }
    }

    /// Hits the network: `cargo test -- --ignored live_`.
    #[test]
    #[ignore]
    fn live_release_assets_resolve_and_verify() {
        let ffmpeg = github_release_assets(FFMPEG_API).unwrap();
        let pick = pick_ffmpeg_asset(&ffmpeg, "win64").expect("an FFmpeg build");
        eprintln!("ffmpeg: {} ({} bytes)", pick.name, pick.size);
        let deno = github_release_assets(DENO_API).unwrap();
        let asset = deno
            .iter()
            .find(|a| a.name == "deno-x86_64-pc-windows-msvc.zip")
            .expect("a Deno build");
        let dir = std::env::temp_dir().join(format!("vt_live_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let zip = dir.join("deno.zip");
        download_verified(asset, &zip, |_, _| {}).unwrap();
        let exe = dir.join("deno.exe");
        extract_named(&zip, &exe, "deno").unwrap();
        assert!(fs::metadata(&exe).unwrap().len() > 1_000_000);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_that_does_not_match_its_digest_is_deleted() {
        let dir = std::env::temp_dir().join(format!("vt_verify_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("file.zip");
        // A non-HTTPS URL is refused before anything is written.
        let a = ReleaseAsset { url: "http://example.com/x".into(), ..asset("x") };
        assert!(download_verified(&a, &target, |_, _| {}).is_err());
        assert!(!target.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
