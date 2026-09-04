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
    /// Outcome of an update check: Ok(Some) = newer release, Ok(None) = current.
    UpdateCheck(Result<Option<crate::update::Release>, String>, CheckKind),
    /// Progress line shown while an update downloads.
    UpdateStatus(String),
    /// Outcome of installing an update; Ok carries the new version string.
    UpdateInstalled(Result<String, String>),
    /// How a worker ended. Always sent before the matching `Busy(_, false)`,
    /// so the queue driver sees the outcome before it advances.
    Finished(Task, Outcome),
    /// Result of an ffprobe run against the Convert tab's input file.
    MediaInfo(Box<Option<MediaInfo>>),
}

/// How a worker run ended, as far as the UI cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Finished and wrote (or already had) a file; carries it when known.
    Success(Option<PathBuf>),
    /// Nothing was written because the user or the conflict rule said so.
    Skipped,
    Cancelled,
    Failed,
}

/// State of one entry in the download queue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueueState {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

/// What ffprobe could tell us about a file, for the Convert tab's summary.
#[derive(Clone, Default, Debug)]
pub struct MediaInfo {
    /// The file this describes - a late answer for a since-changed input is
    /// dropped rather than shown against the wrong file.
    pub path: String,
    pub duration: Option<f64>,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub vcodec: String,
    pub acodec: String,
    pub size_bytes: u64,
    pub bit_rate: Option<u64>,
}

/// Whether an update check was triggered by the user or ran on start.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Automatic,
    Manual,
}

/// Serializable so the last probe can be cached across starts (see
/// `Binaries::fingerprint`); unknown or missing fields fall back to defaults,
/// which simply invalidates nothing - a bad cache entry is dropped instead.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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
    /// Fetch every entry of a playlist/channel URL instead of just the video.
    pub playlist: bool,
    /// Optional `--playlist-items` range, already stripped to safe characters.
    pub playlist_items: String,
    /// Optional `--limit-rate` value ("2M"), empty for unlimited.
    pub rate_limit: String,
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
