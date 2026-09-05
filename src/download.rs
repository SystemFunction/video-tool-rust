//! yt-dlp command building and the download worker (ported from _run_download).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use regex::Regex;

use crate::binaries::{no_window, Binaries};
use crate::consts::{ANON_NAME_HOSTS, IMPERSONATE_AUTO_HOSTS, YTDLP_INSTAGRAM_FIX};
use crate::emit::Emitter;
use crate::i18n::{t, tf, Lang};
use crate::types::{ConflictDecision, ConflictReq, DownloadOpts, Outcome, Task, UiMsg};
use crate::util;

const PROBE_TIMEOUT: Duration = Duration::from_secs(90);

/// How many runs a single download gets: the normal one plus two retries.
const MAX_ATTEMPTS: usize = 3;

/// How the very first run is configured - no retry concessions yet.
const FIRST_ATTEMPT: Retry = Retry {
    safe_clients: false,
    resume: false,
    low_concurrency: false,
};

/// Breather between attempts - retrying a rejection instantly tends to earn
/// another one.
const RETRY_PAUSE: Duration = Duration::from_secs(3);

type ChildSlot = Arc<Mutex<Option<Child>>>;

pub struct DlCtx {
    pub bin: Arc<Binaries>,
    pub em: Emitter,
    pub cancel: Arc<AtomicBool>,
    pub child: ChildSlot,
    pub js_runtime: String,
    pub ytdlp_version: String,
    pub lang: Lang,
}

impl DlCtx {
    fn t(&self, key: &'static str) -> &'static str {
        t(self.lang, key)
    }
    fn tf(&self, key: &'static str, args: &[&str]) -> String {
        tf(self.lang, key, args)
    }
}

/// How a repeated attempt differs from the first one.
///
/// YouTube hands out media URLs that can stop working part-way through the
/// transfer, so a retry is only useful if it extracts fresh ones - and if it
/// keeps what the interrupted attempt already wrote.
#[derive(Clone, Copy, Default)]
pub struct Retry {
    /// Drop the web player clients, whose URLs are the ones that die.
    pub safe_clients: bool,
    /// Keep the partial file even when the user asked to overwrite.
    pub resume: bool,
    /// Fetch the fragments one at a time. Eight parallel range requests on a
    /// URL the CDN is already unhappy with only invite another rejection.
    pub low_concurrency: bool,
}

/// Why a run failed, as far as it changes what the next one should do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    /// The CDN turned the transfer down (403). The URLs are the problem.
    Rejected,
    /// The connection or the server gave out - the same formats are still fine.
    Flaky,
}

/// The run that follows `prev` after a `kind` failure.
fn next_retry(prev: Retry, kind: Failure) -> Retry {
    Retry {
        // Re-extracting from the same web clients hands back the same SABR
        // formats, so a rejection goes straight to the clients that still
        // serve plain URLs instead of spending a run on an identical attempt.
        safe_clients: prev.safe_clients || kind == Failure::Rejected,
        resume: true,
        low_concurrency: true,
    }
}

/// What this particular run should write, and how hard it should try.
#[derive(Clone, Copy, Default)]
pub struct Attempt<'a> {
    /// Output template replacing the default one, e.g. a renamed target.
    pub outtmpl: Option<&'a str>,
    pub force_overwrite: bool,
    pub retry: Retry,
}

