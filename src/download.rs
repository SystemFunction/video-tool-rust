//! yt-dlp command building and the download worker (ported from _run_download).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use regex::Regex;

use crate::binaries::{no_window, Binaries};
use crate::consts::{IMPERSONATE_AUTO_HOSTS, YTDLP_INSTAGRAM_FIX};
use crate::emit::Emitter;
use crate::types::{ConflictDecision, ConflictReq, DownloadOpts, Task, UiMsg};
use crate::util;

const PROBE_TIMEOUT: Duration = Duration::from_secs(90);

type ChildSlot = Arc<Mutex<Option<Child>>>;

pub struct DlCtx {
    pub bin: Arc<Binaries>,
    pub em: Emitter,
    pub cancel: Arc<AtomicBool>,
    pub child: ChildSlot,
    pub js_runtime: String,
    pub ytdlp_version: String,
}

/// Builds the yt-dlp command line as (program, args...).
pub fn build_download_cmd(
    bin: &Binaries,
    url: &str,
    out_dir: &str,
    quality: &str,
    cookies: &Option<String>,
    opts: &DownloadOpts,
    outtmpl: Option<&str>,
    force_overwrite: bool,
) -> Vec<String> {
    let mut cmd: Vec<String> = vec![bin.ytdlp_path()];
    let push = |c: &mut Vec<String>, items: &[&str]| {
        for it in items {
            c.push((*it).to_string());
        }
    };

    match quality {
        "audio_wav" => push(
            &mut cmd,
            &[
                "-x",
                "--audio-format",
                "wav",
                "--postprocessor-args",
                "ExtractAudio:-ar 48000 -ac 2 -c:a pcm_s16le",
            ],
        ),
        "audio" => push(
            &mut cmd,
            &[
                "-x",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
                "--postprocessor-args",
                "ExtractAudio:-codec:a libmp3lame -b:a 320k -ar 44100 -ac 2",
            ],
        ),
        "audio_opus" => push(
            &mut cmd,
            &["-x", "--audio-format", "opus", "--audio-quality", "0"],
        ),
        _ => {
            push(
                &mut cmd,
                &[
                    "--merge-output-format",
                    "mp4",
                    "--postprocessor-args",
                    "Merger:-c:a aac -b:a 192k -ar 48000 -ac 2 -movflags +faststart",
                    "--postprocessor-args",
                    "Metadata:-movflags +faststart",
                    "--postprocessor-args",
                    "EmbedThumbnail:-movflags +faststart",
                ],
            );
            if quality == "best_av1" {
                push(
                    &mut cmd,
                    &[
                        "-f",
                        "bv*[vcodec^=av01]+ba/bv*[vcodec^=avc1]+ba[acodec^=mp4a]/bv*[vcodec^=avc1]+ba/bv*+ba/b",
                        "-S",
                        "vcodec:av01,res,acodec:m4a",
                    ],
                );
            } else if quality == "best" {
                push(
                    &mut cmd,
                    &[
                        "-f",
                        "bv*[vcodec^=avc1]+ba[acodec^=mp4a]/bv*[vcodec^=avc1]+ba/bv*+ba/b",
                        "-S",
                        "vcodec:h264,res,acodec:m4a",
                    ],
                );
            } else {
                let f = format!(
                    "bv*[vcodec^=avc1][height<={q}]+ba[acodec^=mp4a]/bv*[vcodec^=avc1][height<={q}]+ba/bv*[height<={q}]+ba/bv*+ba/b",
                    q = quality
                );
                let s = format!("res:{q},vcodec:h264,acodec:m4a", q = quality);
                cmd.push("-f".into());
                cmd.push(f);
                cmd.push("-S".into());
                cmd.push(s);
            }
        }
    }

    // Base args
    let default_tmpl = Path::new(out_dir)
        .join("%(title).200B.%(ext)s")
        .to_string_lossy()
        .to_string();
    let mut base: Vec<String> = vec![
        "--newline".into(),
        "--no-playlist".into(),
        "--retries".into(),
        "10".into(),
        "--fragment-retries".into(),
        "10".into(),
        "--concurrent-fragments".into(),
        "8".into(),
        "--extractor-retries".into(),
        "3".into(),
        "--throttled-rate".into(),
        "100K".into(),
        "--sleep-requests".into(),
        "1".into(),
        "--no-mtime".into(),
        "--progress-template".into(),
        "download:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s".into(),
        "-o".into(),
        outtmpl.map(|s| s.to_string()).unwrap_or(default_tmpl),
    ];
    if force_overwrite {
        base.push("--force-overwrites".into());
    }

    // YouTube player clients
    let has_cookies = cookies.is_some() || !opts.cookiefile.is_empty();
    let player_clients = if opts.potoken {
        "mweb,tv_simply,web_safari,web_embedded,web_creator"
    } else if has_cookies {
        "default,web_safari,web_creator,web_embedded"
    } else {
        "web_safari,default"
    };
    base.push("--extractor-args".into());
    base.push(format!("youtube:player_client={player_clients}"));

    if !opts.plugins_dir.is_empty() {
        base.push("--plugin-dirs".into());
        base.push("default".into());
        base.push("--plugin-dirs".into());
        base.push(opts.plugins_dir.clone());
    }
    if !opts.potoken_url.trim().is_empty() {
        base.push("--extractor-args".into());
        base.push(format!(
            "youtubepot-bgutilhttp:base_url={}",
            opts.potoken_url.trim()
        ));
    }

    if opts.embed {
        base.push("--embed-thumbnail".into());
        base.push("--embed-metadata".into());
        base.push("--embed-chapters".into());
    }

    let is_audio = matches!(quality, "audio" | "audio_wav" | "audio_opus");
    if opts.subs && !is_audio {
        let langs = if opts.subs_lang.is_empty() {
            "en,de".to_string()
        } else {
            opts.subs_lang.clone()
        };
        base.push("--write-subs".into());
        base.push("--write-auto-subs".into());
        base.push("--sub-langs".into());
        base.push(langs);
        base.push("--embed-subs".into());
        base.push("--convert-subs".into());
        base.push("srt".into());
    }

    if opts.sponsorblock {
        base.push("--sponsorblock-remove".into());
        base.push("sponsor,selfpromo,interaction".into());
        base.push("--sponsorblock-mark".into());
        base.push("all".into());
    }

    // Anti-bot impersonation (auto-enabled for hard hosts).
    let mut impersonate = opts.impersonate;
    if !impersonate && opts.impersonate_available {
        let lo = url.to_lowercase();
        if IMPERSONATE_AUTO_HOSTS.iter().any(|h| lo.contains(h)) {
            impersonate = true;
        }
    }
    if impersonate {
        base.push("--impersonate".into());
        base.push("chrome".into());
    }

    // Cookies (prepended, mirroring the original ordering).
    if !opts.cookiefile.is_empty() {
        let mut prefixed = vec!["--cookies".to_string(), opts.cookiefile.clone()];
        prefixed.append(&mut base);
        base = prefixed;
    } else if let Some(c) = cookies {
        let mut prefixed = vec!["--cookies-from-browser".to_string(), c.clone()];
        prefixed.append(&mut base);
        base = prefixed;
    }

    cmd.append(&mut base);
    cmd.push(url.to_string());
    cmd
}

