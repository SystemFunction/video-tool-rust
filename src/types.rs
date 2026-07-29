//! Shared message/state types used to talk between worker threads and the UI.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Task {
    Download,
    Convert,
}

/// Messages sent from worker threads to the UI thread.
pub enum UiMsg {
    Log(Task, String),
    /// Progress in 0.0..=1.0; a negative value hides the bar.
    Progress(Task, f32),
    Status(Task, String),
    Busy(Task, bool),
    Toast(String, bool),
    SetupLog(String),
    /// Guards the Setup tab while an install/update thread is running.
    SetupBusy(bool),
    Binary(BinaryStatus),
    Conflict(ConflictReq),
}

#[derive(Clone, Default)]
pub struct BinaryStatus {
    pub ytdlp_ok: bool,
    pub ytdlp_version: String,
    pub ffmpeg_ok: bool,
    pub ffmpeg_version: String,
    pub deno_ok: bool,
    pub deno_version: String,
    pub hw_backend: String,
    pub impersonate_ok: bool,
    pub js_runtime: String,
}

/// Options gathered from the Download tab, handed to the worker.
#[derive(Clone, Default)]
pub struct DownloadOpts {
    pub impersonate: bool,
    pub impersonate_available: bool,
    pub sponsorblock: bool,
    pub embed: bool,
    pub subs: bool,
    pub subs_lang: String,
    pub cookiefile: String,
    pub potoken: bool,
    pub potoken_url: String,
    pub plugins_dir: String,
    pub conflict: String,
}

/// A request from the download worker asking the UI to resolve a file collision.
pub struct ConflictReq {
    pub target: PathBuf,
    pub suggestion: String,
    pub reply: Sender<ConflictDecision>,
}

pub enum ConflictDecision {
    Overwrite,
    Rename(PathBuf),
    Skip,
}
