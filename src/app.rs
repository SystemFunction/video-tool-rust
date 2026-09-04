//! The egui desktop UI (ported from VideoToolApp).

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::binaries::Binaries;
use crate::config::Config;
use crate::consts::*;
use crate::convert::{self, ConvertParams, CvCtx};
use crate::download::{self, DlCtx};
use crate::emit::Emitter;
use crate::history;
use crate::i18n::{self, Lang};
use crate::types::{
    BinaryStatus, CheckKind, ConflictDecision, ConflictReq, DownloadOpts, MediaInfo, Outcome,
    QueueState, Task, UiMsg,
};
use crate::update::{self, Release};
use crate::util;

const MAX_LOG: usize = 1200;

/// Tab indices, so the nav, the keyboard shortcuts and the router agree.
const TAB_DOWNLOAD: usize = 0;
const TAB_CONVERT: usize = 1;
const TAB_HISTORY: usize = 2;
const TAB_SETUP: usize = 3;
const TAB_INFO: usize = 4;

/// One queued download, waiting for its turn or already past it.
struct QueueItem {
    url: String,
    state: QueueState,
}

/// Config key holding the last binary probe, see `probe_cache_load`.
const PROBE_CACHE_KEY: &str = "binary_probe_cache";

struct Toast {
    msg: String,
    error: bool,
    until: Instant,
}

pub struct App {
    ctx: egui::Context,
    bin: Arc<Binaries>,
    config: Config,
    tx: Sender<UiMsg>,
    rx: Receiver<UiMsg>,
    status: BinaryStatus,
    /// True while the background probe thread is running.
    probing: bool,
    tab: usize,
    lang: Lang,
    toasts: Vec<Toast>,
    setup_log: String,
    setup_busy: bool,
    channel: String,
    /// "dark" or "light"; anything else is read as dark.
    theme: String,
    /// Last title pushed to the window manager, so an unchanged one is not
    /// resent on every frame while a job runs.
    title: String,

    // download state
    dl_url: String,
    dl_output: String,
    dl_quality: String,
    dl_cookies: String,
    dl_cookiefile: String,
    dl_conflict: String,
    dl_impersonate: bool,
    dl_sponsorblock: bool,
    dl_embed: bool,
    dl_subs: bool,
    dl_subs_lang: String,
    dl_potoken: bool,
    dl_potoken_url: String,
    dl_playlist: bool,
    dl_playlist_items: String,
    dl_rate_limit: String,
    dl_advanced: bool,
    /// URLs lined up for download, oldest first.
    dl_queue: Vec<QueueItem>,
    /// True while the queue is being worked through; Stop clears it so the
    /// remaining entries stay put instead of starting on their own.
    dl_queue_running: bool,
    /// Index of the entry the running worker belongs to.
    dl_current: Option<usize>,
    /// URL and preset of that run, kept for the history entry it becomes.
    dl_current_url: String,
    dl_current_quality: String,
    /// Outcome the worker reported; `Failed` until it says otherwise, so a
    /// worker that dies without a word is not counted as a success.
    dl_last_outcome: Outcome,
    dl_log: Vec<String>,
    dl_status: String,
    dl_progress: Option<f32>,
    dl_busy: bool,
    dl_cancel: Arc<AtomicBool>,
    dl_child: Arc<Mutex<Option<Child>>>,

    // convert state
    cv_input: String,
    cv_output: String,
    cv_category: String,
    cv_codec: String,
    cv_hw: String,
    cv_bitrate_mode: String,
    cv_crf: f32,
    cv_custom_br: f32,
    cv_preserve_color: bool,
    /// Typed trim bounds, kept as text so a half-finished entry is not read
    /// as a cut. They only become numbers when the run starts.
    cv_trim_start: String,
    cv_trim_end: String,
    /// What ffprobe said about the current input, once it answered.
    cv_info: Option<MediaInfo>,
    cv_info_probing: bool,
    /// Input path the probe was started for - a late answer for a file the
    /// user has since replaced is dropped rather than shown against it.
    cv_info_for: String,
    /// Output of the last finished conversion, for "open output folder".
    cv_last_output: String,
    cv_log: Vec<String>,
    cv_status: String,
    cv_progress: Option<f32>,
    cv_busy: bool,
    cv_cancel: Arc<AtomicBool>,
    cv_child: Arc<Mutex<Option<Child>>>,

    // conflict modal
    pending_conflict: Option<ConflictReq>,
    conflict_name: String,
    conflict_error: Option<String>,

    // history
    history: Vec<history::Entry>,
    hist_filter: String,

    // updates
    update_auto: bool,
    update_ui: UpdateUi,
    update_checking: bool,
    /// Progress/result line for the Setup tab and the modal.
    update_status: String,
}

/// What the update modal is currently showing. An install has to stay on
/// screen from the click through to the restart prompt - closing the dialog
/// the moment work starts leaves the user with no sign anything happened.
enum UpdateUi {
    Idle,
    Offer(Release),
    Installing(Release),
    /// Swap succeeded; carries the version now on disk.
    Installed(String),
    /// Install failed; the message lives in `update_status`.
    Failed(Release),
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();

        let bin = Arc::new(Binaries::new());
        let config = Config::load(&bin.app_dir);
        let theme = config.get_str("theme", "dark");
        apply_style(&ctx, &theme);
        let (tx, rx) = std::sync::mpsc::channel::<UiMsg>();

        let downloads = dirs::download_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        // English is the default until the user picks something else.
        let lang = Lang::from_code(&config.get_str("language", Lang::En.code()));

        let mut app = App {
            ctx: ctx.clone(),
            bin: bin.clone(),
            tab: 0,
            lang,
            toasts: Vec::new(),
            setup_log: String::new(),
            setup_busy: false,
            channel: config.get_str("ytdlp_channel", "stable"),
            theme: theme.clone(),
            title: String::new(),

            dl_url: String::new(),
            dl_output: config.get_str("last_output_folder", &downloads),
            dl_quality: config.get_str("last_quality", "best"),
            dl_cookies: config.get_str("last_cookies", "none"),
            dl_cookiefile: config.get_str("cookies_file", ""),
            dl_conflict: config.get_str("conflict_mode", "ask"),
            dl_impersonate: config.get_bool("impersonate", false),
            dl_sponsorblock: config.get_bool("sponsorblock", false),
            dl_embed: config.get_bool("embed", true),
            dl_subs: config.get_bool("subs", false),
            dl_subs_lang: config.get_str("subs_lang", "en,de"),
            dl_potoken: config.get_bool("potoken", false),
            dl_potoken_url: config.get_str("potoken_url", ""),
            dl_playlist: config.get_bool("playlist", false),
            dl_playlist_items: config.get_str("playlist_items", ""),
            dl_rate_limit: config.get_str("rate_limit", ""),
            dl_advanced: false,
            dl_queue: Vec::new(),
            dl_queue_running: false,
            dl_current: None,
            dl_current_url: String::new(),
            dl_current_quality: String::new(),
            dl_last_outcome: Outcome::Failed,
            dl_log: vec![
                format!("  {APP_NAME} v{VERSION}"),
                format!("  {}", "-".repeat(36)),
                String::new(),
                i18n::t(lang, "log.ready_downloads").to_string(),
            ],
            dl_status: i18n::t(lang, "status.ready").to_string(),
            dl_progress: None,
            dl_busy: false,
            dl_cancel: Arc::new(AtomicBool::new(false)),
            dl_child: Arc::new(Mutex::new(None)),

            cv_input: String::new(),
            cv_output: String::new(),
            cv_category: "editing".into(),
            cv_codec: codec_options("editing")[0].0.into(),
            cv_hw: "auto".into(),
            cv_bitrate_mode: "crf".into(),
            cv_crf: 20.0,
            cv_custom_br: 20.0,
            cv_preserve_color: true,
            cv_trim_start: String::new(),
            cv_trim_end: String::new(),
            cv_info: None,
            cv_info_probing: false,
            cv_info_for: String::new(),
            cv_last_output: String::new(),
            cv_log: vec![i18n::t(lang, "log.ready_conversion").to_string()],
            cv_status: i18n::t(lang, "status.ready").to_string(),
            cv_progress: None,
            cv_busy: false,
            cv_cancel: Arc::new(AtomicBool::new(false)),
            cv_child: Arc::new(Mutex::new(None)),

            pending_conflict: None,
            conflict_name: String::new(),
            conflict_error: None,

            history: history::load(&config),
            hist_filter: "all".into(),

            update_auto: config.get_bool("update_check", true),
            update_ui: UpdateUi::Idle,
            update_checking: false,
            update_status: String::new(),

            status: BinaryStatus::default(),
            config,
            tx,
            rx,
            probing: false,
        };

        // Prefill the URL: a clipboard URL wins, otherwise the last used one.
        app.dl_url = clipboard_url().unwrap_or_else(|| app.config.get_str("last_url", ""));

        // Drop the binary a previous update moved aside.
        update::cleanup_backup();