fn spawn(bin: &Binaries, args: &[String]) -> std::io::Result<Command> {
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    bin.apply_env(&mut cmd);
    no_window(&mut cmd);
    Ok(cmd)
}

/// Resolves the file yt-dlp *would* write (via --print filename), or None.
fn probe_target_path(
    ctx: &DlCtx,
    url: &str,
    out_dir: &str,
    quality: &str,
    cookies: &Option<String>,
    opts: &DownloadOpts,
) -> Option<PathBuf> {
    let cmd = build_download_cmd(&ctx.bin, url, out_dir, quality, cookies, opts, None, false);
    // Same command, but simulated and printing only the resulting file name.
    let mut probe: Vec<String> = cmd[..cmd.len() - 1].to_vec();
    probe.extend([
        "--simulate".into(),
        "--no-warnings".into(),
        "--print".into(),
        "filename".into(),
        cmd[cmd.len() - 1].clone(),
    ]);

    let mut command = spawn(&ctx.bin, &probe).ok()?;
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    *ctx.child.lock().unwrap() = Some(child);

    let reader = BufReader::new(stdout);
    let mut lines: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    for line in reader.lines().map_while(Result::ok) {
        let t = line.trim().to_string();
        if !t.is_empty() {
            lines.push(t);
        }
        if ctx.cancel.load(Ordering::SeqCst) || std::time::Instant::now() > deadline {
            break;
        }
    }

    let mut guard = ctx.child.lock().unwrap();
    let status = guard.take().map(|mut c| {
        let _ = c.kill();
        c.wait()
    });
    drop(guard);
    let ok = matches!(status, Some(Ok(s)) if s.success());
    if !ok || ctx.cancel.load(Ordering::SeqCst) {
        // yt-dlp exits non-zero on kill; only trust output when it succeeded.
        if lines.is_empty() {
            return None;
        }
    }

    let last = lines.last()?;
    let mut target = PathBuf::from(last);
    if let Some(ext) = util::audio_ext_by_quality(quality) {
        let cur = target
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if cur != ext {
            target.set_extension(ext);
        }
    }
    Some(target)
}

