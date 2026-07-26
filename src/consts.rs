//! Application constants (ported from the Python header).

pub const VERSION: &str = "0.1.0";
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

/// (key, label) codec choices grouped by category.
pub fn codec_options(category: &str) -> &'static [(&'static str, &'static str)] {
    match category {
        "standard" => &[
            ("h264", "H.264 (compatible)"),
            ("h265", "H.265 / HEVC"),
            ("av1", "AV1 (modern, small)"),
            ("vp9", "VP9 (Web)"),
            ("copy", "Copy stream"),
        ],
        "editing" => &[
            ("h264_allintra", "H.264 All-Intra"),
            ("h264_handbrake", "H.264 Editing"),
            ("vegas_fix", "Vegas Sync Fix"),
            ("prores422", "ProRes 422"),
            ("prores422hq", "ProRes 422 HQ"),
            ("dnxhr_hq", "DNxHR HQ"),
        ],
        "delivery" => &[
            ("youtube", "YouTube Export (H.264)"),
            ("youtube_av1", "YouTube Export (AV1)"),
            ("social", "Instagram / TikTok"),
            ("copy", "Copy stream"),
        ],
        "audio" => &[
            ("audio_mp3", "MP3 (320 kbps)"),
            ("audio_wav", "WAV (PCM 16-bit)"),
        ],
        _ => codec_options("editing"),
    }
}

/// (value, label) quality choices for the Download tab.
pub const QUALITY_OPTIONS: &[(&str, &str)] = &[
    ("best", "Best Quality (H.264)"),
    ("best_av1", "Best Quality (AV1 preferred)"),
    ("2160", "4K (2160p)"),
    ("1440", "1440p"),
    ("1080", "1080p"),
    ("720", "720p"),
    ("480", "480p"),
    ("audio_wav", "Audio Only (WAV - Vegas/NLE)"),
    ("audio", "Audio Only (MP3 320k)"),
    ("audio_opus", "Audio Only (Opus, small)"),
];

pub const CONFLICT_OPTIONS: &[(&str, &str)] = &[
    ("ask", "Ask me (choose the name)"),
    ("rename", "Auto-rename - Title (1).mp4"),
    ("overwrite", "Overwrite existing file"),
    ("skip", "Skip (keep existing file)"),
];

pub const COOKIES_OPTIONS: &[(&str, &str)] = &[
    ("none", "None (default)"),
    ("firefox", "Firefox (recommended on Windows)"),
    ("cookiefile", "Cookies File (cookies.txt)"),
    ("chrome", "Chrome (often blocked/Windows)"),
    ("edge", "Edge (often blocked/Windows)"),
    ("brave", "Brave (often blocked/Windows)"),
    ("safari", "Safari (macOS)"),
];

pub const CATEGORY_OPTIONS: &[(&str, &str)] = &[
    ("standard", "Standard"),
    ("editing", "Editing"),
    ("delivery", "Delivery"),
    ("audio", "Audio (MP3 / WAV)"),
];

pub const HW_OPTIONS: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("nvidia", "NVIDIA NVENC"),
    ("amd", "AMD AMF"),
    ("intel", "Intel QSV"),
    ("cpu", "CPU"),
];

pub const CHANNEL_OPTIONS: &[(&str, &str)] = &[
    ("stable", "Stable (recommended)"),
    ("nightly", "Nightly (latest fixes)"),
    ("master", "Master (bleeding edge)"),
];
