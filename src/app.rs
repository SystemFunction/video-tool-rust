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
use crate::types::{
    BinaryStatus, ConflictDecision, ConflictReq, DownloadOpts, Task, UiMsg,
};
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
    toasts: Vec<Toast>,
    setup_log: String,
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

        let mut app = App {
            ctx: ctx.clone(),
            bin: bin.clone(),
            tab: 0,
            toasts: Vec::new(),
            setup_log: String::new(),
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
                "  Ready for downloads ...".into(),
            ],
            dl_status: "Ready".into(),
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
            cv_log: vec!["  Ready for conversion ...".into()],
            cv_status: "Ready".into(),
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
            return self.toast("Please enter a URL first.", true);
        }
        if !(url.to_lowercase().starts_with("http://") || url.to_lowercase().starts_with("https://")) {
            return self.toast("URL must start with http:// or https://.", true);
        }
        if out_dir.is_empty() {
            return self.toast("Please choose a save folder.", true);
        }
        if !self.status.ytdlp_ok {
            return self.toast("yt-dlp not available - please run Setup.", true);
        }
        let cookiefile = if self.dl_cookies == "cookiefile" {
            self.dl_cookiefile.trim().to_string()
        } else {
            String::new()
        };
        if self.dl_cookies == "cookiefile" && (cookiefile.is_empty() || !Path::new(&cookiefile).is_file()) {
            return self.toast("Cookies file (cookies.txt) is missing or not found.", true);
        }
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            return self.toast(&format!("Could not create folder: {e}"), true);
        }

        self.config.set_str("last_output_folder", &out_dir);
        self.config.set_str("last_url", &url);

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
                if s.is_empty() { "en,de".into() } else { s.to_string() }
            },
            cookiefile,
            potoken: self.dl_potoken,
            potoken_url: self.dl_potoken_url.trim().to_string(),
            plugins_dir: self.bin.plugins_dir.to_string_lossy().to_string(),
            conflict: self.dl_conflict.clone(),
        };
        let quality = self.dl_quality.clone();

        self.dl_log.clear();
        self.dl_status = "Downloading ...".into();
        self.dl_progress = Some(0.0);
        self.dl_busy = true;
        self.dl_cancel.store(false, Ordering::SeqCst);
        self.push_log(Task::Download, &format!("=== Download started ===\n{url}\n"));

        let dctx = DlCtx {
            bin: self.bin.clone(),
            em: self.emitter(),
            cancel: self.dl_cancel.clone(),
            child: self.dl_child.clone(),
            js_runtime: self.status.js_runtime.clone(),
            ytdlp_version: self.status.ytdlp_version.clone(),
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
            return self.toast("Please choose input and output files.", true);
        }
        if !Path::new(&inp).exists() {
            return self.toast("Input file does not exist.", true);
        }
        if let Some(want) = util::convert_audio_ext(&self.cv_codec) {
            let cur = Path::new(&out)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if cur != want {
                out = Path::new(&out).with_extension(want).to_string_lossy().to_string();
                self.cv_output = out.clone();
            }
        }
        if let (Ok(a), Ok(b)) = (std::fs::canonicalize(&inp), std::fs::canonicalize(&out)) {
            if a == b {
                return self.toast("Input and output must not be identical.", true);
            }
        }
        if !self.status.ffmpeg_ok {
            return self.toast("FFmpeg not available - please run Setup.", true);
        }
        if let Some(parent) = Path::new(&out).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return self.toast(&format!("Could not create target folder: {e}"), true);
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
        self.cv_status = "Converting ...".into();
        self.cv_progress = Some(0.0);
        self.cv_busy = true;
        self.cv_cancel.store(false, Ordering::SeqCst);
        let name = convert::file_name(Path::new(&inp));
        self.push_log(Task::Convert, &format!("=== Conversion started ===\n{name}\n"));

        let cctx = CvCtx {
            bin: self.bin.clone(),
            em: self.emitter(),
            cancel: self.cv_cancel.clone(),
            child: self.cv_child.clone(),
        };
        thread::spawn(move || {
            convert::run_conversion(cctx, inp, out, params);
        });
    }

    fn stop_download(&mut self) {
        self.dl_cancel.store(true, Ordering::SeqCst);
        if let Some(c) = self.dl_child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
        self.dl_status = "Cancelled".into();
    }

    fn stop_convert(&mut self) {
        self.cv_cancel.store(true, Ordering::SeqCst);
        if let Some(c) = self.cv_child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
        self.cv_status = "Cancelled".into();
    }

    // ---------------- setup workers ----------------

    fn install_binaries(&mut self) {
        self.setup_log = "Installing binaries ...".into();
        let bin = self.bin.clone();
        let em = self.emitter();
        thread::spawn(move || {
            em.setup_log("Downloading yt-dlp ...");
            let r = bin.install_ytdlp(|w, t| {
                em.setup_log(progress_text("Downloading yt-dlp", w, t));
            });
            if let Err(e) = r {
                em.setup_log(format!("Error: {e}"));
                em.toast(format!("Installation error: {e}"), true);
                return;
            }
            em.setup_log("Downloading FFmpeg ...");
            let r = bin.install_ffmpeg(|name, w, t| {
                em.setup_log(progress_text(&format!("Downloading {name}"), w, t));
            });
            if let Err(e) = r {
                em.setup_log(format!("Error: {e}"));
                em.toast(format!("Installation error: {e}"), true);
                return;
            }
            em.setup_log("Installation completed");
            em.send(UiMsg::Binary(bin.probe_status()));
            em.toast("Binaries installed successfully", false);
        });
    }

    fn update_ytdlp(&mut self) {
        let channel = self.channel.clone();
        self.setup_log = if channel == "stable" {
            "Updating yt-dlp ...".into()
        } else {
            format!("Switching yt-dlp to '{channel}' channel ...")
        };
        let bin = self.bin.clone();
        let em = self.emitter();
        thread::spawn(move || {
            let r = if channel == "stable" {
                bin.install_ytdlp(|w, t| em.setup_log(progress_text("Updating yt-dlp", w, t)))
            } else {
                em.setup_log(format!("Switching yt-dlp to '{channel}' channel ..."));
                bin.update_channel(&channel).map(|out| {
                    if !out.is_empty() {
                        em.log(Task::Download, out);
                    }
                })
            };
            match r {
                Ok(_) => {
                    em.setup_log("yt-dlp updated");
                    em.send(UiMsg::Binary(bin.probe_status()));
                    em.toast("yt-dlp updated successfully", false);
                }
                Err(e) => {
                    em.setup_log(format!("Error: {e}"));
                    em.toast(format!("Update error: {e}"), true);
                }
            }
        });
    }

    fn install_deno(&mut self) {
        self.setup_log = "Downloading Deno ...".into();
        let bin = self.bin.clone();
        let em = self.emitter();
        thread::spawn(move || {
            let r = bin.install_deno(|w, t| em.setup_log(progress_text("Downloading Deno", w, t)));
            match r {
                Ok(_) => {
                    em.setup_log("Deno installed");
                    em.send(UiMsg::Binary(bin.probe_status()));
                    em.toast("Deno (JS runtime) installed", false);
                }
                Err(e) => {
                    em.setup_log(format!("Error: {e}"));
                    em.toast(format!("Deno installation error: {e}"), true);
                }
            }
        });
    }

    fn toast(&mut self, msg: &str, error: bool) {
        self.toasts.push(Toast {
            msg: msg.to_string(),
            error,
            until: Instant::now() + Duration::from_millis(3000),
        });
    }

    // ---------------- UI panels ----------------

    fn ui_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(APP_NAME).color(Color32::from_rgb(77, 208, 225)));
            ui.label(RichText::new(format!("v{VERSION}")).color(Color32::from_rgb(178, 235, 242)));
            ui.add_space(16.0);
            with_available_right(ui, |ui| {
                let (yt_ok, yt) = (self.status.ytdlp_ok, &self.status.ytdlp_version);
                let (ff_ok, ff) = (self.status.ffmpeg_ok, &self.status.ffmpeg_version);
                chip(ui, "FFmpeg", ff_ok, if ff_ok { ff } else { "x" });
                chip(ui, "yt-dlp", yt_ok, if yt_ok { yt } else { "x" });
            });
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
            ui.label(RichText::new(format!("HW: {hw}")).size(11.0).color(dim));
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "Impersonate: {}",
                    if self.status.impersonate_ok { "on" } else { "off" }
                ))
                .size(11.0)
                .color(dim),
            );
            ui.separator();
            let js = if self.status.deno_ok { self.status.deno_version.as_str() } else { "-" };
            ui.label(RichText::new(format!("JS: {js}")).size(11.0).color(dim));
            ui.separator();
            ui.label(
                RichText::new(format!("{} {}", self.bin.system, std::env::consts::ARCH))
                    .size(11.0)
                    .color(dim),
            );
            with_available_right(ui, |ui| {
                ui.label(RichText::new(format!("{APP_NAME} v{VERSION}")).size(11.0).color(dim));
            });
        });
    }

    fn ui_nav(&mut self, ui: &mut egui::Ui) {
        let items = ["Download", "Convert", "Setup", "Info"];
        for (i, label) in items.iter().enumerate() {
            let selected = self.tab == i;
            if ui
                .selectable_label(selected, RichText::new(*label).size(15.0))
                .clicked()
            {
                self.tab = i;
            }
            ui.add_space(4.0);
        }
    }

    fn ui_download(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Download");
        ui.label(
            RichText::new("Videos & audio from YouTube, Instagram, TikTok, X and 1700+ sites")
                .color(Color32::GRAY)
                .size(12.0),
        );
        ui.add_space(8.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new("Source & Quality").strong());
            ui.horizontal(|ui| {
                ui.label("URL:");
                ui.add(egui::TextEdit::singleline(&mut self.dl_url).desired_width(f32::INFINITY).hint_text("YouTube, TikTok, X/Twitter, Instagram ..."));
            });
            ui.horizontal(|ui| {
                if ui.button("📋 Paste").clicked() {
                    if let Some(t) = clipboard_text() {
                        self.dl_url = t;
                    }
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("dl_opts").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Quality");
                if combo(ui, "dl_quality", &mut self.dl_quality, QUALITY_OPTIONS) {
                    self.config.set_str("last_quality", &self.dl_quality);
                }
                ui.end_row();
                ui.label("Cookies");
                if combo(ui, "dl_cookies", &mut self.dl_cookies, COOKIES_OPTIONS) {
                    self.config.set_str("last_cookies", &self.dl_cookies);
                }
                ui.end_row();
                ui.label("If file exists");
                if combo(ui, "dl_conflict", &mut self.dl_conflict, CONFLICT_OPTIONS) {
                    self.config.set_str("conflict_mode", &self.dl_conflict);
                }
                ui.end_row();
            });

            if self.dl_cookies == "cookiefile" {
                ui.horizontal(|ui| {
                    ui.label("cookies.txt:");
                    ui.add(egui::TextEdit::singleline(&mut self.dl_cookiefile).desired_width(f32::INFINITY));
                    if ui.button("Choose").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("cookies", &["txt"]).pick_file() {
                            self.dl_cookiefile = p.to_string_lossy().to_string();
                            self.config.set_str("cookies_file", &self.dl_cookiefile);
                        }
                    }
                });
            }

            ui.add_space(4.0);
            let arrow = if self.dl_advanced { "▼" } else { "▶" };
            if ui.selectable_label(false, format!("{arrow} Advanced options")).clicked() {
                self.dl_advanced = !self.dl_advanced;
            }
            if self.dl_advanced {
                ui.indent("adv", |ui| {
                    if ui.add_enabled(self.status.impersonate_ok, egui::Checkbox::new(&mut self.dl_impersonate, "Impersonate (Anti-Bot)")).changed() {
                        self.config.set_bool("impersonate", self.dl_impersonate);
                    }
                    if ui.checkbox(&mut self.dl_sponsorblock, "SponsorBlock (Remove Sponsors)").changed() {
                        self.config.set_bool("sponsorblock", self.dl_sponsorblock);
                    }
                    if ui.checkbox(&mut self.dl_embed, "Embed Thumbnail/Metadata/Chapters").changed() {
                        self.config.set_bool("embed", self.dl_embed);
                    }
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut self.dl_subs, "Download Subtitles").changed() {
                            self.config.set_bool("subs", self.dl_subs);
                        }
                        ui.label("Languages:");
                        if ui.add(egui::TextEdit::singleline(&mut self.dl_subs_lang).desired_width(120.0)).changed() {
                            self.config.set_str("subs_lang", &self.dl_subs_lang);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut self.dl_potoken, "PO Token / mweb (for 18+ Videos)").changed() {
                            self.config.set_bool("potoken", self.dl_potoken);
                        }
                        ui.label("Provider URL:");
                        if ui.add(egui::TextEdit::singleline(&mut self.dl_potoken_url).desired_width(220.0).hint_text("empty = http://127.0.0.1:4416")).changed() {
                            self.config.set_str("potoken_url", &self.dl_potoken_url);
                        }
                    });
                });
            }
        });

        ui.add_space(6.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new("Save Location").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.dl_output).desired_width(f32::INFINITY));
                if ui.button("📁").on_hover_text("Choose folder").clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title("Choose save folder").pick_folder() {
                        self.dl_output = p.to_string_lossy().to_string();
                        self.config.set_str("last_output_folder", &self.dl_output);
                    }
                }
                if ui.button("↗").on_hover_text("Open folder").clicked() {
                    open_folder(&self.dl_output);
                }
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.dl_busy, egui::Button::new("⬇ Start Download")).clicked() {
                self.spawn_download();
            }
            if ui.add_enabled(self.dl_busy, egui::Button::new("⏹ Stop")).clicked() {
                self.stop_download();
            }
            ui.label(&self.dl_status);
        });
        if let Some(p) = self.dl_progress {
            ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Live Log").strong());
            if ui.button("Clear").clicked() {
                self.dl_log.clear();
            }
        });
        log_view(ui, "dl_log", &self.dl_log, 220.0);
        let _ = ctx;
    }

    fn ui_convert(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.heading("Convert");
        ui.label(RichText::new("FFmpeg conversion with live progress").color(Color32::GRAY).size(12.0));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Input:");
            // Button first so it stays visible - the TextEdit below fills the
            // rest of the row (desired_width INFINITY), which would otherwise
            // push a trailing button off the right edge.
            if ui.button("📂 Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Video/Audio", &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts", "mp3", "wav", "m4a", "aac", "opus", "flac"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                {
                    self.cv_input = p.to_string_lossy().to_string();
                    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if let Some(parent) = p.parent() {
                        self.cv_output = parent.join(format!("{stem}_converted.mp4")).to_string_lossy().to_string();
                    }
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.cv_input).desired_width(f32::INFINITY));
        });
        ui.horizontal(|ui| {
            ui.label("Output:");
            if ui.button("💾 Save as…").clicked() {
                let mut dlg = rfd::FileDialog::new().add_filter("Video/Audio", &["mp4", "mkv", "mov", "webm", "mp3", "wav"]);
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
            egui::Grid::new("cv_opts").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Category");
                if combo(ui, "cv_cat", &mut self.cv_category, CATEGORY_OPTIONS) {
                    let opts = codec_options(&self.cv_category);
                    self.cv_codec = opts[0].0.into();
                }
                ui.end_row();
                ui.label("Codec");
                combo(ui, "cv_codec", &mut self.cv_codec, codec_options(&self.cv_category));
                ui.end_row();
                ui.label("Hardware");
                combo(ui, "cv_hw", &mut self.cv_hw, HW_OPTIONS);
                ui.end_row();
                ui.label("Bitrate Mode");
                combo(ui, "cv_brmode", &mut self.cv_bitrate_mode, &[("crf", "CRF / CQ"), ("custom", "Custom Bitrate")]);
                ui.end_row();
            });

            let audio_only = util::convert_audio_ext(&self.cv_codec).is_some();
            let hint = codec_hint(&self.cv_codec, audio_only);
            if !hint.is_empty() {
                ui.label(RichText::new(hint).color(Color32::from_rgb(255, 183, 77)).size(11.0));
            }

            if !audio_only {
                if self.cv_bitrate_mode == "custom" {
                    ui.add(egui::Slider::new(&mut self.cv_custom_br, 2.0..=200.0).text("Mbps").integer());
                    ui.horizontal(|ui| {
                        for b in [8.0, 20.0, 50.0, 100.0] {
                            if ui.button(format!("{}M", b as i32)).clicked() {
                                self.cv_custom_br = b;
                            }
                        }
                    });
                } else {
                    ui.add(egui::Slider::new(&mut self.cv_crf, 15.0..=30.0).text("CRF").integer());
                }
            }
            ui.checkbox(&mut self.cv_preserve_color, "Preserve Color Metadata (BT.709 / BT.2020)");
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.cv_busy, egui::Button::new("▶ Convert")).clicked() {
                self.spawn_convert();
            }
            if ui.add_enabled(self.cv_busy, egui::Button::new("⏹ Stop")).clicked() {
                self.stop_convert();
            }
            if ui.button("Clear Log").clicked() {
                self.cv_log.clear();
            }
        });
        if let Some(p) = self.cv_progress {
            ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        }
        ui.label(&self.cv_status);

        ui.add_space(6.0);
        ui.label(RichText::new("Live Log").strong());
        log_view(ui, "cv_log", &self.cv_log, 260.0);
    }

    fn ui_setup(&mut self, ui: &mut egui::Ui) {
        ui.heading("Setup");
        ui.label(RichText::new("Manage yt-dlp and FFmpeg").color(Color32::GRAY).size(12.0));
        ui.add_space(8.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            status_row(ui, "yt-dlp", self.status.ytdlp_ok, if self.status.ytdlp_ok { &self.status.ytdlp_version } else { "Not installed" });
            status_row(ui, "FFmpeg", self.status.ffmpeg_ok, if self.status.ffmpeg_ok { &self.status.ffmpeg_version } else { "Not installed" });
            status_row(ui, "Deno (JS Runtime)", self.status.deno_ok, if self.status.deno_ok { &self.status.deno_version } else { "Not installed" });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Local Bin Path:").strong());
                ui.label(RichText::new(self.bin.bin_dir.to_string_lossy()).size(11.0).color(Color32::GRAY));
            });
        });

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("⬇ Install Binaries").clicked() {
                self.install_binaries();
            }
            ui.label("Channel:");
            if combo(ui, "channel", &mut self.channel, CHANNEL_OPTIONS) {
                self.config.set_str("ytdlp_channel", &self.channel);
            }
            if ui.button("⟳ Update yt-dlp").clicked() {
                self.update_ytdlp();
            }
            if ui.button("Install Deno").clicked() {
                self.install_deno();
            }
        });
        ui.add_space(8.0);
        if !self.setup_log.is_empty() {
            ui.label(&self.setup_log);
        }
    }

    fn ui_info(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            ui.heading(APP_NAME);
            ui.label(RichText::new(format!("Version {VERSION}  ·  Rust / egui port")).color(Color32::GRAY));
            ui.add_space(12.0);
        });
        let features = [
            ("Download", "YouTube, TikTok, Instagram, X/Twitter and 1700+ more sites"),
            ("Anti-Bot", "Optional --impersonate (curl_cffi) against 403/Cloudflare"),
            ("Vegas Pro", "H.264/AAC preferred - directly compatible with Vegas Pro 23+"),
            ("Audio", "MP3 (CBR 320k), WAV/PCM or Opus"),
            ("Quality", "4K, 1440p, 1080p, 720p, 480p - with AV1 preference"),
            ("SponsorBlock", "Automatically remove or mark sponsors"),
            ("Convert", "H.264, H.265, AV1 (SVT-AV1), ProRes 422, DNxHR, Vegas Sync Fix, MP3/WAV"),
            ("Hardware", "NVIDIA NVENC, AMD AMF, Intel QSV, Auto-Detect"),
            ("HDR/Color", "Color metadata is preserved during conversion"),
            ("Integrity", "yt-dlp binaries verified against signed SHA2-256SUMS"),
        ];
        egui::Frame::group(ui.style()).show(ui, |ui| {
            for (title, desc) in features {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✔").color(Color32::from_rgb(129, 199, 132)));
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(desc).color(Color32::GRAY).size(12.0));
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
        egui::Window::new("File already exists")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("\"{}\" is already in the target folder.", convert::file_name(&target)));
                ui.label(RichText::new("Save under a different name, overwrite, or skip.").color(Color32::GRAY).size(12.0));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Save as:");
                    ui.add(egui::TextEdit::singleline(&mut self.conflict_name).desired_width(280.0));
                });
                if let Some(err) = &self.conflict_error {
                    ui.label(RichText::new(err).color(Color32::from_rgb(239, 154, 154)));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save as").clicked() {
                        let cleaned = util::sanitize_filename(&self.conflict_name);
                        let stem = util::strip_media_ext(&cleaned).trim().trim_end_matches(['.', ' ']).to_string();
                        if stem.is_empty() {
                            self.conflict_error = Some("Please enter a file name.".into());
                        } else {
                            let ext = target.extension().map(|e| e.to_string_lossy().to_string());
                            let new_name = match ext {
                                Some(e) => format!("{stem}.{e}"),
                                None => stem,
                            };
                            let new_path = target.with_file_name(new_name);
                            if new_path.exists() {
                                self.conflict_error = Some("A file with this name already exists.".into());
                            } else {
                                decision = Some(ConflictDecision::Rename(new_path));
                            }
                        }
                    }
                    if ui.button("Overwrite").clicked() {
                        decision = Some(ConflictDecision::Overwrite);
                    }
                    if ui.button("Skip").clicked() {
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
                    egui::Frame::none().fill(bg).inner_margin(8.0).rounding(6.0).show(ui, |ui| {
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
        egui::SidePanel::left("nav").resizable(false).default_width(150.0).show(ctx, |ui| {
            ui.add_space(10.0);
            self.ui_nav(ui);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                0 => self.ui_download(ui, ctx),
                1 => self.ui_convert(ui, ctx),
                2 => self.ui_setup(ui),
                _ => self.ui_info(ui),
            });
        });

        self.show_conflict_modal(ctx);
        self.show_toasts(ctx);

        if self.dl_busy || self.cv_busy {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}

// ---------------- free helpers ----------------

fn label_of(current: &str, options: &[(&str, &str)]) -> String {
    options
        .iter()
        .find(|(v, _)| *v == current)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| current.to_string())
}