fn log_preflight_hints(ctx: &DlCtx, url: &str, quality: &str, cookies: &Option<String>, opts: &DownloadOpts) {
    let lo = url.to_lowercase();
    let has_cookies = cookies.is_some() || !opts.cookiefile.is_empty();
    let is_audio = matches!(quality, "audio" | "audio_wav" | "audio_opus");
    if (lo.contains("youtube.com") || lo.contains("youtu.be")) && !is_audio {
        if ctx.js_runtime.is_empty() {
            ctx.em.log(
                Task::Download,
                "Warning: no JS runtime (Deno) found - YouTube often only offers 360p without one. Setup tab -> 'Install Deno'.",
            );
        } else if !has_cookies && !opts.potoken {
            ctx.em.log(
                Task::Download,
                "Note: without cookies or a PO token, YouTube may withhold some HD formats.",
            );
        }
    }
    if lo.contains("instagram.com") {
        if util::ytdlp_older_than(&ctx.ytdlp_version, YTDLP_INSTAGRAM_FIX) {
            ctx.em.log(
                Task::Download,
                format!(
                    "Warning: yt-dlp {} is too old for Instagram (empty media response bug, #17074). Setup tab -> 'Update yt-dlp'.",
                    ctx.ytdlp_version
                ),
            );
        }
        if !has_cookies {
            ctx.em.log(
                Task::Download,
                "Note: Instagram often requires login cookies (cookies.txt is the most reliable).",
            );
        }
        if !opts.impersonate_available {
            ctx.em.log(
                Task::Download,
                "Warning: browser impersonation is unavailable - Instagram usually blocks downloads without it.",
            );
        }
    }
}