/// Builds the yt-dlp command line as (program, args...).
pub fn build_download_cmd(
    bin: &Binaries,
    url: &str,
    out_dir: &str,
    quality: &str,
    cookies: &Option<String>,
    opts: &DownloadOpts,
    attempt: Attempt,
) -> Vec<String> {
    let Attempt { outtmpl, force_overwrite, retry } = attempt;
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

    // Base args. The directory is a literal path but the template language
    // treats '%' as a field marker, so a folder like "50% off" would expand
    // into something unrelated - escape it before appending the pattern.
    let default_tmpl = {
        // Escaping only doubles '%', so joining the pattern on afterwards is
        // still a plain path join.
        let mut tmpl = PathBuf::from(util::escape_outtmpl(&Path::new(out_dir).to_string_lossy()));
        if opts.playlist {
            // A playlist drops dozens of files into the folder, so it gets one
            // of its own. The comma chain picks the first field the extractor
            // actually filled; the default keeps the path from collapsing when
            // it filled none.
            tmpl.push("%(playlist_title,playlist_id,uploader|Playlist)s");
            tmpl.push("%(playlist_index)s - %(title).150B.%(ext)s");
        } else {
            tmpl.push("%(title).200B.%(ext)s");
        }
        tmpl.to_string_lossy().to_string()
    };
    let mut base: Vec<String> = vec![
        "--newline".into(),
        if opts.playlist { "--yes-playlist".into() } else { "--no-playlist".into() },
        "--retries".into(),
        "10".into(),
        "--fragment-retries".into(),
        "10".into(),
        "--concurrent-fragments".into(),
        if retry.low_concurrency { "1".into() } else { "8".into() },
        "--extractor-retries".into(),
        "3".into(),
        "--throttled-rate".into(),
        "100K".into(),
        "--sleep-requests".into(),
        "1".into(),
        "--no-mtime".into(),
        // Without this yt-dlp writes its screen output in the ANSI code page
        // (cp1252 on a German Windows), so a title carrying anything outside
        // that page arrived as bytes we cannot decode. The frozen build
        // ignores PYTHONIOENCODING, hence the option rather than an env var -
        // see `util::lossy_lines` for why such a line used to be fatal.
        "--encoding".into(),
        "utf-8".into(),
        "--progress-template".into(),
        "download:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s".into(),
        "-o".into(),
        outtmpl.map(|s| s.to_string()).unwrap_or(default_tmpl),
    ];
    if opts.playlist && !opts.playlist_items.trim().is_empty() {
        base.push("--playlist-items".into());
        base.push(opts.playlist_items.trim().to_string());
    }
    if !opts.rate_limit.trim().is_empty() {
        base.push("--limit-rate".into());
        base.push(opts.rate_limit.trim().to_string());
    }
    if force_overwrite {
        base.push("--force-overwrites".into());
        // --force-overwrites implies --no-continue. On a retry the half-written
        // file is from our own interrupted attempt, so resuming it is right;
        // the later option wins, and overwriting stays on.
        if retry.resume {
            base.push("--continue".into());
        }
    }

    // YouTube player clients
    let has_cookies = cookies.is_some() || !opts.cookiefile.is_empty();
    let player_clients = if retry.safe_clients {
        // Unless a PO token is supplied, YouTube pushes the web clients onto
        // SABR streaming: most of their formats come back without a URL, and
        // the few that keep one are bound to a token we do not have, so the
        // CDN starts answering 403 a few percent into the transfer. The
        // default rotation and the VR client still serve plain URLs.
        "default,android_vr"
    } else if opts.potoken {
        "mweb,tv_simply,web_safari,web_embedded,web_creator"
    } else if has_cookies {
        "default,web_safari,web_creator,web_embedded"
    } else {
        // Default first: yt-dlp's own rotation picks clients that actually
        // hand out downloadable URLs, and whatever it finds must not be
        // shadowed by a same-numbered SABR format from web_safari.
        "default,web_safari"
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
        // WAV has no container slot for cover art - yt-dlp aborts the whole
        // postprocessing chain with "Supported filetypes for thumbnail
        // embedding are: ..." if we ask for it anyway.
        if quality != "audio_wav" {
            base.push("--embed-thumbnail".into());
        }
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

/// For hosts that put the uploader's name into the title, returns the output
/// template for a neutral "video-1234.<ext>" plus that stem for the log.
///
/// The number is drawn once per download so the pre-flight name probe and the
/// download itself agree on the target.
fn anon_outtmpl(url: &str, out_dir: &str) -> Option<(String, String)> {
    let lo = url.to_lowercase();
    if !ANON_NAME_HOSTS.iter().any(|h| lo.contains(h)) {
        return None;
    }
    let stem = util::pick_anon_stem(Path::new(out_dir));
    // The stem is "video-<digits>" and carries no '%', so only the directory
    // needs escaping before the template is joined onto it.
    let mut tmpl = PathBuf::from(util::escape_outtmpl(&Path::new(out_dir).to_string_lossy()));
    tmpl.push(format!("{stem}.%(ext)s"));
    Some((tmpl.to_string_lossy().to_string(), stem))
}

fn spawn(bin: &Binaries, args: &[String]) -> Command {
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    bin.apply_env(&mut cmd);
    no_window(&mut cmd);
    // No console is attached, so yt-dlp must never wait on stdin.
    cmd.stdin(Stdio::null());
    cmd
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
    let cmd = build_download_cmd(&ctx.bin, url, out_dir, quality, cookies, opts, Attempt::default());
    // Same command, but simulated and printing only the resulting file name.
    let mut probe: Vec<String> = cmd[..cmd.len() - 1].to_vec();
    probe.extend([
        "--simulate".into(),
        "--no-warnings".into(),
        "--print".into(),
        "filename".into(),
        cmd[cmd.len() - 1].clone(),
    ]);

    let mut command = spawn(&ctx.bin, &probe);
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    *util::lock(&ctx.child) = Some(child);

    // Read on a helper thread. Reading inline would block in `lines()` until
    // yt-dlp produced output, so an extractor that stalls without printing
    // anything would hang here forever - the deadline below was unreachable.
    let (line_tx, line_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in util::lossy_lines(stdout) {
            let t = line.trim().to_string();
            if !t.is_empty() && line_tx.send(t).is_err() {
                return;
            }
        }
    });

    let mut lines: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    loop {
        if ctx.cancel.load(Ordering::SeqCst) || std::time::Instant::now() >= deadline {
            break;
        }
        match line_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => lines.push(line),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    // Take the child out of the slot before waiting on it, so the Stop button
    // on the UI thread never blocks behind a `wait()`.
    let child = util::lock(&ctx.child).take();
    let status = child.map(|mut c| {
        let _ = c.kill();
        c.wait()
    });
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
            ctx.em.log(Task::Download, ctx.t("dlw.warn_nojs"));
        } else if !has_cookies && !opts.potoken {
            ctx.em.log(Task::Download, ctx.t("dlw.note_nocookies"));
        }
    }
    if lo.contains("instagram.com") {
        if util::ytdlp_older_than(&ctx.ytdlp_version, YTDLP_INSTAGRAM_FIX) {
            ctx.em.log(
                Task::Download,
                ctx.tf("dlw.warn_ig_old", &[&ctx.ytdlp_version]),
            );
        }
        if !has_cookies {
            ctx.em.log(Task::Download, ctx.t("dlw.note_ig_cookies"));
        }
        if !opts.impersonate_available {
            ctx.em.log(Task::Download, ctx.t("dlw.warn_ig_impersonate"));
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

    ctx.em.status(Task::Download, ctx.t("dlw.checking_target"));
    let target = probe_target_path(ctx, url, out_dir, quality, cookies, opts);
    if ctx.cancel.load(Ordering::SeqCst) {
        return (None, false, true);
    }
    let target = match target {
        Some(t) => t,
        None => {
            ctx.em.log(Task::Download, ctx.t("dlw.no_target_name"));
            return (None, false, false);
        }
    };
    if !target.exists() {
        ctx.em.log(
            Task::Download,
            ctx.tf("dlw.target_file", &[&file_name(&target)]),
        );
        return (None, false, false);
    }

    if opts.conflict == "rename" {
        let new_path = util::unique_path(&target);
        ctx.em.log(
            Task::Download,
            ctx.tf(
                "dlw.exists_saving_as",
                &[&file_name(&target), &file_name(&new_path)],
            ),
        );
        return (Some(util::outtmpl_for(&new_path)), false, false);
    }

    // "ask" -> hand the decision to the UI and block until it answers.
    let suggestion = util::unique_path(&target);
    let (reply_tx, reply_rx) = mpsc::channel::<ConflictDecision>();
    ctx.em.status(Task::Download, ctx.t("dlw.waiting_decision"));
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
                    ctx.tf("dlw.overwriting", &[&file_name(&target)]),
                );
                return (None, true, false);
            }
            Ok(ConflictDecision::Rename(p)) => {
                ctx.em
                    .log(Task::Download, ctx.tf("dlw.saving_as", &[&file_name(&p)]));
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
        ctx.em.log(Task::Download, ctx.t("dlw.panic"));
        ctx.em.status(Task::Download, ctx.t("dlw.failed"));
    }
    *util::lock(&ctx.child) = None;
    ctx.em.busy(Task::Download, false);
}