        // Asking yt-dlp and FFmpeg for their versions costs seconds - both
        // unpack themselves before they answer - and the answer only changes
        // when the binaries do. A start that recognises the files it probed
        // last time therefore skips the probe entirely and is ready at once.
        let cached = probe_cache_load(&app.config, &app.bin);
        match cached {
            Some(status) => app.apply_status(status),
            None => app.start_probe(),
        }
        if app.update_auto {
            app.start_update_check(CheckKind::Automatic);
        }
        app
    }

    // ---------------- i18n shorthands ----------------

    fn t(&self, key: &'static str) -> &'static str {
        i18n::t(self.lang, key)
    }

    fn tf(&self, key: &'static str, args: &[&str]) -> String {
        i18n::tf(self.lang, key, args)
    }

    fn set_lang(&mut self, lang: Lang) {
        if self.lang == lang {
            return;
        }
        self.lang = lang;
        self.config.set_str("language", lang.code());
        // Statuses are plain strings, so an idle one would otherwise keep the
        // previous language until the next run.
        if !self.dl_busy {
            self.dl_status = self.t("status.ready").to_string();
        }
        if !self.cv_busy {
            self.cv_status = self.t("status.ready").to_string();
        }
    }

    fn emitter(&self) -> Emitter {
        Emitter::new(self.tx.clone(), self.ctx.clone())
    }

    fn start_probe(&mut self) {
        if self.probing {
            return;
        }
        self.probing = true;
        let bin = self.bin.clone();
        let em = self.emitter();
        thread::spawn(move || {
            em.send(UiMsg::Binary(bin.probe_status()));
        });
    }

    /// Adopts a probe result, from the cache or from a fresh run.
    fn apply_status(&mut self, status: BinaryStatus) {
        self.probing = false;
        if !status.impersonate_ok {
            self.dl_impersonate = false;
        }
        self.status = status;
    }

    fn start_update_check(&mut self, kind: CheckKind) {
        if self.update_checking || matches!(self.update_ui, UpdateUi::Installing(_)) {
            return;
        }
        self.update_checking = true;
        if kind == CheckKind::Manual {
            self.update_status = self.t("update.checking").to_string();
        }
        let em = self.emitter();
        thread::spawn(move || {
            em.send(UiMsg::UpdateCheck(update::check_latest(), kind));
        });
    }

    fn start_update_install(&mut self, release: Release) {
        let Some(asset) = release.asset.clone() else {
            return;
        };
        let version = release.version.clone();
        self.update_status = self.t("update.downloading").to_string();
        self.update_ui = UpdateUi::Installing(release);
        let em = self.emitter();
        let lang = self.lang;
        thread::spawn(move || {
            let prefix = i18n::t(lang, "update.downloading");
            let result = update::install(&asset, |w, t| {
                em.update_status(progress_text(lang, prefix, w, t));
            })
            .map(|_| version);
            em.send(UiMsg::UpdateInstalled(result));
        });
    }

    // ---------------- message draining ----------------

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                UiMsg::Log(task, line) => self.push_log(task, &line),
                UiMsg::Progress(task, v) => {
                    let p = if v < 0.0 { None } else { Some(v) };
                    match task {
                        Task::Download => self.dl_progress = p,
                        Task::Convert => self.cv_progress = p,
                    }
                }
                UiMsg::Status(task, s) => match task {
                    Task::Download => self.dl_status = s,
                    Task::Convert => self.cv_status = s,
                },
                UiMsg::Busy(task, b) => match task {
                    Task::Download => {
                        self.dl_busy = b;
                        if !b {
                            self.dl_progress = None;
                            // Sent after the worker's `Finished`, so the
                            // outcome it reported is already in hand.
                            self.download_ended();
                        }
                    }
                    Task::Convert => {
                        self.cv_busy = b;
                        if !b {
                            self.cv_progress = None;
                        }
                    }
                },
                UiMsg::Finished(Task::Download, outcome) => self.dl_last_outcome = outcome,
                UiMsg::Finished(Task::Convert, outcome) => self.convert_finished(outcome),
                UiMsg::MediaInfo(info) => {
                    let info = *info;
                    // A probe the user has already moved on from answers for
                    // the wrong file; only the current one is adopted.
                    let current = self.cv_input.trim();
                    if info.as_ref().map(|i| i.path.as_str()) == Some(current)
                        || self.cv_info_for == current
                    {
                        self.cv_info_probing = false;
                        self.cv_info = info;
                    }
                }
                UiMsg::Toast(msg, error) => self.toasts.push(Toast {
                    msg,
                    error,
                    until: Instant::now() + Duration::from_millis(3000),
                }),
                UiMsg::SetupLog(s) => self.setup_log = s,
                UiMsg::SetupBusy(b) => self.setup_busy = b,
                UiMsg::Binary(status) => {
                    probe_cache_store(&mut self.config, &self.bin, &status);
                    self.apply_status(status);
                }
                UiMsg::Conflict(req) => {
                    self.conflict_name = req.suggestion.clone();
                    self.conflict_error = None;
                    self.pending_conflict = Some(req);
                }
                UiMsg::UpdateStatus(s) => self.update_status = s,
                UiMsg::UpdateCheck(result, kind) => {
                    self.update_checking = false;
                    match result {
                        Ok(Some(release)) => {
                            // A version the user chose to skip stays silent on
                            // start, but a manual check always reports it.
                            let skipped =
                                self.config.get_str("update_skip_version", "") == release.version;
                            if kind == CheckKind::Manual || !skipped {
                                self.update_status.clear();
                                self.update_ui = UpdateUi::Offer(release);
                            }
                        }
                        Ok(None) => {
                            if kind == CheckKind::Manual {
                                self.update_status = self.tf("update.up_to_date", &[VERSION]);
                            }
                        }
                        Err(e) => {
                            // A failed automatic check stays quiet - being
                            // offline is not something to interrupt over.
                            if kind == CheckKind::Manual {
                                self.update_status = self.tf("update.check_failed", &[&e]);
                            }
                        }
                    }
                }
                UiMsg::UpdateInstalled(result) => match result {
                    Ok(version) => {
                        self.update_status = self.tf("update.installed", &[&version]);
                        self.update_ui = UpdateUi::Installed(version);
                    }
                    Err(e) => {
                        self.update_status = self.tf("update.failed", &[&e]);
                        let release = match std::mem::replace(&mut self.update_ui, UpdateUi::Idle) {
                            UpdateUi::Installing(r) => r,
                            other => {
                                self.update_ui = other;
                                continue;
                            }
                        };
                        self.update_ui = UpdateUi::Failed(release);
                    }
                },
            }
        }
    }

    fn push_log(&mut self, task: Task, message: &str) {
        let log = match task {
            Task::Download => &mut self.dl_log,
            Task::Convert => &mut self.cv_log,
        };
        for line in message.replace('\r', "").split('\n') {
            log.push(line.to_string());
        }
        if log.len() > MAX_LOG {
            let excess = log.len() - MAX_LOG;
            log.drain(0..excess);
        }
    }

    // ---------------- spawning workers ----------------

    /// The settings a run needs regardless of which URL is next.
    fn download_settings_ok(&mut self) -> bool {
        let out_dir = self.dl_output.trim().to_string();
        if out_dir.is_empty() {
            self.toast_key("toast.need_folder", true);
            return false;
        }
        if !self.status.ytdlp_ok {
            self.toast_key("toast.no_ytdlp", true);
            return false;
        }
        if self.dl_cookies == "cookiefile" {
            let file = self.dl_cookiefile.trim().to_string();
            if file.is_empty() || !Path::new(&file).is_file() {
                self.toast_key("toast.no_cookiefile", true);
                return false;
            }
        }
        // An unparseable limit would reach yt-dlp as a bare argument, so it is
        // caught here rather than turning into a confusing extractor error.
        if !self.dl_rate_limit.trim().is_empty() && !is_rate_limit(&self.dl_rate_limit) {
            self.toast_key("toast.bad_rate", true);
            return false;
        }
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            let msg = self.tf("toast.mkdir_failed", &[&e.to_string()]);
            self.toast(&msg, true);
            return false;
        }
        true
    }

    /// Moves the typed URL into the queue; `false` means it was not usable.
    fn enqueue_typed_url(&mut self) -> bool {
        let url = self.dl_url.trim().to_string();
        if url.is_empty() {
            self.toast_key("toast.need_url", true);
            return false;
        }
        if !is_http_url(&url) {
            self.toast_key("toast.bad_url", true);
            return false;
        }
        // Only waiting entries block a duplicate - queueing something again
        // after it finished is a normal way to retry it.
        if self
            .dl_queue
            .iter()
            .any(|i| i.url == url && i.state == QueueState::Pending)
        {
            self.toast_key("dl.queue_dupe", true);
            return false;
        }
        self.dl_queue.push(QueueItem {
            url,
            state: QueueState::Pending,
        });
        self.dl_url.clear();
        true
    }

    /// Start: queue whatever is typed, then work the list off top to bottom.
    fn start_downloads(&mut self) {
        if self.dl_busy {
            return;
        }
        if !self.dl_url.trim().is_empty() && !self.enqueue_typed_url() {
            return;
        }
        if !self
            .dl_queue
            .iter()
            .any(|i| i.state == QueueState::Pending)
        {
            return self.toast_key("toast.need_url", true);
        }
        if !self.download_settings_ok() {
            return;
        }
        self.dl_queue_running = true;
        self.dl_log.clear();
        self.advance_queue();
    }

    /// Starts the next waiting entry, or closes the run out.
    fn advance_queue(&mut self) {
        if self.dl_busy || !self.dl_queue_running {
            return;
        }
        let Some(index) = self
            .dl_queue
            .iter()
            .position(|i| i.state == QueueState::Pending)
        else {
            self.dl_queue_running = false;
            let count = |s: QueueState| self.dl_queue.iter().filter(|i| i.state == s).count();
            let (done, failed) = (count(QueueState::Done), count(QueueState::Failed));
            // A single download already reports itself; only a real queue
            // needs the summary on top of it.
            if self.dl_queue.len() > 1 {
                let msg = self.tf("dl.queue_done", &[&done.to_string(), &failed.to_string()]);
                self.toast(&msg, failed > 0);
            }
            return;
        };
        self.dl_queue[index].state = QueueState::Running;
        self.dl_current = Some(index);
        let url = self.dl_queue[index].url.clone();
        self.spawn_download(url);
    }

    /// Files the finished run away and lets the queue move on.
    fn download_ended(&mut self) {
        let Some(index) = self.dl_current.take() else {
            return;
        };
        let outcome = std::mem::replace(&mut self.dl_last_outcome, Outcome::Failed);
        let state = match &outcome {
            Outcome::Success(_) => QueueState::Done,
            Outcome::Skipped => QueueState::Skipped,
            // Stop is not a failure - the entry goes back to waiting so a
            // second Start picks up where this one left off.
            Outcome::Cancelled => QueueState::Pending,
            Outcome::Failed => QueueState::Failed,
        };
        if let Some(item) = self.dl_queue.get_mut(index) {
            item.state = state;
        }
        if let Outcome::Success(path) = &outcome {
            let url = self.dl_current_url.clone();
            let detail = label_of(self.lang, &self.dl_current_quality, QUALITY_OPTIONS);
            self.remember(history::Kind::Download, path.as_deref(), &url, &detail);
        }
        if matches!(outcome, Outcome::Cancelled) {
            self.dl_queue_running = false;
            return;
        }
        self.advance_queue();
    }

    fn spawn_download(&mut self, url: String) {
        if self.dl_busy {
            return;
        }
        let out_dir = self.dl_output.trim().to_string();
        let cookiefile = if self.dl_cookies == "cookiefile" {
            self.dl_cookiefile.trim().to_string()
        } else {
            String::new()
        };

        self.config.set_str("last_output_folder", &out_dir);
        self.config.set_str("last_url", &url);
        self.config.flush(true);

        let cookies = if matches!(self.dl_cookies.as_str(), "none" | "cookiefile") {
            None
        } else {
            Some(self.dl_cookies.clone())
        };
        let opts = DownloadOpts {
            impersonate: self.dl_impersonate && self.status.impersonate_ok,
            impersonate_available: self.status.impersonate_ok,
            sponsorblock: self.dl_sponsorblock,
            embed: self.dl_embed,
            subs: self.dl_subs,
            subs_lang: {
                let s = self.dl_subs_lang.trim();
                if s.is_empty() {
                    "en,de".into()
                } else {
                    s.to_string()
                }
            },
            cookiefile,
            potoken: self.dl_potoken,
            potoken_url: self.dl_potoken_url.trim().to_string(),
            plugins_dir: self.bin.plugins_dir.to_string_lossy().to_string(),
            conflict: self.dl_conflict.clone(),
            playlist: self.dl_playlist,
            playlist_items: clean_playlist_items(&self.dl_playlist_items),
            rate_limit: self.dl_rate_limit.trim().to_string(),
        };
        let quality = self.dl_quality.clone();
        self.dl_current_url = url.clone();
        self.dl_current_quality = quality.clone();
        self.dl_last_outcome = Outcome::Failed;

        self.dl_status = self.t("status.downloading").to_string();
        self.dl_progress = Some(0.0);
        self.dl_busy = true;
        self.dl_cancel.store(false, Ordering::SeqCst);
        let started = self.t("dlw.started").to_string();
        self.push_log(Task::Download, &format!("{started}\n{url}\n"));

        let dctx = DlCtx {
            bin: self.bin.clone(),
            em: self.emitter(),
            cancel: self.dl_cancel.clone(),
            child: self.dl_child.clone(),
            js_runtime: self.status.js_runtime.clone(),
            ytdlp_version: self.status.ytdlp_version.clone(),
            lang: self.lang,
        };
        thread::spawn(move || {
            download::run_download(dctx, url, out_dir, quality, cookies, opts);
        });
    }

    fn spawn_convert(&mut self) {
        if self.cv_busy {
            return;
        }
        let inp = self.cv_input.trim().to_string();
        let mut out = self.cv_output.trim().to_string();
        if inp.is_empty() || out.is_empty() {
            return self.toast_key("toast.need_io", true);
        }
        if !Path::new(&inp).exists() {
            return self.toast_key("toast.no_input", true);
        }
        if let Some(want) = util::convert_audio_ext(&self.cv_codec) {
            let cur = Path::new(&out)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if cur != want {
                out = Path::new(&out)
                    .with_extension(want)
                    .to_string_lossy()
                    .to_string();
                self.cv_output = out.clone();
            }
        }
        // ffmpeg opens the output before it finishes reading the input, so an
        // in-place conversion would truncate the source. `canonicalize` only
        // resolves paths that exist, hence the lexical fallback.
        let same = match (std::fs::canonicalize(&inp), std::fs::canonicalize(&out)) {
            (Ok(a), Ok(b)) => a == b,
            _ => Path::new(&inp) == Path::new(&out),
        };
        if same {
            return self.toast_key("toast.same_io", true);
        }
        if !self.status.ffmpeg_ok {
            return self.toast_key("toast.no_ffmpeg", true);
        }
        if let Some(parent) = Path::new(&out).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let msg = self.tf("toast.mkdir_out_failed", &[&e.to_string()]);
                return self.toast(&msg, true);
            }
        }

        let Some((trim_start, trim_end)) = self.trim_bounds() else {
            return self.toast_key("cv.trim_bad", true);
        };

        let params = ConvertParams {
            codec_key: self.cv_codec.clone(),
            hw_setting: self.cv_hw.clone(),
            crf: self.cv_crf as i32,
            use_custom: self.cv_bitrate_mode == "custom",
            custom_br: self.cv_custom_br as i32,
            preserve_color: self.cv_preserve_color,
            trim_start,
            trim_end,
        };

        self.cv_last_output.clear();
        self.cv_log.clear();
        self.cv_status = self.t("status.converting").to_string();
        self.cv_progress = Some(0.0);
        self.cv_busy = true;
        self.cv_cancel.store(false, Ordering::SeqCst);
        let name = convert::file_name(Path::new(&inp));
        let started = self.t("cvw.started").to_string();
        self.push_log(Task::Convert, &format!("{started}\n{name}\n"));

        let cctx = CvCtx {
            bin: self.bin.clone(),
            em: self.emitter(),
            cancel: self.cv_cancel.clone(),
            child: self.cv_child.clone(),
            lang: self.lang,
        };
        thread::spawn(move || {
            convert::run_conversion(cctx, inp, out, params);
        });
    }

    fn stop_download(&mut self) {
        // The whole list stops, not just the entry in flight - "Stop" on a
        // queue that immediately started the next URL would be no stop at all.
        self.dl_queue_running = false;
        self.dl_cancel.store(true, Ordering::SeqCst);
        if let Some(c) = util::lock(&self.dl_child).as_mut() {
            let _ = c.kill();
        }
        self.dl_status = self.t("status.cancelled").to_string();
    }

    /// The typed trim bounds as seconds, or `None` when they make no sense.
    ///
    /// An empty field is "no bound"; a field with something unreadable in it
    /// is an error, so a typo cannot quietly turn into a cut at second zero.
    fn trim_bounds(&self) -> Option<(Option<f64>, Option<f64>)> {
        let read = |text: &str| -> Option<Option<f64>> {
            if text.trim().is_empty() {
                Some(None)
            } else {
                util::parse_time_input(text).map(Some)
            }
        };
        let start = read(&self.cv_trim_start)?;
        let end = read(&self.cv_trim_end)?;
        match (start, end) {
            (Some(a), Some(b)) if b <= a => None,
            (None, Some(b)) if b <= 0.0 => None,
            _ => Some((start, end)),
        }
    }

    /// Reads the Convert tab's input file in the background.
    fn start_media_probe(&mut self) {
        let file = self.cv_input.trim().to_string();
        self.cv_info = None;
        self.cv_info_for = file.clone();
        if file.is_empty() || !Path::new(&file).is_file() || !self.status.ffmpeg_ok {
            self.cv_info_probing = false;
            return;
        }
        self.cv_info_probing = true;
        let bin = self.bin.clone();
        let em = self.emitter();
        thread::spawn(move || {
            let info = convert::probe_media_info(&bin, &file);
            em.send(UiMsg::MediaInfo(Box::new(info)));
        });
    }

    /// Adopts a finished conversion: its output and its history entry.
    fn convert_finished(&mut self, outcome: Outcome) {
        let Outcome::Success(Some(path)) = outcome else {
            return;
        };
        self.cv_last_output = path.to_string_lossy().to_string();
        let detail = label_of(self.lang, &self.cv_codec, codec_options(&self.cv_category));
        self.remember(history::Kind::Convert, Some(path.as_path()), "", &detail);
    }

    /// Writes one finished job into the history and persists the list.
    fn remember(
        &mut self,
        kind: history::Kind,
        path: Option<&Path>,
        url: &str,
        detail: &str,
    ) {
        let name = path
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| url.to_string());
        if name.is_empty() {
            return;
        }
        history::push(
            &mut self.history,
            history::Entry {
                kind,
                name,
                path: path.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                url: url.to_string(),
                detail: detail.to_string(),
                when: history::now_secs(),
            },
        );
        history::store(&mut self.config, &self.history);
    }

    fn log_of(&self, task: Task) -> &[String] {
        match task {
            Task::Download => &self.dl_log,
            Task::Convert => &self.cv_log,
        }
    }

    fn copy_log(&mut self, task: Task) {
        let text = self.log_of(task).join("\n");
        if text.trim().is_empty() {
            return self.toast_key("common.log_empty", true);
        }
        if copy_to_clipboard(&text) {
            self.toast_key("common.log_copied", false);
        }
    }

    fn save_log(&mut self, task: Task) {
        // CRLF so the file opens readably in Notepad, which is where a log
        // handed to someone else usually lands first.
        let text = self.log_of(task).join("\r\n");
        if text.trim().is_empty() {
            return self.toast_key("common.log_empty", true);
        }
        let stem = match task {
            Task::Download => "download",
            Task::Convert => "convert",
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Text", &["txt"])
            .set_file_name(format!("video-tool-{stem}.txt"))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, text) {
            Ok(_) => {
                let msg = self.tf("common.log_saved", &[&convert::file_name(&path)]);
                self.toast(&msg, false);
            }
            Err(e) => {
                let msg = self.tf("common.log_save_failed", &[&e.to_string()]);
                self.toast(&msg, true);
            }
        }
    }

    fn set_theme(&mut self, theme: &str) {
        if self.theme == theme {
            return;
        }
        self.theme = theme.to_string();
        self.config.set_str("theme", theme);
        apply_style(&self.ctx, theme);
    }

    fn dark(&self) -> bool {
        self.theme != "light"
    }

    fn stop_convert(&mut self) {
        self.cv_cancel.store(true, Ordering::SeqCst);
        if let Some(c) = util::lock(&self.cv_child).as_mut() {
            let _ = c.kill();
        }
        self.cv_status = self.t("status.cancelled").to_string();
    }

    // ---------------- setup workers ----------------

    fn install_binaries(&mut self) {
        self.setup_busy = true;
        self.setup_log = self.t("setup.installing").to_string();
        let bin = self.bin.clone();
        let em = self.emitter();
        let lang = self.lang;
        thread::spawn(move || {
            em.setup_log(i18n::t(lang, "setup.dl_ytdlp"));
            let r = bin.install_ytdlp(|w, t| {
                em.setup_log(progress_text(lang, i18n::t(lang, "setup.dl_ytdlp"), w, t));
            });
            if let Err(e) = r {
                em.setup_log(i18n::tf(lang, "common.error", &[&e]));
                em.toast(i18n::tf(lang, "setup.install_error", &[&e]), true);
                em.setup_busy(false);
                return;
            }
            em.setup_log(i18n::t(lang, "setup.dl_ffmpeg"));
            let r = bin.install_ffmpeg(|name, w, t| {
                let prefix = i18n::tf(lang, "setup.dl_named", &[name]);
                em.setup_log(progress_text(lang, &prefix, w, t));
            });
            if let Err(e) = r {
                em.setup_log(i18n::tf(lang, "common.error", &[&e]));
                em.toast(i18n::tf(lang, "setup.install_error", &[&e]), true);
                em.setup_busy(false);
                return;
            }
            em.setup_log(i18n::t(lang, "setup.install_done"));
            em.send(UiMsg::Binary(bin.probe_status()));
            em.toast(i18n::t(lang, "setup.install_ok_toast"), false);
            em.setup_busy(false);
        });
    }

    fn update_ytdlp(&mut self) {
        let channel = self.channel.clone();
        self.setup_busy = true;
        self.setup_log = if channel == "stable" {
            self.t("setup.updating_ytdlp_status").to_string()
        } else {
            self.tf("setup.switch_channel", &[&channel])
        };
        let bin = self.bin.clone();
        let em = self.emitter();
        let lang = self.lang;
        thread::spawn(move || {
            let r = if channel == "stable" {
                bin.install_ytdlp(|w, t| {
                    em.setup_log(progress_text(
                        lang,
                        i18n::t(lang, "setup.updating_ytdlp"),
                        w,
                        t,
                    ))
                })
            } else {
                em.setup_log(i18n::tf(lang, "setup.switch_channel", &[&channel]));
                bin.update_channel(&channel).map(|out| {
                    if !out.is_empty() {
                        em.log(Task::Download, out);
                    }
                })
            };
            match r {
                Ok(_) => {
                    em.setup_log(i18n::t(lang, "setup.ytdlp_updated"));
                    em.send(UiMsg::Binary(bin.probe_status()));
                    em.toast(i18n::t(lang, "setup.ytdlp_updated_toast"), false);
                }
                Err(e) => {
                    em.setup_log(i18n::tf(lang, "common.error", &[&e]));
                    em.toast(i18n::tf(lang, "setup.update_error", &[&e]), true);
                }
            }
            em.setup_busy(false);
        });
    }

    fn install_deno(&mut self) {
        self.setup_busy = true;
        self.setup_log = self.t("setup.dl_deno_status").to_string();
        let bin = self.bin.clone();
        let em = self.emitter();
        let lang = self.lang;
        thread::spawn(move || {
            let r = bin.install_deno(|w, t| {
                em.setup_log(progress_text(lang, i18n::t(lang, "setup.dl_deno"), w, t))
            });
            match r {
                Ok(_) => {
                    em.setup_log(i18n::t(lang, "setup.deno_installed"));
                    em.send(UiMsg::Binary(bin.probe_status()));
                    em.toast(i18n::t(lang, "setup.deno_installed_toast"), false);
                }
                Err(e) => {
                    em.setup_log(i18n::tf(lang, "common.error", &[&e]));
                    em.toast(i18n::tf(lang, "setup.deno_error", &[&e]), true);
                }
            }
            em.setup_busy(false);
        });
    }

    fn toast(&mut self, msg: &str, error: bool) {
        self.toasts.push(Toast {
            msg: msg.to_string(),
            error,
            until: Instant::now() + Duration::from_millis(3000),
        });
    }

    fn toast_key(&mut self, key: &'static str, error: bool) {
        let msg = self.t(key).to_string();
        self.toast(&msg, error);
    }

    // ---------------- UI panels ----------------

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(APP_NAME).color(Color32::from_rgb(77, 208, 225)));
            ui.label(RichText::new(format!("v{VERSION}")).color(Color32::from_rgb(178, 235, 242)));
            ui.add_space(16.0);
            let mut picked: Option<Lang> = None;
            let mut next_theme: Option<&str> = None;
            let lang = self.lang;
            with_available_right(ui, |ui| {
                let (yt_ok, yt) = (self.status.ytdlp_ok, &self.status.ytdlp_version);
                let (ff_ok, ff) = (self.status.ffmpeg_ok, &self.status.ffmpeg_version);
                chip(ui, "FFmpeg", ff_ok, if ff_ok { ff } else { "x" });
                chip(ui, "yt-dlp", yt_ok, if yt_ok { yt } else { "x" });
                ui.add_space(8.0);
                // The icon shows what a click switches to, not what is on.
                let dark = self.theme != "light";
                let icon = if dark { "☀" } else { "🌙" };
                if ui
                    .button(RichText::new(icon).size(15.0))
                    .on_hover_text(i18n::t(lang, "theme.toggle"))
                    .clicked()
                {
                    next_theme = Some(if dark { "light" } else { "dark" });
                }
                ui.add_space(4.0);
                // Sits with the other global state, reachable from every tab.
                egui::ComboBox::from_id_salt("lang")
                    .selected_text(format!("🌐 {}", lang.label()))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for option in i18n::LANGS {
                            if ui
                                .selectable_label(option == lang, option.label())
                                .clicked()
                            {
                                picked = Some(option);
                            }
                        }
                    })
                    .response
                    .on_hover_text(i18n::t(lang, "common.language"));
            });
            if let Some(l) = picked {
                self.set_lang(l);
            }
            if let Some(theme) = next_theme {
                self.set_theme(theme);
            }
        });
    }

    fn ui_footer(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let hw = match self.status.hw_backend.as_str() {
                "nvidia" => "NVIDIA NVENC",
                "amd" => "AMD AMF",
                "intel" => "Intel QSV",
                _ => "CPU",
            };
            let dim = Color32::from_rgb(150, 150, 150);
            ui.label(RichText::new(self.tf("footer.hw", &[hw])).size(11.0).color(dim));
            ui.separator();
            let imp = if self.status.impersonate_ok {
                self.t("common.on")
            } else {
                self.t("common.off")
            };
            ui.label(
                RichText::new(self.tf("footer.impersonate", &[imp]))
                    .size(11.0)
                    .color(dim),
            );
            ui.separator();
            let js = if self.status.deno_ok {
                self.status.deno_version.as_str()
            } else {
                "-"
            };
            ui.label(RichText::new(self.tf("footer.js", &[js])).size(11.0).color(dim));
            ui.separator();
            ui.label(
                RichText::new(format!("{} {}", self.bin.system, std::env::consts::ARCH))
                    .size(11.0)
                    .color(dim),
            );
            with_available_right(ui, |ui| {
                ui.label(
                    RichText::new(format!("{APP_NAME} v{VERSION}"))
                        .size(11.0)
                        .color(dim),
                );
            });
        });
    }

    fn ui_nav(&mut self, ui: &mut egui::Ui) {
        // Translated tab names are longer than the English ones - without this
        // "Konvertieren" and "Téléchargement" break across two lines.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let items = [
            ("⬇", "nav.download"),
            ("⚙", "nav.convert"),
            ("🕘", "nav.history"),
            ("🔧", "nav.setup"),
            ("ℹ", "nav.info"),
        ];
        for (i, (icon, key)) in items.iter().enumerate() {
            let selected = self.tab == i;
            // A running job is worth seeing from the other tabs too.
            let busy = (i == TAB_DOWNLOAD && self.dl_busy) || (i == TAB_CONVERT && self.cv_busy);
            let label = if busy {
                format!("{icon}  {} ●", self.t(key))
            } else {
                format!("{icon}  {}", self.t(key))
            };
            let text = RichText::new(label).size(15.0);
            let text = if busy { text.color(accent()) } else { text };
            if ui.selectable_label(selected, text).clicked() {
                self.tab = i;
            }
            ui.add_space(4.0);
        }
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);
        // The hint is a sentence, not a tab name - it may wrap.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.label(
            RichText::new(self.t("common.shortcuts"))
                .size(10.0)
                .color(Color32::GRAY),
        );
    }

    fn ui_download(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading(self.t("nav.download"));
        ui.label(
            RichText::new(self.t("dl.subtitle"))
                .color(Color32::GRAY)
                .size(12.0),
        );
        ui.add_space(8.0);

        let lang = self.lang;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new(self.t("dl.source_quality")).strong());
            ui.horizontal(|ui| {
                ui.label(self.t("dl.url"));
                let hint = self.t("dl.url_hint");
                ui.add(
                    egui::TextEdit::singleline(&mut self.dl_url)
                        .desired_width(f32::INFINITY)
                        .hint_text(hint),
                );
            });
            ui.horizontal(|ui| {
                if ui.button(self.t("dl.paste")).clicked() {
                    if let Some(t) = clipboard_text() {
                        self.dl_url = t;
                    }
                }
                if ui
                    .button(self.t("dl.add_queue"))
                    .on_hover_text(self.t("dl.add_queue_hint"))
                    .clicked()
                    && self.enqueue_typed_url()
                {
                    let waiting = self
                        .dl_queue
                        .iter()
                        .filter(|i| i.state == QueueState::Pending)
                        .count();
                    let msg = self.tf("dl.queue_added", &[&waiting.to_string()]);
                    self.toast(&msg, false);
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("dl_opts")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(self.t("dl.quality"));
                    if combo(ui, lang, "dl_quality", &mut self.dl_quality, QUALITY_OPTIONS) {
                        self.config.set_str("last_quality", &self.dl_quality);
                    }
                    ui.end_row();
                    ui.label(self.t("dl.cookies"));
                    if combo(ui, lang, "dl_cookies", &mut self.dl_cookies, COOKIES_OPTIONS) {
                        self.config.set_str("last_cookies", &self.dl_cookies);
                    }
                    ui.end_row();
                    ui.label(self.t("dl.conflict"));
                    if combo(ui, lang, "dl_conflict", &mut self.dl_conflict, CONFLICT_OPTIONS) {
                        self.config.set_str("conflict_mode", &self.dl_conflict);
                    }
                    ui.end_row();
                });

            if self.dl_cookies == "cookiefile" {
                ui.horizontal(|ui| {
                    ui.label(self.t("dl.cookiefile"));
                    // Button before the field: a TextEdit sized to INFINITY
                    // consumes the rest of the row and would push anything
                    // after it past the right edge.
                    if ui.button(self.t("common.choose")).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("cookies", &["txt"])
                            .pick_file()
                        {
                            self.dl_cookiefile = p.to_string_lossy().to_string();
                            self.config.set_str("cookies_file", &self.dl_cookiefile);
                        }
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.dl_cookiefile)
                            .desired_width(f32::INFINITY),
                    );
                });
            }

            ui.add_space(4.0);
            let arrow = if self.dl_advanced { "▼" } else { "▶" };
            if ui
                .selectable_label(false, format!("{arrow} {}", self.t("dl.advanced")))
                .clicked()
            {
                self.dl_advanced = !self.dl_advanced;
            }
            if self.dl_advanced {
                ui.indent("adv", |ui| {
                    // Labels are resolved up front: taking one straight from
                    // `self` inside a call that also borrows a field mutably
                    // would overlap the two borrows.
                    let (impersonate_ok, l_impersonate) =
                        (self.status.impersonate_ok, self.t("dl.impersonate"));
                    if ui
                        .add_enabled(
                            impersonate_ok,
                            egui::Checkbox::new(&mut self.dl_impersonate, l_impersonate),
                        )
                        .changed()
                    {
                        self.config.set_bool("impersonate", self.dl_impersonate);
                    }
                    let l_sponsorblock = self.t("dl.sponsorblock");
                    if ui
                        .checkbox(&mut self.dl_sponsorblock, l_sponsorblock)
                        .changed()
                    {
                        self.config.set_bool("sponsorblock", self.dl_sponsorblock);
                    }
                    let l_embed = self.t("dl.embed");
                    if ui.checkbox(&mut self.dl_embed, l_embed).changed() {
                        self.config.set_bool("embed", self.dl_embed);
                    }
                    ui.horizontal(|ui| {
                        let l_subs = self.t("dl.subs");
                        if ui.checkbox(&mut self.dl_subs, l_subs).changed() {
                            self.config.set_bool("subs", self.dl_subs);
                        }
                        ui.label(self.t("dl.subs_langs"));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.dl_subs_lang)
                                    .desired_width(120.0),
                            )
                            .changed()
                        {
                            self.config.set_str("subs_lang", &self.dl_subs_lang);
                        }
                    });
                    ui.horizontal(|ui| {
                        let l_playlist = self.t("dl.playlist");
                        let hint = self.t("dl.playlist_hint");
                        if ui
                            .checkbox(&mut self.dl_playlist, l_playlist)
                            .on_hover_text(hint)
                            .changed()
                        {
                            self.config.set_bool("playlist", self.dl_playlist);
                        }
                        ui.label(self.t("dl.playlist_items"));
                        let items_hint = self.t("dl.playlist_items_hint");
                        let playlist = self.dl_playlist;
                        if ui
                            .add_enabled(
                                playlist,
                                egui::TextEdit::singleline(&mut self.dl_playlist_items)
                                    .desired_width(110.0)
                                    .hint_text(items_hint),
                            )
                            .changed()
                        {
                            // Only a range ever reaches yt-dlp; anything else
                            // the field collects is dropped as it is typed.
                            self.dl_playlist_items = clean_playlist_items(&self.dl_playlist_items);
                            self.config
                                .set_str("playlist_items", &self.dl_playlist_items);
                        }
                        ui.add_space(8.0);
                        ui.label(self.t("dl.rate_limit"));
                        let rate_hint = self.t("dl.rate_limit_hint");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.dl_rate_limit)
                                    .desired_width(90.0)
                                    .hint_text(rate_hint),
                            )
                            .changed()
                        {
                            self.config.set_str("rate_limit", &self.dl_rate_limit);
                        }
                    });
                    ui.horizontal(|ui| {
                        let l_potoken = self.t("dl.potoken");
                        if ui.checkbox(&mut self.dl_potoken, l_potoken).changed() {
                            self.config.set_bool("potoken", self.dl_potoken);
                        }
                        ui.label(self.t("dl.provider_url"));
                        let hint = self.t("dl.provider_hint");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.dl_potoken_url)
                                    .desired_width(220.0)
                                    .hint_text(hint),
                            )
                            .changed()
                        {
                            self.config.set_str("potoken_url", &self.dl_potoken_url);
                        }
                    });
                });
            }
        });

        ui.add_space(6.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new(self.t("dl.save_location")).strong());
            ui.horizontal(|ui| {
                // Same ordering rule as above - these buttons used to be
                // declared after the INFINITY-width field and were pushed
                // out of the visible row entirely.
                if ui
                    .button(self.t("dl.choose_folder"))
                    .on_hover_text(self.t("dl.choose_folder_hint"))
                    .clicked()
                {
                    let mut dlg = rfd::FileDialog::new().set_title(self.t("dl.choose_folder_title"));
                    let current = Path::new(self.dl_output.trim());
                    if current.is_dir() {
                        dlg = dlg.set_directory(current);
                    }
                    if let Some(p) = dlg.pick_folder() {
                        self.dl_output = p.to_string_lossy().to_string();
                        self.config.set_str("last_output_folder", &self.dl_output);
                    }
                }
                if ui
                    .button(self.t("dl.open_folder"))
                    .on_hover_text(self.t("dl.open_folder_hint"))
                    .clicked()
                {
                    open_folder(&self.dl_output);
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.dl_output).desired_width(f32::INFINITY),
                );
            });
        });

        ui.add_space(6.0);
        self.ui_queue(ui);

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let waiting = self
                .dl_queue
                .iter()
                .filter(|i| i.state == QueueState::Pending)
                .count();
            // With something already lined up the button says how much work
            // pressing it starts, rather than just "Start".
            let extra = usize::from(!self.dl_url.trim().is_empty());
            let label = if waiting + extra > 1 {
                self.tf("dl.start_queue", &[&(waiting + extra).to_string()])
            } else {
                self.t("dl.start").to_string()
            };
            if ui
                .add_enabled(!self.dl_busy, egui::Button::new(label))
                .clicked()
            {
                self.start_downloads();
            }
            if ui
                .add_enabled(self.dl_busy, egui::Button::new(self.t("common.stop")))
                .clicked()
            {
                self.stop_download();
            }
            ui.label(&self.dl_status);
        });
        if let Some(p) = self.dl_progress {
            ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        }

        ui.add_space(6.0);
        self.ui_log_toolbar(ui, Task::Download);
        log_view(ui, "dl_log", &self.dl_log, 200.0, self.dark());
        let _ = ctx;
    }

    /// The queue list. Hidden entirely while nothing is lined up, so the tab
    /// looks exactly as it did for the one-URL-at-a-time case.
    fn ui_queue(&mut self, ui: &mut egui::Ui) {
        if self.dl_queue.is_empty() {
            return;
        }
        let lang = self.lang;
        let mut remove: Option<usize> = None;
        let mut clear_all = false;
        let mut clear_done = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(i18n::t(lang, "dl.queue_title")).strong());
                let running = self
                    .dl_queue
                    .iter()
                    .filter(|i| i.state != QueueState::Pending)
                    .count();
                ui.label(
                    RichText::new(i18n::tf(
                        lang,
                        "dl.queue_running",
                        &[&running.to_string(), &self.dl_queue.len().to_string()],
                    ))
                    .size(11.0)
                    .color(Color32::GRAY),
                );
                with_available_right(ui, |ui| {
                    if ui.button(i18n::t(lang, "dl.queue_clear")).clicked() {
                        clear_all = true;
                    }
                    if ui.button(i18n::t(lang, "dl.queue_clear_done")).clicked() {
                        clear_done = true;
                    }
                });
            });
            ui.add_space(2.0);
            egui::ScrollArea::vertical()
                .id_salt("dl_queue")
                .max_height(140.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, item) in self.dl_queue.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let (mark, color) = queue_marks(item.state);
                            ui.label(RichText::new(mark).color(color));
                            ui.label(
                                RichText::new(i18n::t(lang, queue_state_key(item.state)))
                                    .size(11.0)
                                    .color(color),
                            );
                            // Running entries are what the worker is holding;
                            // dropping one would renumber it mid-flight.
                            let removable = item.state != QueueState::Running;
                            if ui
                                .add_enabled(removable, egui::Button::new("✖").small())
                                .on_hover_text(i18n::t(lang, "common.remove"))
                                .clicked()
                            {
                                remove = Some(i);
                            }
                            ui.label(RichText::new(short_url(&item.url)).size(11.0))
                                .on_hover_text(&item.url);
                        });
                    }
                });
        });
        if let Some(i) = remove {
            self.dl_queue.remove(i);
            // The running entry keeps its identity even if the list shrank
            // above it.
            if let Some(current) = self.dl_current.as_mut() {
                if i < *current {
                    *current -= 1;
                }
            }
        }
        if clear_done {
            self.dl_queue
                .retain(|i| matches!(i.state, QueueState::Pending | QueueState::Running));
        }
        if clear_all {
            // Whatever is downloading right now stays - the list is a plan,
            // not a kill switch; Stop is.
            self.dl_queue.retain(|i| i.state == QueueState::Running);
            self.dl_queue_running = false;
        }
        if clear_done || clear_all {
            self.dl_current = self
                .dl_queue
                .iter()
                .position(|i| i.state == QueueState::Running);
        }
    }

    /// Header row above a live log: title plus copy / save / clear.
    fn ui_log_toolbar(&mut self, ui: &mut egui::Ui, task: Task) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.t("common.live_log")).strong());
            if ui.button(self.t("common.copy")).clicked() {
                self.copy_log(task);
            }
            if ui.button(self.t("common.save")).clicked() {
                self.save_log(task);
            }
            if ui.button(self.t("common.clear")).clicked() {
                match task {
                    Task::Download => self.dl_log.clear(),
                    Task::Convert => self.cv_log.clear(),
                }
            }
        });
    }

    fn ui_convert(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.heading(self.t("nav.convert"));
        ui.label(
            RichText::new(self.t("cv.subtitle"))
                .color(Color32::GRAY)
                .size(12.0),
        );
        ui.add_space(8.0);

        let lang = self.lang;
        let mut probe = false;
        ui.horizontal(|ui| {
            ui.label(self.t("cv.input"));
            // Button first so it stays visible - the TextEdit below fills the
            // rest of the row (desired_width INFINITY), which would otherwise
            // push a trailing button off the right edge.
            if ui.button(self.t("cv.browse")).clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter(
                        self.t("cv.filter_media"),
                        &[
                            "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts", "mp3",
                            "wav", "m4a", "aac", "opus", "flac",
                        ],
                    )
                    .add_filter(self.t("cv.filter_all"), &["*"])
                    .pick_file()
                {
                    self.cv_input = p.to_string_lossy().to_string();
                    let stem = p
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if let Some(parent) = p.parent() {
                        self.cv_output = parent
                            .join(format!("{stem}_converted.mp4"))
                            .to_string_lossy()
                            .to_string();
                    }
                    probe = true;
                }
            }
            // ffprobe starts a process, so a typed path is read once the field
            // is done rather than on every keystroke.
            if ui
                .add(egui::TextEdit::singleline(&mut self.cv_input).desired_width(f32::INFINITY))
                .lost_focus()
            {
                probe = true;
            }
        });
        if probe {
            self.start_media_probe();
        }
        self.ui_media_info(ui);
        ui.horizontal(|ui| {
            ui.label(self.t("cv.output"));
            if ui.button(self.t("cv.save_as")).clicked() {
                let mut dlg = rfd::FileDialog::new().add_filter(
                    self.t("cv.filter_media"),
                    &["mp4", "mkv", "mov", "webm", "mp3", "wav"],
                );
                let cur = std::path::Path::new(&self.cv_output);
                if let Some(parent) = cur.parent() {
                    if parent.is_dir() {
                        dlg = dlg.set_directory(parent);
                    }
                }
                if let Some(name) = cur.file_name().and_then(|n| n.to_str()) {
                    dlg = dlg.set_file_name(name);
                }
                if let Some(p) = dlg.save_file() {
                    self.cv_output = p.to_string_lossy().to_string();
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.cv_output).desired_width(f32::INFINITY));
        });

        ui.add_space(6.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("cv_opts")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(self.t("cv.category"));
                    if combo(ui, lang, "cv_cat", &mut self.cv_category, CATEGORY_OPTIONS) {
                        let opts = codec_options(&self.cv_category);
                        self.cv_codec = opts[0].0.into();
                    }
                    ui.end_row();
                    ui.label(self.t("cv.codec"));
                    combo(
                        ui,
                        lang,
                        "cv_codec",
                        &mut self.cv_codec,
                        codec_options(&self.cv_category),
                    );
                    ui.end_row();
                    ui.label(self.t("cv.hardware"));
                    combo(ui, lang, "cv_hw", &mut self.cv_hw, HW_OPTIONS);
                    ui.end_row();
                    ui.label(self.t("cv.bitrate_mode"));
                    combo(
                        ui,
                        lang,
                        "cv_brmode",
                        &mut self.cv_bitrate_mode,
                        BITRATE_MODE_OPTIONS,
                    );
                    ui.end_row();
                });

            let audio_only = util::convert_audio_ext(&self.cv_codec).is_some();
            if let Some(hint) = codec_hint(&self.cv_codec, audio_only) {
                ui.label(
                    RichText::new(self.t(hint))
                        .color(Color32::from_rgb(255, 183, 77))
                        .size(11.0),
                );
            }

            if !audio_only {
                if self.cv_bitrate_mode == "custom" {
                    let l_mbps = self.t("cv.mbps");
                    ui.add(
                        egui::Slider::new(&mut self.cv_custom_br, 2.0..=200.0)
                            .text(l_mbps)
                            .integer(),
                    );
                    ui.horizontal(|ui| {
                        for b in [8.0, 20.0, 50.0, 100.0] {
                            if ui.button(format!("{}M", b as i32)).clicked() {
                                self.cv_custom_br = b;
                            }
                        }
                    });
                } else {
                    let l_crf = self.t("cv.crf");
                    ui.add(
                        egui::Slider::new(&mut self.cv_crf, 15.0..=30.0)
                            .text(l_crf)
                            .integer(),
                    );
                }
            }
            let l_color = self.t("cv.preserve_color");
            ui.checkbox(&mut self.cv_preserve_color, l_color);

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.t("cv.trim")).strong());
                ui.label(self.t("cv.trim_start"));
                let hint = self.t("cv.trim_hint");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cv_trim_start)
                        .desired_width(80.0)
                        .hint_text("0:00"),
                );
                ui.label(self.t("cv.trim_end"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.cv_trim_end)
                        .desired_width(80.0)
                        .hint_text("1:30"),
                )
                .on_hover_text(hint);
            });
            // Says what the fields currently mean, including when they say
            // nothing usable yet.
            let (text, color) = match self.trim_bounds() {
                Some((None, None)) => (String::new(), Color32::GRAY),
                Some((start, end)) => {
                    let from = util::format_clock(start.unwrap_or(0.0));
                    let to = end
                        .map(util::format_clock)
                        .unwrap_or_else(|| self.t("cv.trim_end_of_file").to_string());
                    let span = match (start, end) {
                        (s, Some(e)) => util::format_clock(e - s.unwrap_or(0.0)),
                        _ => format!("{from} -> {to}"),
                    };
                    (self.tf("cv.trim_active", &[&span]), Color32::GRAY)
                }
                None => (
                    self.t("cv.trim_bad").to_string(),
                    Color32::from_rgb(239, 154, 154),
                ),
            };
            if !text.is_empty() {
                ui.label(RichText::new(text).size(11.0).color(color));
            }

            if let Some(estimate) = self.output_estimate() {
                ui.label(
                    RichText::new(self.tf("cv.estimate", &[&estimate]))
                        .size(11.0)
                        .color(Color32::GRAY),
                );
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.cv_busy, egui::Button::new(self.t("cv.start")))
                .clicked()
            {
                self.spawn_convert();
            }
            if ui
                .add_enabled(self.cv_busy, egui::Button::new(self.t("common.stop")))
                .clicked()
            {
                self.stop_convert();
            }
            // Only meaningful once something has actually been written.
            let done = !self.cv_last_output.is_empty();
            if ui
                .add_enabled(done, egui::Button::new(self.t("cv.open_output")))
                .clicked()
            {
                if let Some(parent) = Path::new(&self.cv_last_output).parent() {
                    open_folder(&parent.to_string_lossy());
                }
            }
            ui.label(&self.cv_status);
        });
        if let Some(p) = self.cv_progress {
            ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        }

        ui.add_space(6.0);
        self.ui_log_toolbar(ui, Task::Convert);
        log_view(ui, "cv_log", &self.cv_log, 240.0, self.dark());
    }

    /// What ffprobe found in the chosen input, once it has answered.
    fn ui_media_info(&mut self, ui: &mut egui::Ui) {
        if self.cv_input.trim().is_empty() {
            return;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(self.t("cv.info")).strong());
                if self.cv_info_probing {
                    ui.label(
                        RichText::new(self.t("cv.info_probing"))
                            .size(11.0)
                            .color(Color32::GRAY),
                    );
                    return;
                }
                let Some(info) = &self.cv_info else {
                    ui.label(
                        RichText::new(self.t("cv.info_none"))
                            .size(11.0)
                            .color(Color32::from_rgb(255, 183, 77)),
                    );
                    return;
                };
                let dim = Color32::GRAY;
                let mut fact = |label: &str, value: String| {
                    if value.is_empty() {
                        return;
                    }
                    ui.label(RichText::new(format!("{label}:")).size(11.0).color(dim));
                    ui.label(RichText::new(value).size(11.0));
                    ui.add_space(8.0);
                };
                fact(
                    self.t("cv.info_length"),
                    info.duration.map(util::format_clock).unwrap_or_default(),
                );
                let mut video = String::new();
                if info.width > 0 && info.height > 0 {
                    video = format!("{}x{}", info.width, info.height);
                }
                if let Some(fps) = info.fps {
                    video = format!("{video} @ {fps:.2} fps");
                }
                if !info.vcodec.is_empty() {
                    video = format!("{} ({})", video.trim(), info.vcodec);
                }
                fact(self.t("cv.info_video"), video.trim().to_string());
                fact(self.t("cv.info_audio"), info.acodec.clone());
                fact(self.t("cv.info_size"), util::format_bytes(info.size_bytes));
            });
        });
    }

    /// Rough size of what a custom-bitrate run will write.
    ///
    /// Only the custom-bitrate mode can be estimated at all - CRF decides its
    /// rate from the picture, which nothing here can predict.
    fn output_estimate(&self) -> Option<String> {
        if self.cv_bitrate_mode != "custom" || util::convert_audio_ext(&self.cv_codec).is_some() {
            return None;
        }
        let total = self.cv_info.as_ref()?.duration?;
        let (start, end) = self.trim_bounds()?;
        let start = start.unwrap_or(0.0).min(total);
        let seconds = end.map(|e| e - start).unwrap_or(total - start).max(0.0);
        // Video plus the 192 kbit/s the profiles use for audio.
        let bits = (self.cv_custom_br as f64 * 1_000_000.0 + 192_000.0) * seconds;
        Some(util::format_bytes((bits / 8.0) as u64))
    }

    fn ui_setup(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t("nav.setup"));
        ui.label(
            RichText::new(self.t("setup.subtitle"))
                .color(Color32::GRAY)
                .size(12.0),
        );
        ui.add_space(8.0);

        let lang = self.lang;
        let missing = self.t("setup.not_installed");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            status_row(
                ui,
                "yt-dlp",
                self.status.ytdlp_ok,
                if self.status.ytdlp_ok {
                    &self.status.ytdlp_version
                } else {
                    missing
                },
            );
            status_row(
                ui,
                "FFmpeg",
                self.status.ffmpeg_ok,
                if self.status.ffmpeg_ok {
                    &self.status.ffmpeg_version
                } else {
                    missing
                },
            );
            status_row(
                ui,
                self.t("setup.deno"),
                self.status.deno_ok,
                if self.status.deno_ok {
                    &self.status.deno_version
                } else {
                    missing
                },
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.t("setup.bin_path")).strong());
                ui.label(
                    RichText::new(self.bin.bin_dir.to_string_lossy())
                        .size(11.0)
                        .color(Color32::GRAY),
                );
            });
        });

        ui.add_space(8.0);
        // Two installers writing the same file at once would corrupt it, so
        // every entry point here is gated on the same flag.
        let idle = !self.setup_busy;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(idle, egui::Button::new(self.t("setup.install")))
                .clicked()
            {
                self.install_binaries();
            }
            ui.label(self.t("setup.channel"));
            if combo(ui, lang, "channel", &mut self.channel, CHANNEL_OPTIONS) {
                self.config.set_str("ytdlp_channel", &self.channel);
            }
            if ui
                .add_enabled(idle, egui::Button::new(self.t("setup.update_ytdlp")))
                .clicked()
            {
                self.update_ytdlp();
            }
            if ui
                .add_enabled(idle, egui::Button::new(self.t("setup.install_deno")))
                .clicked()
            {
                self.install_deno();
            }
            // Versions above come from a cache keyed on the binary files, so a
            // tool installed elsewhere on PATH needs one explicit re-read.
            if ui
                .add_enabled(
                    idle && !self.probing,
                    egui::Button::new(self.t("setup.recheck")),
                )
                .clicked()
            {
                self.start_probe();
            }
        });
        if self.setup_busy {
            ui.add_space(4.0);
            ui.label(
                RichText::new(self.t("setup.busy_hint"))
                    .size(11.0)
                    .color(Color32::GRAY),
            );
        }
        ui.add_space(8.0);
        if !self.setup_log.is_empty() {
            ui.label(&self.setup_log);
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        // The header carries a one-click toggle; this is where the setting
        // itself lives, next to the other things that stick around.
        let mut theme = self.theme.clone();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.t("setup.appearance")).strong());
                combo(ui, lang, "theme", &mut theme, THEME_OPTIONS);
            });
        });
        self.set_theme(&theme);

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new(format!("{APP_NAME} v{VERSION}")).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let idle = !self.update_checking
                    && !matches!(self.update_ui, UpdateUi::Installing(_));
                if ui
                    .add_enabled(idle, egui::Button::new(self.t("update.check_now")))
                    .clicked()
                {
                    self.start_update_check(CheckKind::Manual);
                }
                let l_auto = self.t("update.auto_check");
                if ui.checkbox(&mut self.update_auto, l_auto).changed() {
                    self.config.set_bool("update_check", self.update_auto);
                }
                // Only meaningful once a swap has actually happened.
                if matches!(self.update_ui, UpdateUi::Installed(_))
                    && ui.button(self.t("update.restart")).clicked()
                {
                    self.restart_for_update();
                }
            });
            if !self.update_status.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&self.update_status).size(11.0).color(Color32::GRAY));
            }
        });
    }

    fn restart_for_update(&mut self) {
        self.config.flush(true);
        match update::restart() {
            Ok(_) => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => {
                let msg = self.tf("update.failed", &[&e]);
                self.toast(&msg, true);
            }
        }
    }

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t("nav.history"));
        ui.label(
            RichText::new(self.t("hist.subtitle"))
                .color(Color32::GRAY)
                .size(12.0),
        );
        ui.add_space(8.0);

        let lang = self.lang;
        let matches_filter = |e: &history::Entry, filter: &str| match filter {
            "download" => e.kind == history::Kind::Download,
            "convert" => e.kind == history::Kind::Convert,
            _ => true,
        };
        let shown = self
            .history
            .iter()
            .filter(|e| matches_filter(e, &self.hist_filter))
            .count();

        let mut clear = false;
        ui.horizontal(|ui| {
            combo(
                ui,
                lang,
                "hist_filter",
                &mut self.hist_filter,
                HISTORY_FILTER_OPTIONS,
            );
            ui.label(
                RichText::new(i18n::tf(
                    lang,
                    "hist.count",
                    &[&shown.to_string(), &self.history.len().to_string()],
                ))
                .size(11.0)
                .color(Color32::GRAY),
            );
            with_available_right(ui, |ui| {
                if ui
                    .add_enabled(
                        !self.history.is_empty(),
                        egui::Button::new(i18n::t(lang, "hist.clear")),
                    )
                    .clicked()
                {
                    clear = true;
                }
            });
        });
        ui.add_space(6.0);

        if self.history.is_empty() {
            ui.label(
                RichText::new(self.t("hist.empty"))
                    .color(Color32::GRAY)
                    .size(12.0),
            );
        }

        // Actions are collected and applied after the loop - acting inside it
        // would mean mutating the list the loop is walking.
        let mut open_file: Option<String> = None;
        let mut open_dir: Option<String> = None;
        let mut copy: Option<String> = None;
        let mut reuse: Option<String> = None;
        let mut remove: Option<usize> = None;

        egui::ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, entry) in self.history.iter().enumerate() {
                    if !matches_filter(entry, &self.hist_filter) {
                        continue;
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(entry.kind.icon()).color(accent()));
                            ui.label(RichText::new(&entry.name).strong())
                                .on_hover_text(&entry.path);
                            with_available_right(ui, |ui| {
                                if !entry.url.is_empty() {
                                    if ui
                                        .small_button("↩")
                                        .on_hover_text(i18n::t(lang, "hist.reuse"))
                                        .clicked()
                                    {
                                        reuse = Some(entry.url.clone());
                                    }
                                    if ui
                                        .small_button("🔗")
                                        .on_hover_text(i18n::t(lang, "hist.copy_url"))
                                        .clicked()
                                    {
                                        copy = Some(entry.url.clone());
                                    }
                                }
                                if !entry.path.is_empty()
                                    && ui
                                        .small_button("📋")
                                        .on_hover_text(i18n::t(lang, "hist.copy_path"))
                                        .clicked()
                                {
                                    copy = Some(entry.path.clone());
                                }
                                let exists = entry.exists();
                                if ui
                                    .add_enabled(exists, egui::Button::new("📂").small())
                                    .on_hover_text(i18n::t(lang, "common.open_folder"))
                                    .clicked()
                                {
                                    open_dir = entry.folder();
                                }
                                if ui
                                    .add_enabled(exists, egui::Button::new("▶").small())
                                    .on_hover_text(i18n::t(lang, "common.open_file"))
                                    .clicked()
                                {
                                    open_file = Some(entry.path.clone());
                                }
                                if ui
                                    .small_button("✖")
                                    .on_hover_text(i18n::t(lang, "common.remove"))
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                            });
                        });
                        ui.horizontal_wrapped(|ui| {
                            let dim = Color32::GRAY;
                            if !entry.detail.is_empty() {
                                ui.label(RichText::new(&entry.detail).size(11.0).color(dim));
                                ui.label(RichText::new("·").size(11.0).color(dim));
                            }
                            ui.label(
                                RichText::new(relative_when(lang, entry.when))
                                    .size(11.0)
                                    .color(dim),
                            );
                            if !entry.path.is_empty() && !entry.exists() {
                                ui.label(RichText::new("·").size(11.0).color(dim));
                                ui.label(
                                    RichText::new(i18n::t(lang, "hist.missing"))
                                        .size(11.0)
                                        .color(Color32::from_rgb(255, 183, 77)),
                                );
                            }
                        });
                    });
                    ui.add_space(2.0);
                }
            });

        if let Some(path) = open_file {
            open_path(&path);
        }
        if let Some(dir) = open_dir {
            open_folder(&dir);
        }
        if let Some(text) = copy {
            if copy_to_clipboard(&text) {
                self.toast_key("hist.copied", false);
            }
        }
        if let Some(url) = reuse {
            self.dl_url = url;
            self.tab = TAB_DOWNLOAD;
            self.toast_key("hist.reused", false);
        }
        if let Some(i) = remove {
            self.history.remove(i);
            history::store(&mut self.config, &self.history);
        }
        if clear {
            self.history.clear();
            history::store(&mut self.config, &self.history);
        }
    }

    fn ui_info(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            ui.heading(APP_NAME);
            ui.label(RichText::new(self.tf("info.subtitle", &[VERSION])).color(Color32::GRAY));
            ui.add_space(12.0);
        });
        let features = [
            ("info.f_download", "info.f_download_d"),
            ("info.f_queue", "info.f_queue_d"),
            ("info.f_playlist", "info.f_playlist_d"),
            ("info.f_trim", "info.f_trim_d"),
            ("info.f_history", "info.f_history_d"),
            ("info.f_antibot", "info.f_antibot_d"),
            ("info.f_vegas", "info.f_vegas_d"),
            ("info.f_audio", "info.f_audio_d"),
            ("info.f_quality", "info.f_quality_d"),
            ("info.f_sponsorblock", "info.f_sponsorblock_d"),
            ("info.f_convert", "info.f_convert_d"),
            ("info.f_hardware", "info.f_hardware_d"),
            ("info.f_hdr", "info.f_hdr_d"),
            ("info.f_integrity", "info.f_integrity_d"),
        ];
        egui::Frame::group(ui.style()).show(ui, |ui| {
            for (title, desc) in features {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✔").color(Color32::from_rgb(129, 199, 132)));
                    ui.label(RichText::new(self.t(title)).strong());
                    ui.label(
                        RichText::new(self.t(desc))
                            .color(Color32::GRAY)
                            .size(12.0),
                    );
                });
            }
        });
    }

    /// Keyboard shortcuts: Ctrl+1..5 for the tabs, Ctrl+Enter to start the
    /// visible tab, Esc to stop whatever is running.
    ///
    /// A modal owns the keyboard while it is up, so nothing is read then -
    /// Esc there means "close the dialog", not "cancel my download".
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.pending_conflict.is_some() || !matches!(self.update_ui, UpdateUi::Idle) {
            return;
        }
        enum Action {
            Tab(usize),
            Start,
            Stop,
        }
        let tabs = [
            (TAB_DOWNLOAD, egui::Key::Num1),
            (TAB_CONVERT, egui::Key::Num2),
            (TAB_HISTORY, egui::Key::Num3),
            (TAB_SETUP, egui::Key::Num4),
            (TAB_INFO, egui::Key::Num5),
        ];
        let action = ctx.input(|i| {
            if i.modifiers.command {
                if let Some((tab, _)) = tabs.iter().find(|(_, key)| i.key_pressed(*key)) {
                    return Some(Action::Tab(*tab));
                }
                if i.key_pressed(egui::Key::Enter) {
                    return Some(Action::Start);
                }
            }
            i.key_pressed(egui::Key::Escape).then_some(Action::Stop)
        });
        match action {
            Some(Action::Tab(tab)) => self.tab = tab,
            Some(Action::Start) => match self.tab {
                TAB_DOWNLOAD => self.start_downloads(),
                TAB_CONVERT => self.spawn_convert(),
                _ => {}
            },
            Some(Action::Stop) => {
                if self.dl_busy {
                    self.stop_download();
                }
                if self.cv_busy {
                    self.stop_convert();
                }
            }
            None => {}
        }
    }

    /// Mirrors the running job into the window title, so a minimised window
    /// still says how far along it is.
    fn sync_title(&mut self, ctx: &egui::Context) {
        let base = format!("{APP_NAME} v{VERSION}");
        let running = if self.dl_busy {
            self.dl_progress.map(|p| (p, self.t("nav.download")))
        } else if self.cv_busy {
            self.cv_progress.map(|p| (p, self.t("nav.convert")))
        } else {
            None
        };
        let title = match running {
            Some((p, what)) => format!("{:.0}% {what} - {base}", p * 100.0),
            None => base,
        };
        if title != self.title {
            self.title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn show_conflict_modal(&mut self, ctx: &egui::Context) {
        if self.pending_conflict.is_none() {
            return;
        }
        let target = self.pending_conflict.as_ref().unwrap().target.clone();
        let mut decision: Option<ConflictDecision> = None;
        let body = self.tf("conflict.body", &[&convert::file_name(&target)]);
        egui::Window::new(self.t("conflict.title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(body);
                ui.label(
                    RichText::new(self.t("conflict.hint"))
                        .color(Color32::GRAY)
                        .size(12.0),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(self.t("conflict.save_as_label"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.conflict_name).desired_width(280.0),
                    );
                });
                if let Some(err) = &self.conflict_error {
                    ui.label(RichText::new(err).color(Color32::from_rgb(239, 154, 154)));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(self.t("conflict.save_as")).clicked() {
                        let cleaned = util::sanitize_filename(&self.conflict_name);
                        let stem = util::strip_media_ext(&cleaned)
                            .trim()
                            .trim_end_matches(['.', ' '])
                            .to_string();
                        if stem.is_empty() {
                            self.conflict_error =
                                Some(self.t("conflict.need_name").to_string());
                        } else {
                            let ext = target.extension().map(|e| e.to_string_lossy().to_string());
                            let new_name = match ext {
                                Some(e) => format!("{stem}.{e}"),
                                None => stem,
                            };
                            let new_path = target.with_file_name(new_name);
                            if new_path.exists() {
                                self.conflict_error =
                                    Some(self.t("conflict.name_taken").to_string());
                            } else {
                                decision = Some(ConflictDecision::Rename(new_path));
                            }
                        }
                    }
                    if ui.button(self.t("conflict.overwrite")).clicked() {
                        decision = Some(ConflictDecision::Overwrite);
                    }
                    if ui.button(self.t("conflict.skip")).clicked() {
                        decision = Some(ConflictDecision::Skip);
                    }
                });
            });

        if let Some(d) = decision {
            if let Some(req) = self.pending_conflict.take() {
                let _ = req.reply.send(d);
            }
            self.conflict_error = None;
        }
    }

    fn show_update_modal(&mut self, ctx: &egui::Context) {
        if matches!(self.update_ui, UpdateUi::Idle) {
            return;
        }
        // Snapshot what the modal needs so the closure does not borrow self.
        let (title_version, notes, page_url, has_asset) = match &self.update_ui {
            UpdateUi::Offer(r) | UpdateUi::Installing(r) | UpdateUi::Failed(r) => (
                r.version.clone(),
                r.notes.clone(),
                r.page_url.clone(),
                r.asset.is_some(),
            ),
            UpdateUi::Installed(v) => (v.clone(), String::new(), String::new(), true),
            UpdateUi::Idle => unreachable!(),
        };
        let installing = matches!(self.update_ui, UpdateUi::Installing(_));
        let installed = matches!(self.update_ui, UpdateUi::Installed(_));
        let failed = matches!(self.update_ui, UpdateUi::Failed(..));
        let offering = matches!(self.update_ui, UpdateUi::Offer(_));

        let body = if installed {
            self.tf("update.installed", &[&title_version])
        } else {
            self.tf("update.body", &[&title_version, VERSION])
        };
        let status = self.update_status.clone();

        let mut install = false;
        let mut restart = false;
        let mut open_page = false;
        let mut close = false;
        let mut skip = false;

        egui::Window::new(self.t("update.title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.label(RichText::new(body).strong());

                if offering && !notes.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.t("update.notes"))
                            .size(12.0)
                            .color(Color32::GRAY),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("update_notes")
                        .max_height(180.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&notes).size(12.0));
                        });
                }
                if offering && !has_asset {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.t("update.no_asset"))
                            .size(11.0)
                            .color(Color32::from_rgb(255, 183, 77)),
                    );
                }
                if (installing || failed) && !status.is_empty() {
                    ui.add_space(6.0);
                    let color = if failed {
                        Color32::from_rgb(239, 154, 154)
                    } else {
                        Color32::GRAY
                    };
                    ui.label(RichText::new(&status).size(12.0).color(color));
                }
                if installing {
                    ui.add_space(4.0);
                    ui.add(egui::ProgressBar::new(0.0).desired_height(4.0).animate(true));
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if offering {
                        if ui
                            .add_enabled(has_asset, egui::Button::new(self.t("update.install")))
                            .clicked()
                        {
                            install = true;
                        }
                        if ui.button(self.t("update.open_page")).clicked() {
                            open_page = true;
                        }
                        if ui.button(self.t("update.later")).clicked() {
                            close = true;
                        }
                        if ui.button(self.t("update.skip")).clicked() {
                            skip = true;
                        }
                    } else if installed {
                        if ui.button(self.t("update.restart")).clicked() {
                            restart = true;
                        }
                        if ui.button(self.t("update.later")).clicked() {
                            close = true;
                        }
                    } else if failed {
                        if ui.button(self.t("update.open_page")).clicked() {
                            open_page = true;
                        }
                        if ui.button(self.t("update.later")).clicked() {
                            close = true;
                        }
                    }
                });
            });

        if open_page {
            open_url(&page_url);
            close = true;
        }
        if skip {
            self.config.set_str("update_skip_version", &title_version);
            close = true;
        }
        if install {
            if let UpdateUi::Offer(r) = std::mem::replace(&mut self.update_ui, UpdateUi::Idle) {
                self.start_update_install(r);
            }
        } else if restart {
            self.restart_for_update();
        } else if close {
            self.update_ui = UpdateUi::Idle;
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -20.0])
            .show(ctx, |ui| {
                for t in &self.toasts {
                    let bg = if t.error {
                        Color32::from_rgb(140, 30, 30)
                    } else {
                        Color32::from_rgb(30, 110, 50)
                    };
                    egui::Frame::none()
                        .fill(bg)
                        .inner_margin(8.0)
                        .rounding(6.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&t.msg).color(Color32::WHITE));
                        });
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.handle_shortcuts(ctx);
        self.sync_title(ctx);

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(4.0);
            self.ui_header(ui);
            ui.add_space(4.0);
        });
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.add_space(2.0);
            self.ui_footer(ui);
            ui.add_space(2.0);
        });
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(190.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);
                self.ui_nav(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.tab {
                    TAB_DOWNLOAD => self.ui_download(ui, ctx),
                    TAB_CONVERT => self.ui_convert(ui, ctx),
                    TAB_HISTORY => self.ui_history(ui),
                    TAB_SETUP => self.ui_setup(ui),
                    _ => self.ui_info(ui),
                });
        });

        self.show_conflict_modal(ctx);
        self.show_update_modal(ctx);
        self.show_toasts(ctx);

        // Coalesced write of anything the frame changed.
        self.config.flush(false);

        if self.dl_busy || self.cv_busy {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.config.flush(true);
    }
}