fn combo(ui: &mut egui::Ui, id: &str, current: &mut String, options: &[(&str, &str)]) -> bool {
    let before = current.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(label_of(current, options))
        .show_ui(ui, |ui| {
            for (val, lbl) in options {
                ui.selectable_value(current, (*val).to_string(), *lbl);
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
    egui::Frame::none().fill(bg).inner_margin(egui::Margin::symmetric(8.0, 3.0)).rounding(8.0).show(ui, |ui| {
        let short: String = value.chars().take(16).collect();
        ui.label(RichText::new(format!("{name}: {short}")).size(11.0).color(Color32::WHITE));
    });
}

fn status_row(ui: &mut egui::Ui, label: &str, ok: bool, value: &str) {
    ui.horizontal(|ui| {
        let mark = if ok { "✔" } else { "✖" };
        let color = if ok { Color32::from_rgb(129, 199, 132) } else { Color32::from_rgb(239, 154, 154) };
        ui.label(RichText::new(mark).color(color));
        ui.label(RichText::new(label).strong());
        ui.label(RichText::new(value).color(Color32::GRAY));
    });
}

fn codec_hint(key: &str, audio_only: bool) -> &'static str {
    if audio_only {
        "Note: audio only - the video track is discarded."
    } else if key.contains("prores") || key.contains("dnxhr") || key == "vegas_fix" {
        "Note: this codec runs most stably on CPU."
    } else if key == "copy" {
        "Note: stream copy - no re-encoding."
    } else if key.contains("av1") {
        "Note: AV1 - very efficient, but slow on CPU."
    } else {
        ""
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

fn log_view(ui: &mut egui::Ui, id: &str, lines: &[String], height: f32) {
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
                .show(ui, |ui| {
                    for line in lines {
                        ui.label(RichText::new(line).monospace().size(11.0).color(log_color(line)));
                    }
                });
        });
}

fn with_available_right<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add).inner
}

fn progress_text(prefix: &str, written: u64, total: u64) -> String {
    let mb = written as f64 / (1024.0 * 1024.0);
    if total > 0 {
        let pct = written as f64 * 100.0 / total as f64;
        format!("{prefix} ... {pct:.0}% ({mb:.1} MiB)")
    } else {
        format!("{prefix} ... {mb:.1} MiB")
    }
}

fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok().map(|s| s.trim().to_string())
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