/// What one yt-dlp run ended up doing.
struct RunOutcome {
    code: i32,
    skipped_existing: bool,
    /// Everything the run printed on either stream, lowercased.
    logs: String,
    /// The file the run says it wrote, when one of its lines named it.
    produced: Option<PathBuf>,
}

/// Pulls the file yt-dlp says it wrote out of one of its own log lines.
///
/// yt-dlp announces every destination it opens, so the *last* line of a run
/// that names one is the file the user ends up with: the per-stream downloads
/// come first, the merge or the audio extraction that replaces them last.
pub fn output_path_from_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('[')?;
    let (_tag, after) = rest.split_once("] ")?;
    let unquote = |s: &str| s.trim().trim_matches('"').trim().to_string();
    let candidate = if let Some(p) = after.strip_prefix("Destination: ") {
        unquote(p)
    } else if let Some(p) = after.strip_prefix("Merging formats into ") {
        unquote(p)
    } else if let Some(p) = after.strip_prefix("Moving file ") {
        // "Moving file \"from\" to \"to\"" - only the destination matters.
        unquote(p.rsplit_once(" to ")?.1)
    } else if let Some(p) = after.strip_suffix(" has already been downloaded") {
        unquote(p)
    } else {
        return None;
    };
    (!candidate.is_empty()).then_some(candidate)
}

