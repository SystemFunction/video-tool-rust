//! Application constants (ported from the Python header).
//!
//! Dropdown tables are `(value, i18n-key)` pairs - the second element is
//! resolved through `crate::i18n` at draw time, never shown verbatim.

pub const VERSION: &str = "0.2.0";
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

/// Hosts whose video titles are built from the uploader's name ("Video by
/// someone"), which would end up in the file name. Downloads from these get a
/// neutral "video-1234" name instead.
pub const ANON_NAME_HOSTS: &[&str] = &["instagram.com"];

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

pub const HISTORY_FILTER_OPTIONS: &[(&str, &str)] = &[
    ("all", "histf.all"),
    ("download", "histf.download"),
    ("convert", "histf.convert"),
];

pub const THEME_OPTIONS: &[(&str, &str)] = &[
    ("dark", "theme.dark"),
    ("light", "theme.light"),
];

/// Most entries the history keeps. Old ones fall off the end.
pub const HISTORY_LIMIT: usize = 200;

/// Characters a `--playlist-items` range may consist of. Anything else is
/// dropped rather than handed to yt-dlp, which would read it as a new option.
pub const PLAYLIST_ITEMS_CHARS: &str = "0123456789,:-";

/// Accepted shape of a `--limit-rate` value, e.g. "2M", "500K", "1.5M".
pub fn is_rate_limit(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.len() > 12 {
        return false;
    }
    let (num, unit) = match t.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((i, _)) => t.split_at(i),
        None => (t, ""),
    };
    if !matches!(unit.to_ascii_lowercase().as_str(), "" | "k" | "m" | "g") {
        return false;
    }
    num.parse::<f64>().map(|v| v > 0.0).unwrap_or(false)
}

/// Keeps only the characters a playlist range may contain.
///
/// The leading separators go too: a value starting with "-" would reach
/// yt-dlp as an option rather than as the range it is meant to be.
pub fn clean_playlist_items(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| PLAYLIST_ITEMS_CHARS.contains(*c))
        .collect();
    kept.trim_start_matches([',', ':', '-']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_rate_reaches_yt_dlp() {
        for good in ["2M", "500K", "1.5M", "800", "3g"] {
            assert!(is_rate_limit(good), "{good}");
        }
        // Anything else would arrive as a bare argument and be read as an
        // option or a URL.
        for bad in ["", "  ", "fast", "-1M", "0", "2MB", "--rm -rf", "2 M"] {
            assert!(!is_rate_limit(bad), "{bad}");
        }
    }

    #[test]
    fn a_playlist_range_keeps_only_range_characters() {
        assert_eq!(clean_playlist_items("1-10"), "1-10");
        assert_eq!(clean_playlist_items("3,5,7"), "3,5,7");
        assert_eq!(clean_playlist_items("1:3:2"), "1:3:2");
        assert_eq!(clean_playlist_items(" 1 - 10 "), "1-10");
        assert_eq!(clean_playlist_items("1;rm -rf /"), "1-");
        // A value that would arrive as an option instead of a range.
        assert_eq!(clean_playlist_items("-f bestvideo"), "");
        assert_eq!(clean_playlist_items(",,2-4"), "2-4");
    }
}