// ---------------- free helpers ----------------

/// The one colour the app calls its own, readable on both themes.
fn accent() -> Color32 {
    Color32::from_rgb(38, 166, 189)
}

/// Installs the visuals and the spacing both themes share.
///
/// Only the palette differs between them; every rounding, margin and font
/// size is set once here so the two cannot drift apart.
fn apply_style(ctx: &egui::Context, theme: &str) {
    let dark = theme != "light";
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.selection.bg_fill = accent().linear_multiply(if dark { 0.45 } else { 0.30 });
    visuals.selection.stroke.color = if dark {
        Color32::from_rgb(224, 247, 250)
    } else {
        Color32::from_rgb(10, 60, 70)
    };
    visuals.hyperlink_color = accent();
    let rounding = egui::Rounding::same(6.0);
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.open.rounding = rounding;
    visuals.window_rounding = egui::Rounding::same(8.0);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.indent = 18.0;
    ctx.set_style(style);
}

fn is_http_url(url: &str) -> bool {
    let lo = url.to_lowercase();
    lo.starts_with("http://") || lo.starts_with("https://")
}

fn copy_to_clipboard(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_string()))
        .is_ok()
}

/// Symbol and colour for one queue state.
fn queue_marks(state: QueueState) -> (&'static str, Color32) {
    match state {
        QueueState::Pending => ("○", Color32::GRAY),
        QueueState::Running => ("▶", accent()),
        QueueState::Done => ("✔", Color32::from_rgb(129, 199, 132)),
        QueueState::Failed => ("✖", Color32::from_rgb(239, 154, 154)),
        QueueState::Skipped => ("–", Color32::from_rgb(255, 183, 77)),
    }
}