/// (outtmpl, force_overwrite, cancelled)
fn resolve_conflict(
    ctx: &DlCtx,
    url: &str,
    out_dir: &str,
    quality: &str,
    cookies: &Option<String>,
    opts: &DownloadOpts,
) -> (Option<String>, bool, bool) {
    match opts.conflict.as_str() {
        "overwrite" => return (None, true, false),
        "skip" => return (None, false, false),
        _ => {}
    }

    ctx.em.status(Task::Download, "Checking target file ...");
    let target = probe_target_path(ctx, url, out_dir, quality, cookies, opts);
    if ctx.cancel.load(Ordering::SeqCst) {
        return (None, false, true);
    }
    let target = match target {
        Some(t) => t,
        None => {
            ctx.em.log(
                Task::Download,
                "Note: could not determine the target file name in advance - if a file with the same name exists, yt-dlp will skip the download.",
            );
            return (None, false, false);
        }
    };
    if !target.exists() {
        ctx.em.log(
            Task::Download,
            format!("Target file: {}", file_name(&target)),
        );
        return (None, false, false);
    }

    if opts.conflict == "rename" {
        let new_path = util::unique_path(&target);
        ctx.em.log(
            Task::Download,
            format!(
                "\"{}\" already exists - saving as \"{}\".",
                file_name(&target),
                file_name(&new_path)
            ),
        );
        return (Some(util::outtmpl_for(&new_path)), false, false);
    }

    // "ask" -> hand the decision to the UI and block until it answers.
    let suggestion = util::unique_path(&target);
    let (reply_tx, reply_rx) = mpsc::channel::<ConflictDecision>();
    ctx.em.status(Task::Download, "Waiting for your decision ...");
    ctx.em.send(UiMsg::Conflict(ConflictReq {
        target: target.clone(),
        suggestion: file_name(&suggestion),
        reply: reply_tx,
    }));

    loop {
        if ctx.cancel.load(Ordering::SeqCst) {
            return (None, false, true);
        }
        match reply_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(ConflictDecision::Overwrite) => {
                ctx.em.log(
                    Task::Download,
                    format!("Overwriting the existing \"{}\".", file_name(&target)),
                );
                return (None, true, false);
            }
            Ok(ConflictDecision::Rename(p)) => {
                ctx.em
                    .log(Task::Download, format!("Saving as \"{}\".", file_name(&p)));
                return (Some(util::outtmpl_for(&p)), false, false);
            }
            Ok(ConflictDecision::Skip) => return (None, false, true),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return (None, false, true),
        }
    }
}

/// The download worker (runs on its own thread).
pub fn run_download(
    ctx: DlCtx,
    url: String,
    out_dir: String,
    quality: String,
    cookies: Option<String>,
    opts: DownloadOpts,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_download_inner(&ctx, &url, &out_dir, &quality, &cookies, &opts);
    }));
    if result.is_err() {
        ctx.em.log(Task::Download, "Error: internal download worker panic");
        ctx.em.status(Task::Download, "Download failed");
    }
    *ctx.child.lock().unwrap() = None;
    ctx.em.busy(Task::Download, false);
}