/// Runs yt-dlp once, feeding progress and log lines to the UI.
///
/// `None` means the process could not be started at all - status and log
/// already say so.
fn run_ytdlp(ctx: &DlCtx, cmd: &[String]) -> Option<RunOutcome> {
    let mut command = spawn(&ctx.bin, cmd);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            ctx.em
                .log(Task::Download, ctx.tf("common.error", &[&e.to_string()]));
            ctx.em.status(Task::Download, ctx.t("dlw.failed"));
            return None;
        }
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        ctx.em.status(Task::Download, ctx.t("dlw.failed"));
        return None;
    };
    *util::lock(&ctx.child) = Some(child);

    // Forward stderr to the log on a helper thread. yt-dlp reports its errors
    // there, so the collected copy is what the diagnosis below reads.
    let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let em_err = ctx.em.clone();
    let err_lines = Arc::clone(&collected);
    let err_handle = thread::spawn(move || {
        for line in util::lossy_lines(stderr) {
            let t = line.trim();
            if !t.is_empty() {
                util::lock(&err_lines).push(t.to_lowercase());
                em_err.log(Task::Download, t.to_string());
            }
        }
    });

    let pct_re = Regex::new(r"(\d+(?:\.\d+)?)").unwrap();
    let generic_re = Regex::new(r"(\d+(?:\.\d+)?)%").unwrap();
    let mut last_progress = String::new();
    let mut skipped_existing = false;
    let mut produced: Option<PathBuf> = None;

    for line in util::lossy_lines(stdout) {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let is_progress = line.starts_with("download:");
        if !(is_progress && line == last_progress) {
            ctx.em.log(Task::Download, line.clone());
            util::lock(&collected).push(line.to_lowercase());
        }
        if is_progress {
            last_progress = line.clone();
        }
        if line.to_lowercase().contains("has already been downloaded") {
            skipped_existing = true;
        }
        if !is_progress {
            if let Some(p) = output_path_from_line(&line) {
                produced = Some(PathBuf::from(p));
            }
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
                            ctx.tf("dlw.progress_status", &[&m[1], parts[1], parts[2]]),
                        );
                    }
                }
            }
        } else if let Some(m) = generic_re.captures(&line) {
            if let Ok(p) = m[1].parse::<f32>() {
                ctx.em.progress(Task::Download, (p / 100.0).clamp(0.0, 1.0));
                ctx.em
                    .status(Task::Download, ctx.tf("dlw.downloading_pct", &[&m[1]]));
            }
        }
    }
    err_handle.join().ok();

    // Bound to a local first: a guard used directly as the match scrutinee
    // would stay alive for the whole match and hold the lock across `wait()`.
    let child = util::lock(&ctx.child).take();
    let code = match child {
        Some(mut c) => c.wait().ok().and_then(|s| s.code()).unwrap_or(-1),
        None => -1, // Stop already killed it
    };

    let logs = util::lock(&collected).join("\n");
    Some(RunOutcome {
        code,
        skipped_existing,
        logs,
        produced,
    })
}