fn queue_state_key(state: QueueState) -> &'static str {
    match state {
        QueueState::Pending => "qs.pending",
        QueueState::Running => "qs.running",
        QueueState::Done => "qs.done",
        QueueState::Failed => "qs.failed",
        QueueState::Skipped => "qs.skipped",
    }
}

/// Shortens a URL for the queue list; the full one stays in the tooltip.
fn short_url(url: &str) -> String {
    const MAX: usize = 72;
    let trimmed = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    // Cut on character boundaries - a URL may well carry non-ASCII.
    let head: String = trimmed.chars().take(MAX - 12).collect();
    let tail: String = {
        let all: Vec<char> = trimmed.chars().collect();
        all[all.len() - 9..].iter().collect()
    };
    format!("{head}...{tail}")
}

/// How long ago something happened, in the coarsest unit that still says
/// something. Absolute dates would need a calendar and a timezone; the
/// history is read as "what did I just do", for which this is enough.
fn relative_when(lang: Lang, when: u64) -> String {
    if when == 0 {
        return String::new();
    }
    let now = history::now_secs();
    let secs = now.saturating_sub(when);
    if secs < 90 {
        return i18n::t(lang, "when.now").to_string();
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return i18n::tf(lang, "when.minutes", &[&minutes.to_string()]);
    }
    let hours = minutes / 60;
    if hours < 24 {
        return i18n::tf(lang, "when.hours", &[&hours.to_string()]);
    }
    i18n::tf(lang, "when.days", &[&(hours / 24).to_string()])
}