fn run_download_inner(
    ctx: &DlCtx,
    url: &str,
    out_dir: &str,
    quality: &str,
    cookies: &Option<String>,
    opts: &DownloadOpts,
) {
    log_preflight_hints(ctx, url, quality, cookies, opts);

    let (outtmpl, force_overwrite, cancelled) =
        resolve_conflict(ctx, url, out_dir, quality, cookies, opts);
    if cancelled {
        let stopped = ctx.cancel.load(Ordering::SeqCst);
        ctx.em.progress(Task::Download, -1.0);
        if stopped {
            ctx.em.status(Task::Download, "Cancelled");
            ctx.em.log(Task::Download, "\n=== Download cancelled ===");
        } else {
            ctx.em
                .status(Task::Download, "Skipped - file already exists");
            ctx.em
                .log(Task::Download, "\n=== Skipped - the existing file was kept ===");
            ctx.em
                .toast("Skipped - the existing file was kept", true);
        }
        return;
    }

    let cmd = build_download_cmd(
        &ctx.bin,
        url,
        out_dir,
        quality,
        cookies,
        opts,
        outtmpl.as_deref(),
        force_overwrite,
    );
    ctx.em.status(Task::Download, "Downloading ...");

    let mut command = match spawn(&ctx.bin, &cmd) {
        Ok(c) => c,
        Err(e) => {
            ctx.em.log(Task::Download, format!("Error: {e}"));
            ctx.em.status(Task::Download, "Download failed");
            return;
        }
    };
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            ctx.em.log(Task::Download, format!("Error: {e}"));
            ctx.em.status(Task::Download, "Download failed");
            return;
        }
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    *ctx.child.lock().unwrap() = Some(child);

    // Forward stderr to the log on a helper thread.
    let em_err = ctx.em.clone();
    let err_handle = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let t = line.trim();
            if !t.is_empty() {
                em_err.log(Task::Download, t.to_string());
            }
        }
    });

    let pct_re = Regex::new(r"(\d+(?:\.\d+)?)").unwrap();
    let generic_re = Regex::new(r"(\d+(?:\.\d+)?)%").unwrap();
    let mut last_progress = String::new();
    let mut skipped_existing = false;
    let mut all_logs: Vec<String> = Vec::new();

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let is_progress = line.starts_with("download:");
        if !(is_progress && line == last_progress) {
            ctx.em.log(Task::Download, line.clone());
            all_logs.push(line.to_lowercase());
        }
        if is_progress {
            last_progress = line.clone();
        }
        if line.to_lowercase().contains("has already been downloaded") {
            skipped_existing = true;
        }

        if is_progress {
            let rest = &line["download:".len()..];
            let parts: Vec<&str> = rest.split('|').map(|p| p.trim()).collect();
            if parts.len() >= 3 {
                if let Some(m) = pct_re.captures(parts[0]) {
                    if let Ok(p) = m[1].parse::<f32>() {
                        ctx.em.progress(Task::Download, (p / 100.0).clamp(0.0, 1.0));
                        ctx.em.status(
                            Task::Download,
                            format!("{}%  |  {}  |  ETA {}", &m[1], parts[1], parts[2]),
                        );
                    }
                }
            }
        } else if let Some(m) = generic_re.captures(&line) {
            if let Ok(p) = m[1].parse::<f32>() {
                ctx.em.progress(Task::Download, (p / 100.0).clamp(0.0, 1.0));
                ctx.em
                    .status(Task::Download, format!("Downloading ... {}%", &m[1]));
            }
        }
    }
    err_handle.join().ok();

    let code = {
        let mut guard = ctx.child.lock().unwrap();
        match guard.take() {
            Some(mut c) => c.wait().ok().and_then(|s| s.code()).unwrap_or(-1),
            None => -1, // Stop already killed it
        }
    };

    let cancelled = ctx.cancel.load(Ordering::SeqCst);
    if cancelled {
        ctx.em.status(Task::Download, "Cancelled");
        ctx.em.log(Task::Download, "\n=== Download cancelled ===");
    } else if code == 0 && skipped_existing {
        ctx.em.progress(Task::Download, -1.0);
        ctx.em
            .status(Task::Download, "Nothing downloaded - file already exists");
        ctx.em.log(
            Task::Download,
            "\n=== Nothing downloaded - a file with this name already exists ===",
        );
        ctx.em.log(
            Task::Download,
            "Tip: set 'If file exists' to 'Ask me', 'Auto-rename' or 'Overwrite' to download it anyway.",
        );
        ctx.em
            .toast("Nothing downloaded - file already exists", true);
    } else if code == 0 {
        ctx.em.progress(Task::Download, 1.0);
        ctx.em.status(Task::Download, "Download completed");
        ctx.em.log(Task::Download, "\n=== Download successful ===");
        ctx.em.toast("Download successful", false);
    } else {
        ctx.em.status(Task::Download, "Download failed");
        ctx.em
            .log(Task::Download, format!("\n=== Download failed (code {code}) ==="));
        let joined = all_logs.join("\n");
        if joined.contains("empty media response") {
            ctx.em.log(
                Task::Download,
                format!(
                    "Tip: known Instagram \"empty media response\" bug (yt-dlp #17074), fixed in {}. Update yt-dlp in the Setup tab.",
                    YTDLP_INSTAGRAM_FIX
                ),
            );
        } else if joined.contains("requested format is not available")
            || joined.contains("please sign in")
            || joined.contains("login required")
        {
            ctx.em.log(
                Task::Download,
                "Tip: the site withheld formats or wants a login. Set cookies (cookies.txt), install Deno, or update yt-dlp.",
            );
        } else if ctx.js_runtime.is_empty() {
            ctx.em.log(
                Task::Download,
                "Tip: no JS runtime detected. Setup tab -> 'Install Deno' (needed for the YouTube n-challenge).",
            );
        }
        ctx.em.toast("Download failed", true);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}