/// Classifies a failed run, or `None` if a second attempt would be pointless.
///
/// The 403 is the interesting one: YouTube accepts the first few megabytes and
/// then rejects the rest of the transfer, which yt-dlp's own `--retries` cannot
/// fix because they reuse the very URL that just died. It is told apart from
/// the plain network hiccups because only it says something about the formats
/// that were picked.
fn failure_kind(logs: &str) -> Option<Failure> {
    const REJECTED: [&str; 2] = ["http error 403", "unable to download video data"];
    const FLAKY: [&str; 5] = [
        "http error 500",
        "http error 502",
        "http error 503",
        "connection reset",
        "the read operation timed out",
    ];
    if REJECTED.iter().any(|n| logs.contains(n)) {
        Some(Failure::Rejected)
    } else if FLAKY.iter().any(|n| logs.contains(n)) {
        Some(Failure::Flaky)
    } else {
        None
    }
}

/// Sleeps unless Stop is pressed; `false` means the wait was cut short.
fn sleep_cancellable(ctx: &DlCtx, total: Duration) -> bool {
    let deadline = std::time::Instant::now() + total;
    while std::time::Instant::now() < deadline {
        if ctx.cancel.load(Ordering::SeqCst) {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
    true
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

    // A freshly drawn "video-1234" is already known to be free, so the whole
    // conflict dance - including a second yt-dlp run just to learn the target
    // name - is skipped and the download starts right away.
    let anon = anon_outtmpl(url, out_dir);
    let (outtmpl, force_overwrite, cancelled) = match &anon {
        Some((tmpl, stem)) => {
            ctx.em.log(Task::Download, ctx.tf("dlw.anon_name", &[stem]));
            (Some(tmpl.clone()), false, false)
        }
        // A playlist writes many files, so probing for "the" target name would
        // only ever describe its first entry. yt-dlp's own per-file handling
        // takes over instead.
        None if opts.playlist => {
            ctx.em.log(Task::Download, ctx.t("dlw.playlist_mode"));
            (None, opts.conflict == "overwrite", false)
        }
        None => resolve_conflict(ctx, url, out_dir, quality, cookies, opts),
    };
    if cancelled {
        let stopped = ctx.cancel.load(Ordering::SeqCst);
        ctx.em.progress(Task::Download, -1.0);
        if stopped {
            ctx.em.status(Task::Download, ctx.t("status.cancelled"));
            ctx.em.log(Task::Download, ctx.t("dlw.cancelled_log"));
            ctx.em.send(UiMsg::Finished(Task::Download, Outcome::Cancelled));
        } else {
            ctx.em.status(Task::Download, ctx.t("dlw.skipped_status"));
            ctx.em.log(Task::Download, ctx.t("dlw.skipped_log"));
            ctx.em.toast(ctx.t("dlw.skipped_toast"), true);
            ctx.em.send(UiMsg::Finished(Task::Download, Outcome::Skipped));
        }
        return;
    }

    // The media URLs YouTube hands out can stop working part-way through the
    // transfer; a fresh run gets fresh ones, so a stalled 403 is retried here
    // instead of being reported as a dead end.
    let mut attempt = 0usize;
    let mut retry = FIRST_ATTEMPT;
    let (code, skipped_existing, joined, produced) = loop {
        let cmd = build_download_cmd(
            &ctx.bin,
            url,
            out_dir,
            quality,
            cookies,
            opts,
            Attempt {
                outtmpl: outtmpl.as_deref(),
                force_overwrite,
                retry,
            },
        );
        ctx.em.status(Task::Download, ctx.t("status.downloading"));

        let Some(run) = run_ytdlp(ctx, &cmd) else {
            return;
        };
        let spent = attempt + 1 >= MAX_ATTEMPTS || ctx.cancel.load(Ordering::SeqCst);
        let next = failure_kind(&run.logs).filter(|_| run.code != 0 && !spent);
        let Some(kind) = next else {
            break (run.code, run.skipped_existing, run.logs, run.produced);
        };
        attempt += 1;
        retry = next_retry(retry, kind);
        let msg = if kind == Failure::Rejected {
            "dlw.retrying"
        } else {
            "dlw.retrying_net"
        };
        ctx.em.log(
            Task::Download,
            ctx.tf(
                msg,
                &[&attempt.to_string(), &(MAX_ATTEMPTS - 1).to_string()],
            ),
        );
        ctx.em.status(Task::Download, ctx.t("dlw.retry_status"));
        if !sleep_cancellable(ctx, RETRY_PAUSE) {
            break (run.code, run.skipped_existing, run.logs, run.produced);
        }
    };

    // A renamed target is the file we asked for; the log line only confirms
    // it. When neither is known the outcome simply carries no path.
    let produced = produced.or_else(|| {
        outtmpl
            .as_deref()
            // `outtmpl_for` doubled every '%' for the template language;
            // stripping the extension field and halving them back gives the
            // literal path again.
            .map(|t| PathBuf::from(t.replace(".%(ext)s", "").replace("%%", "%")))
            .filter(|p| p.exists())
    });

    let cancelled = ctx.cancel.load(Ordering::SeqCst);
    if cancelled {
        ctx.em.status(Task::Download, ctx.t("status.cancelled"));
        ctx.em.log(Task::Download, ctx.t("dlw.cancelled_log"));
        ctx.em.send(UiMsg::Finished(Task::Download, Outcome::Cancelled));
    } else if code == 0 && skipped_existing {
        ctx.em.progress(Task::Download, -1.0);
        ctx.em.status(Task::Download, ctx.t("dlw.nothing_status"));
        ctx.em.log(Task::Download, ctx.t("dlw.nothing_log"));
        ctx.em.log(Task::Download, ctx.t("dlw.nothing_tip"));
        ctx.em.toast(ctx.t("dlw.nothing_status"), true);
        ctx.em.send(UiMsg::Finished(Task::Download, Outcome::Skipped));
    } else if code == 0 {
        ctx.em.progress(Task::Download, 1.0);
        ctx.em.status(Task::Download, ctx.t("dlw.completed"));
        ctx.em.log(Task::Download, ctx.t("dlw.success_log"));
        ctx.em.toast(ctx.t("dlw.success_toast"), false);
        ctx.em
            .send(UiMsg::Finished(Task::Download, Outcome::Success(produced)));
    } else {
        ctx.em.status(Task::Download, ctx.t("dlw.failed"));
        ctx.em.log(
            Task::Download,
            ctx.tf("dlw.failed_log", &[&code.to_string()]),
        );
        if joined.contains("empty media response") {
            ctx.em.log(
                Task::Download,
                ctx.tf("dlw.tip_instagram", &[YTDLP_INSTAGRAM_FIX]),
            );
        } else if joined.contains("requested format is not available")
            || joined.contains("please sign in")
            || joined.contains("login required")
        {
            ctx.em.log(Task::Download, ctx.t("dlw.tip_formats"));
        } else if failure_kind(&joined) == Some(Failure::Rejected) {
            ctx.em.log(Task::Download, ctx.t("dlw.tip_403"));
        } else if ctx.js_runtime.is_empty() {
            ctx.em.log(Task::Download, ctx.t("dlw.tip_nojs"));
        }
        ctx.em.toast(ctx.t("dlw.failed"), true);
        ctx.em.send(UiMsg::Finished(Task::Download, Outcome::Failed));
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_uploader_named_hosts_get_an_anonymous_name() {
        assert!(anon_outtmpl("https://www.youtube.com/watch?v=abc", ".").is_none());
        assert!(anon_outtmpl("https://www.instagram.com/reel/abc/", ".").is_some());
        // Host matching must not care about the case of the typed URL.
        assert!(anon_outtmpl("https://www.INSTAGRAM.com/reel/abc/", ".").is_some());
    }

    /// The joined command line, for substring assertions.
    fn cmd_line(quality: &str, opts: &DownloadOpts, force_overwrite: bool, retry: Retry) -> String {
        let bin = Binaries::new();
        build_download_cmd(
            &bin,
            "https://www.youtube.com/watch?v=abc",
            "out",
            quality,
            &None,
            opts,
            Attempt { outtmpl: None, force_overwrite, retry },
        )
        .join(" ")
    }

    #[test]
    fn wav_never_asks_for_a_thumbnail() {
        let opts = DownloadOpts { embed: true, ..Default::default() };
        assert!(!cmd_line("audio_wav", &opts, false, Retry::default()).contains("--embed-thumbnail"));
        // The formats that can hold one still get it.
        for q in ["audio", "audio_opus", "1080"] {
            let line = cmd_line(q, &opts, false, Retry::default());
            assert!(line.contains("--embed-thumbnail"), "{q}: {line}");
        }
    }

    #[test]
    fn the_default_client_rotation_comes_before_web_safari() {
        let line = cmd_line("1080", &DownloadOpts::default(), false, Retry::default());
        assert!(line.contains("player_client=default,web_safari"), "{line}");
    }

    #[test]
    fn a_rejection_drops_the_web_clients_right_away() {
        let second = next_retry(FIRST_ATTEMPT, Failure::Rejected);
        let line = cmd_line("1080", &DownloadOpts::default(), true, second);
        assert!(!line.contains("web_safari"), "{line}");
        // --force-overwrites turns resuming off, so it has to be turned back on
        // after it - the later option is the one yt-dlp honours.
        let force = line.find("--force-overwrites").unwrap();
        let cont = line.find("--continue").unwrap();
        assert!(force < cont, "{line}");
    }

    #[test]
    fn a_network_hiccup_keeps_the_formats_it_had() {
        let second = next_retry(FIRST_ATTEMPT, Failure::Flaky);
        assert!(cmd_line("1080", &DownloadOpts::default(), false, second).contains("web_safari"));
        // Once dropped, the web clients stay dropped for the rest of the run.
        let third = next_retry(next_retry(FIRST_ATTEMPT, Failure::Rejected), Failure::Flaky);
        assert!(!cmd_line("1080", &DownloadOpts::default(), false, third).contains("web_safari"));
    }

    #[test]
    fn only_the_first_attempt_starts_from_scratch_and_runs_parallel() {
        let opts = DownloadOpts::default();
        let first = cmd_line("1080", &opts, true, FIRST_ATTEMPT);
        assert!(!first.contains("--continue"), "{first}");
        assert!(first.contains("--concurrent-fragments 8"), "{first}");
        for kind in [Failure::Rejected, Failure::Flaky] {
            let line = cmd_line("1080", &opts, true, next_retry(FIRST_ATTEMPT, kind));
            assert!(line.contains("--continue"), "{kind:?}: {line}");
            assert!(line.contains("--concurrent-fragments 1"), "{kind:?}: {line}");
        }
    }

    #[test]
    fn a_mid_transfer_rejection_is_worth_another_run() {
        assert_eq!(
            failure_kind("error: unable to download video data: http error 403: forbidden"),
            Some(Failure::Rejected)
        );
        assert_eq!(
            failure_kind("error: unable to download data: http error 503"),
            Some(Failure::Flaky)
        );
        // A missing format or a login wall will not go away on a retry.
        assert_eq!(failure_kind("error: requested format is not available"), None);
        assert_eq!(failure_kind("error: sign in to confirm your age"), None);
    }

    #[test]
    fn the_file_yt_dlp_wrote_is_read_off_its_own_log_lines() {
        assert_eq!(
            output_path_from_line("[download] Destination: C:/v/clip.f137.mp4"),
            Some("C:/v/clip.f137.mp4".to_string())
        );
        assert_eq!(
            output_path_from_line("[Merger] Merging formats into \"C:/v/clip.mp4\""),
            Some("C:/v/clip.mp4".to_string())
        );
        assert_eq!(
            output_path_from_line("[MoveFiles] Moving file \"tmp/a.mp4\" to \"out/a.mp4\""),
            Some("out/a.mp4".to_string())
        );
        assert_eq!(
            output_path_from_line("[download] out/a.mp4 has already been downloaded"),
            Some("out/a.mp4".to_string())
        );
        // Progress and ordinary chatter name no file.
        assert_eq!(output_path_from_line("[youtube] Extracting URL: https://x"), None);
        assert_eq!(output_path_from_line("download:  12.0%|1.2MiB/s|00:10"), None);
    }

    #[test]
    fn a_playlist_run_asks_for_the_whole_list_in_its_own_folder() {
        let opts = DownloadOpts {
            playlist: true,
            playlist_items: "1-5".into(),
            ..Default::default()
        };
        let line = cmd_line("1080", &opts, false, Retry::default());
        assert!(line.contains("--yes-playlist"), "{line}");
        assert!(!line.contains("--no-playlist"), "{line}");
        assert!(line.contains("--playlist-items 1-5"), "{line}");
        assert!(line.contains("%(playlist_index)s"), "{line}");
        // A single video keeps the flat name it always had.
        let single = cmd_line("1080", &DownloadOpts::default(), false, Retry::default());
        assert!(single.contains("--no-playlist"), "{single}");
        assert!(!single.contains("%(playlist_index)s"), "{single}");
        assert!(!single.contains("--playlist-items"), "{single}");
    }

    #[test]
    fn a_speed_limit_is_only_passed_when_one_was_typed() {
        let opts = DownloadOpts { rate_limit: "2M".into(), ..Default::default() };
        assert!(cmd_line("1080", &opts, false, Retry::default()).contains("--limit-rate 2M"));
        let line = cmd_line("1080", &DownloadOpts::default(), false, Retry::default());
        assert!(!line.contains("--limit-rate"), "{line}");
    }

    #[test]
    fn the_anonymous_template_carries_no_title_field() {
        let (tmpl, stem) = anon_outtmpl("https://instagram.com/p/abc/", "out 50%").unwrap();
        assert!(tmpl.starts_with("out 50%%"), "directory not escaped: {tmpl}");
        assert!(tmpl.ends_with(&format!("{stem}.%(ext)s")), "got {tmpl}");
        assert!(!tmpl.contains("%(title)"), "got {tmpl}");
        let n: u64 = stem.strip_prefix("video-").unwrap().parse().unwrap();
        assert!((1..=9999).contains(&n), "got {stem}");
    }
}