fn label_of(lang: Lang, current: &str, options: &[(&'static str, &'static str)]) -> String {
    options
        .iter()
        .find(|(v, _)| *v == current)
        .map(|(_, key)| i18n::t(lang, key).to_string())
        .unwrap_or_else(|| current.to_string())
}

fn combo(
    ui: &mut egui::Ui,
    lang: Lang,
    id: &str,
    current: &mut String,
    options: &[(&'static str, &'static str)],
) -> bool {
    let before = current.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(label_of(lang, current, options))
        .show_ui(ui, |ui| {
            for (val, key) in options {
                ui.selectable_value(current, (*val).to_string(), i18n::t(lang, key));
            }
        });
    before != *current
}

fn chip(ui: &mut egui::Ui, name: &str, ok: bool, value: &str) {
    let bg = if ok {
        Color32::from_rgb(27, 67, 40)
    } else {
        Color32::from_rgb(80, 27, 27)
    };
    egui::Frame::none()
        .fill(bg)
        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
        .rounding(8.0)
        .show(ui, |ui| {
            let short: String = value.chars().take(16).collect();
            ui.label(
                RichText::new(format!("{name}: {short}"))
                    .size(11.0)
                    .color(Color32::WHITE),
            );
        });
}

fn status_row(ui: &mut egui::Ui, label: &str, ok: bool, value: &str) {
    ui.horizontal(|ui| {
        let mark = if ok { "✔" } else { "✖" };
        let color = if ok {
            Color32::from_rgb(129, 199, 132)
        } else {
            Color32::from_rgb(239, 154, 154)
        };
        ui.label(RichText::new(mark).color(color));
        ui.label(RichText::new(label).strong());
        ui.label(RichText::new(value).color(Color32::GRAY));
    });
}

