//! Application constants (ported from the Python header).
//!
//! Dropdown tables are `(value, i18n-key)` pairs - the second element is
//! resolved through `crate::i18n` at draw time, never shown verbatim.

pub const VERSION: &str = "0.1.1";
pub const APP_NAME: &str = "Video Tool";

/// First stable yt-dlp release with the reworked Instagram extractor.
pub const YTDLP_INSTAGRAM_FIX: &str = "2026.07.04";

/// Hosts that block yt-dlp's default TLS fingerprint - impersonation is
/// auto-enabled for these when curl_cffi support is available.
pub const IMPERSONATE_AUTO_HOSTS: &[&str] = &[
    "instagram.com",
    "tiktok.com",
    "twitter.com",
    "//x.com",
    "www.x.com",
    "facebook.com",
    "fb.watch",
];

/// (key, i18n-key) codec choices grouped by category.
pub fn codec_options(category: &str) -> &'static [(&'static str, &'static str)] {
    match category {
        "standard" => &[
            ("h264", "codec.h264"),
            ("h265", "codec.h265"),
            ("av1", "codec.av1"),
            ("vp9", "codec.vp9"),
            ("copy", "codec.copy"),
        ],
        "editing" => &[
            ("h264_allintra", "codec.h264_allintra"),
            ("h264_handbrake", "codec.h264_handbrake"),
            ("vegas_fix", "codec.vegas_fix"),
            ("prores422", "codec.prores422"),
            ("prores422hq", "codec.prores422hq"),
            ("dnxhr_hq", "codec.dnxhr_hq"),
        ],
        "delivery" => &[
            ("youtube", "codec.youtube"),
            ("youtube_av1", "codec.youtube_av1"),
            ("social", "codec.social"),
            ("copy", "codec.copy"),
        ],
        "audio" => &[
            ("audio_mp3", "codec.audio_mp3"),
            ("audio_wav", "codec.audio_wav"),
        ],
        _ => codec_options("editing"),
    }
}

/// (value, i18n-key) quality choices for the Download tab.
pub const QUALITY_OPTIONS: &[(&str, &str)] = &[
    ("best", "quality.best"),
    ("best_av1", "quality.best_av1"),
    ("2160", "quality.2160"),
    ("1440", "quality.1440"),
    ("1080", "quality.1080"),
    ("720", "quality.720"),
    ("480", "quality.480"),
    ("audio_wav", "quality.audio_wav"),
    ("audio", "quality.audio"),
    ("audio_opus", "quality.audio_opus"),
];

pub const CONFLICT_OPTIONS: &[(&str, &str)] = &[
    ("ask", "conflictopt.ask"),
    ("rename", "conflictopt.rename"),
    ("overwrite", "conflictopt.overwrite"),
    ("skip", "conflictopt.skip"),
];

pub const COOKIES_OPTIONS: &[(&str, &str)] = &[
    ("none", "cookies.none"),
    ("firefox", "cookies.firefox"),
    ("cookiefile", "cookies.cookiefile"),
    ("chrome", "cookies.chrome"),
    ("edge", "cookies.edge"),
    ("brave", "cookies.brave"),
    ("safari", "cookies.safari"),
];

pub const CATEGORY_OPTIONS: &[(&str, &str)] = &[
    ("standard", "cat.standard"),
    ("editing", "cat.editing"),
    ("delivery", "cat.delivery"),
    ("audio", "cat.audio"),
];

pub const HW_OPTIONS: &[(&str, &str)] = &[
    ("auto", "hw.auto"),
    ("nvidia", "hw.nvidia"),
    ("amd", "hw.amd"),
    ("intel", "hw.intel"),
    ("cpu", "hw.cpu"),
];

pub const BITRATE_MODE_OPTIONS: &[(&str, &str)] = &[
    ("crf", "brmode.crf"),
    ("custom", "brmode.custom"),
];

pub const CHANNEL_OPTIONS: &[(&str, &str)] = &[
    ("stable", "channel.stable"),
    ("nightly", "channel.nightly"),
    ("master", "channel.master"),
];
