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
use crate::i18n::{self, Lang};
use crate::types::{BinaryStatus, ConflictDecision, ConflictReq, DownloadOpts, Task, UiMsg};
use crate::util;

const MAX_LOG: usize = 1200;

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
    probed: bool,
    tab: usize,
    lang: Lang,
    toasts: Vec<Toast>,
    setup_log: String,
    setup_busy: bool,
    channel: String,

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
    dl_advanced: bool,
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        ctx.set_visuals(egui::Visuals::dark());

        let bin = Arc::new(Binaries::new());
        let config = Config::load(&bin.app_dir);
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
            dl_advanced: false,
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
            cv_log: vec![i18n::t(lang, "log.ready_conversion").to_string()],
            cv_status: i18n::t(lang, "status.ready").to_string(),
            cv_progress: None,
            cv_busy: false,
            cv_cancel: Arc::new(AtomicBool::new(false)),
            cv_child: Arc::new(Mutex::new(None)),

            pending_conflict: None,
            conflict_name: String::new(),
            conflict_error: None,

            status: BinaryStatus::default(),
            config,
            tx,
            rx,
            probed: false,
        };

        // Prefill the URL: a clipboard URL wins, otherwise the last used one.
        app.dl_url = clipboard_url().unwrap_or_else(|| app.config.get_str("last_url", ""));

        // Kick off the background binary probe.
        app.start_probe();
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
        self.probed = true;
        let bin = self.bin.clone();
        let em = self.emitter();
        thread::spawn(move || {
            em.send(UiMsg::Binary(bin.probe_status()));
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
                        }
                    }
                    Task::Convert => {
                        self.cv_busy = b;
                        if !b {
                            self.cv_progress = None;
                        }
                    }
                },
                UiMsg::Toast(msg, error) => self.toasts.push(Toast {
                    msg,
                    error,
                    until: Instant::now() + Duration::from_millis(3000),
                }),
                UiMsg::SetupLog(s) => self.setup_log = s,
                UiMsg::SetupBusy(b) => self.setup_busy = b,
                UiMsg::Binary(status) => {
                    if !status.impersonate_ok {
                        self.dl_impersonate = false;
                    }
                    self.status = status;
                }
                UiMsg::Conflict(req) => {
                    self.conflict_name = req.suggestion.clone();
                    self.conflict_error = None;
                    self.pending_conflict = Some(req);
                }
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

    fn spawn_download(&mut self) {
        if self.dl_busy {
            return;
        }
        let url = self.dl_url.trim().to_string();
        let out_dir = self.dl_output.trim().to_string();
        if url.is_empty() {
            return self.toast_key("toast.need_url", true);
        }
        if !(url.to_lowercase().starts_with("http://") || url.to_lowercase().starts_with("https://"))
        {
            return self.toast_key("toast.bad_url", true);
        }
        if out_dir.is_empty() {
            return self.toast_key("toast.need_folder", true);
        }
        if !self.status.ytdlp_ok {
            return self.toast_key("toast.no_ytdlp", true);
        }
        let cookiefile = if self.dl_cookies == "cookiefile" {
            self.dl_cookiefile.trim().to_string()
        } else {
            String::new()
        };
        if self.dl_cookies == "cookiefile"
            && (cookiefile.is_empty() || !Path::new(&cookiefile).is_file())
        {
            return self.toast_key("toast.no_cookiefile", true);
        }
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            let msg = self.tf("toast.mkdir_failed", &[&e.to_string()]);
            return self.toast(&msg, true);
        }

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
        };
        let quality = self.dl_quality.clone();

        self.dl_log.clear();
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

        let params = ConvertParams {
            codec_key: self.cv_codec.clone(),
            hw_setting: self.cv_hw.clone(),
            crf: self.cv_crf as i32,
            use_custom: self.cv_bitrate_mode == "custom",
            custom_br: self.cv_custom_br as i32,
            preserve_color: self.cv_preserve_color,
        };

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
        self.dl_cancel.store(true, Ordering::SeqCst);
        if let Some(c) = util::lock(&self.dl_child).as_mut() {
            let _ = c.kill();
        }
        self.dl_status = self.t("status.cancelled").to_string();
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
            let lang = self.lang;
            with_available_right(ui, |ui| {
                let (yt_ok, yt) = (self.status.ytdlp_ok, &self.status.ytdlp_version);
                let (ff_ok, ff) = (self.status.ffmpeg_ok, &self.status.ffmpeg_version);
                chip(ui, "FFmpeg", ff_ok, if ff_ok { ff } else { "x" });
                chip(ui, "yt-dlp", yt_ok, if yt_ok { yt } else { "x" });
                ui.add_space(8.0);
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
        let items = ["nav.download", "nav.convert", "nav.setup", "nav.info"];
        for (i, key) in items.iter().enumerate() {
            let selected = self.tab == i;
            if ui
                .selectable_label(selected, RichText::new(self.t(key)).size(15.0))
                .clicked()
            {
                self.tab = i;
            }
            ui.add_space(4.0);
        }
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
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.dl_busy, egui::Button::new(self.t("dl.start")))
                .clicked()
            {
                self.spawn_download();
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
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.t("common.live_log")).strong());
            if ui.button(self.t("common.clear")).clicked() {
                self.dl_log.clear();
            }
        });
        log_view(ui, "dl_log", &self.dl_log, 220.0);
        let _ = ctx;
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
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.cv_input).desired_width(f32::INFINITY));
        });
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
            if ui.button(self.t("cv.clear_log")).clicked() {
                self.cv_log.clear();
            }
        });
        if let Some(p) = self.cv_progress {
            ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        }
        ui.label(&self.cv_status);

        ui.add_space(6.0);
        ui.label(RichText::new(self.t("common.live_log")).strong());
        log_view(ui, "cv_log", &self.cv_log, 260.0);
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
                    0 => self.ui_download(ui, ctx),
                    1 => self.ui_convert(ui, ctx),
                    2 => self.ui_setup(ui),
                    _ => self.ui_info(ui),
                });
        });

        self.show_conflict_modal(ctx);
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

fn log_color(line: &str) -> Color32 {
    if line.starts_with("===") || line.starts_with("---") {
        return Color32::from_rgb(159, 168, 218);
    }
    let lo = line.to_lowercase();
    for kw in ["error", "failed", "traceback", "warning", "warn"] {
        if lo.contains(kw) {
            return Color32::from_rgb(239, 154, 154);
        }
    }
    for kw in ["success", "completed", "complete"] {
        if lo.contains(kw) {
            return Color32::from_rgb(165, 214, 167);
        }
    }
    for kw in ["download:", "%|", "frame=", "fps=", "speed=", "bitrate="] {
        if lo.contains(kw) {
            return Color32::from_rgb(128, 222, 234);
        }
    }
    Color32::from_rgb(200, 200, 200)
}

const LOG_FONT_SIZE: f32 = 11.0;

fn log_view(ui: &mut egui::Ui, id: &str, lines: &[String], height: f32) {
    // Rows are uniform monospace, so only the visible slice needs widgets.
    // Emitting all of them meant up to MAX_LOG labels per frame while a job
    // was running and repaints were frequent.
    let row_height = ui.fonts(|f| f.row_height(&egui::FontId::monospace(LOG_FONT_SIZE)))
        + ui.spacing().item_spacing.y;
    egui::Frame::none()
        .fill(Color32::from_rgb(20, 20, 24))
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
                                .color(log_color(line)),
                        );
                    }
                });
        });
}

fn with_available_right<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
        .inner
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