/// The i18n key of the caveat shown under the codec picker, if any.
fn codec_hint(key: &str, audio_only: bool) -> Option<&'static str> {
    if audio_only {
        Some("cv.hint_audio_only")
    } else if key.contains("prores") || key.contains("dnxhr") || key == "vegas_fix" {
        Some("cv.hint_cpu")
    } else if key == "copy" {
        Some("cv.hint_copy")
    } else if key.contains("av1") {
        Some("cv.hint_av1")
    } else {
        None
    }
}

/// Colour of one log line. Both palettes carry the same four meanings; the
/// light one only darkens them enough to stay readable on a pale background.
fn log_color(line: &str, dark: bool) -> Color32 {
    let pick = |d: (u8, u8, u8), l: (u8, u8, u8)| {
        let (r, g, b) = if dark { d } else { l };
        Color32::from_rgb(r, g, b)
    };
    if line.starts_with("===") || line.starts_with("---") {
        return pick((159, 168, 218), (63, 81, 181));
    }
    let lo = line.to_lowercase();
    for kw in ["error", "failed", "traceback", "warning", "warn"] {
        if lo.contains(kw) {
            return pick((239, 154, 154), (183, 28, 28));
        }
    }
    for kw in ["success", "completed", "complete"] {
        if lo.contains(kw) {
            return pick((165, 214, 167), (27, 94, 32));
        }
    }
    for kw in ["download:", "%|", "frame=", "fps=", "speed=", "bitrate="] {
        if lo.contains(kw) {
            return pick((128, 222, 234), (0, 105, 122));
        }
    }
    pick((200, 200, 200), (55, 55, 60))
}

const LOG_FONT_SIZE: f32 = 11.0;

fn log_view(ui: &mut egui::Ui, id: &str, lines: &[String], height: f32, dark: bool) {
    // Rows are uniform monospace, so only the visible slice needs widgets.
    // Emitting all of them meant up to MAX_LOG labels per frame while a job
    // was running and repaints were frequent.
    let row_height = ui.fonts(|f| f.row_height(&egui::FontId::monospace(LOG_FONT_SIZE)))
        + ui.spacing().item_spacing.y;
    let fill = if dark {
        Color32::from_rgb(20, 20, 24)
    } else {
        Color32::from_rgb(244, 245, 248)
    };
    egui::Frame::none()
        .fill(fill)
        .inner_margin(8.0)
        .rounding(6.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show_rows(ui, row_height, lines.len(), |ui, range| {
                    for line in &lines[range] {
                        // A truly empty label collapses to zero height and
                        // would desynchronise the virtualised row offsets.
                        let text = if line.is_empty() { " " } else { line.as_str() };
                        ui.label(
                            RichText::new(text)
                                .monospace()
                                .size(LOG_FONT_SIZE)
                                .color(log_color(line, dark)),
                        );
                    }
                });
        });
}

fn with_available_right<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
        .inner
}

/// A stored probe result together with what it describes.
#[derive(serde::Serialize, serde::Deserialize)]
struct ProbeCache {
    /// App version the entry was written by - a new build re-probes once
    /// rather than trusting a result an older probe produced.
    app: String,
    fingerprint: String,
    status: BinaryStatus,
}

/// Returns the cached probe result when it still matches the binaries on disk.
fn probe_cache_load(config: &Config, bin: &Binaries) -> Option<BinaryStatus> {
    let fingerprint = bin.fingerprint()?;
    let cached: ProbeCache =
        serde_json::from_value(config.get_value(PROBE_CACHE_KEY)?.clone()).ok()?;
    (cached.app == VERSION && cached.fingerprint == fingerprint).then_some(cached.status)
}

fn probe_cache_store(config: &mut Config, bin: &Binaries, status: &BinaryStatus) {
    let Some(fingerprint) = bin.fingerprint() else {
        return;
    };
    let entry = ProbeCache {
        app: VERSION.to_string(),
        fingerprint,
        status: status.clone(),
    };
    if let Ok(value) = serde_json::to_value(entry) {
        config.set_value(PROBE_CACHE_KEY, value);
    }
}

fn progress_text(lang: Lang, prefix: &str, written: u64, total: u64) -> String {
    let mb = written as f64 / (1024.0 * 1024.0);
    if total > 0 {
        let pct = written as f64 * 100.0 / total as f64;
        i18n::tf(
            lang,
            "setup.progress_pct",
            &[prefix, &format!("{pct:.0}"), &format!("{mb:.1}")],
        )
    } else {
        i18n::tf(lang, "setup.progress_plain", &[prefix, &format!("{mb:.1}")])
    }
}

fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .ok()?
        .get_text()
        .ok()
        .map(|s| s.trim().to_string())
}

fn clipboard_url() -> Option<String> {
    let t = clipboard_text()?;
    let re = regex::Regex::new(r"^https?://\S+$").ok()?;
    if re.is_match(&t) {
        Some(t)
    } else {
        None
    }
}

/// Opens an https URL in the default browser.
fn open_url(url: &str) {
    // Handing an arbitrary string to the shell is how "open this link" turns
    // into "run this command", so only plain https URLs get through.
    if !url.starts_with("https://") || url.contains(char::is_whitespace) {
        return;
    }
    #[cfg(windows)]
    {
        // `explorer <url>` avoids cmd.exe's argument parsing entirely.
        let _ = std::process::Command::new("explorer").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Hands a file to whatever the desktop opens it with.
///
/// Only a path that exists is passed on, and it goes to the opener as a
/// single argument, so nothing in the name can turn into a second one.
fn open_path(path: &str) {
    if path.is_empty() || !Path::new(path).is_file() {
        return;
    }
    let p = PathBuf::from(path);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(p).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

fn open_folder(path: &str) {
    if path.is_empty() || !Path::new(path).exists() {
        return;
    }
    let p = PathBuf::from(path);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(p).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_are_accepted_for_the_queue() {
        assert!(is_http_url("https://example.com/v"));
        assert!(is_http_url("HTTP://example.com/v"));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("example.com/v"));
        assert!(!is_http_url(""));
    }

    #[test]
    fn a_long_url_is_shortened_without_splitting_a_character() {
        let short = "https://youtu.be/abc";
        assert_eq!(short_url(short), "youtu.be/abc");
        let long = format!("https://example.com/{}", "ä".repeat(200));
        let out = short_url(&long);
        assert!(out.contains("..."), "{out}");
        assert!(out.chars().count() < 100, "{out}");
        // The tail is the end of the URL, so two similar links stay apart.
        assert!(out.ends_with(&"ä".repeat(9)), "{out}");
    }

    #[test]
    fn every_queue_state_has_a_translated_label() {
        for state in [
            QueueState::Pending,
            QueueState::Running,
            QueueState::Done,
            QueueState::Failed,
            QueueState::Skipped,
        ] {
            let key = queue_state_key(state);
            for lang in i18n::LANGS {
                assert_ne!(i18n::t(lang, key), key, "missing {key} for {lang:?}");
            }
        }
    }
}
