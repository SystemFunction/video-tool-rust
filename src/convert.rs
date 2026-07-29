//! FFmpeg command building and the conversion worker (ported from _run_conversion).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::binaries::{no_window, Binaries};
use crate::emit::Emitter;
use crate::i18n::{t, tf, Lang};
use crate::types::Task;
use crate::util;

type ChildSlot = Arc<Mutex<Option<Child>>>;

pub struct CvCtx {
    pub bin: Arc<Binaries>,
    pub em: Emitter,
    pub cancel: Arc<AtomicBool>,
    pub child: ChildSlot,
    pub lang: Lang,
}

impl CvCtx {
    fn t(&self, key: &'static str) -> &'static str {
        t(self.lang, key)
    }
    fn tf(&self, key: &'static str, args: &[&str]) -> String {
        tf(self.lang, key, args)
    }
}

#[derive(Clone)]
pub struct ConvertParams {
    pub codec_key: String,
    pub hw_setting: String,
    pub crf: i32,
    pub use_custom: bool,
    pub custom_br: i32,
    pub preserve_color: bool,
}

pub struct Profile {
    pub codec_key: String,
    pub audio_only: bool,
    pub video_codec: String,
    pub audio_codec: String,
    pub audio_bitrate: Option<String>,
    pub preset: Option<String>,
    pub extra: Vec<String>,
    pub use_custom: bool,
    pub custom_br: i32,
    pub crf: i32,
    pub hw_resolved: String,
    pub hw_requested: String,
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn build_convert_args(
    bin: &Binaries,
    params: &ConvertParams,
    source_pix_fmt: Option<&str>,
) -> Profile {
    let codec_key = params.codec_key.clone();
    let hw_requested = params.hw_setting.clone();
    let hw = if hw_requested == "auto" {
        bin.detect_hw_encoder()
    } else {
        hw_requested.clone()
    };
    let crf = params.crf;
    let use_custom = params.use_custom;
    let custom_br = params.custom_br;
    let is_10bit = source_pix_fmt
        .map(|p| p.contains("10") || p.contains("p010"))
        .unwrap_or(false);

    let mut video_codec = "libx264".to_string();
    let mut audio_codec = "aac".to_string();
    let mut audio_bitrate: Option<String> = Some("192k".to_string());
    let mut preset: Option<String> = Some("medium".to_string());
    let mut extra: Vec<String> = Vec::new();
    let audio_only = util::convert_audio_ext(&codec_key).is_some();

    match codec_key.as_str() {
        "audio_mp3" => {
            audio_codec = "libmp3lame".into();
            audio_bitrate = Some("320k".into());
        }
        "audio_wav" => {
            audio_codec = "pcm_s16le".into();
            audio_bitrate = None;
        }
        "copy" => {
            video_codec = "copy".into();
            audio_codec = "copy".into();
        }
        "vp9" => {
            video_codec = "libvpx-vp9".into();
            extra = v(&["-crf", "30", "-b:v", "0", "-row-mt", "1"]);
        }
        "av1" => {
            if hw == "nvidia" {
                video_codec = "av1_nvenc".into();
                preset = Some("p4".into());
                extra = vec![
                    "-rc".into(), "vbr".into(), "-cq".into(), (crf + 3).to_string(),
                    "-b:v".into(), "0".into(), "-pix_fmt".into(),
                    if is_10bit { "p010le" } else { "yuv420p" }.into(),
                ];
            } else if hw == "intel" {
                video_codec = "av1_qsv".into();
                preset = Some("medium".into());
                extra = vec![
                    "-global_quality".into(), crf.to_string(),
                    "-pix_fmt".into(), "nv12".into(),
                ];
            } else {
                video_codec = "libsvtav1".into();
                preset = None;
                extra = vec![
                    "-preset".into(), "6".into(), "-crf".into(), crf.to_string(),
                    "-pix_fmt".into(),
                    if is_10bit { "yuv420p10le" } else { "yuv420p" }.into(),
                ];
            }
        }
        "h264_allintra" => {
            extra = vec![
                "-g".into(), "1".into(), "-bf".into(), "0".into(), "-crf".into(),
                crf.to_string(), "-profile:v".into(), "high".into(), "-pix_fmt".into(),
                "yuv420p".into(),
            ];
        }
        "h264_handbrake" => {
            extra = vec![
                "-crf".into(), crf.to_string(), "-profile:v".into(), "high".into(),
                "-pix_fmt".into(), "yuv420p".into(),
            ];
        }
        "vegas_fix" => {
            preset = Some("fast".into());
            extra = v(&[
                "-fps_mode", "cfr", "-r", "30", "-crf", "16", "-g", "1", "-bf", "0",
                "-pix_fmt", "yuv420p",
            ]);
            audio_codec = "pcm_s16le".into();
            audio_bitrate = None;
        }
        "prores422" => {
            video_codec = "prores_ks".into();
            extra = v(&["-profile:v", "2", "-pix_fmt", "yuv422p10le"]);
            audio_codec = "pcm_s16le".into();
            audio_bitrate = None;
        }
        "prores422hq" => {
            video_codec = "prores_ks".into();
            extra = v(&["-profile:v", "3", "-pix_fmt", "yuv422p10le"]);
            audio_codec = "pcm_s16le".into();
            audio_bitrate = None;
        }
        "dnxhr_hq" => {
            video_codec = "dnxhd".into();
            extra = v(&["-profile:v", "dnxhr_hq", "-pix_fmt", "yuv422p"]);
            audio_codec = "pcm_s16le".into();
            audio_bitrate = None;
        }
        "youtube" => {
            preset = Some("slow".into());
            let pix = if is_10bit { "yuv420p10le" } else { "yuv420p" };
            extra = vec![
                "-crf".into(), "18".into(), "-profile:v".into(),
                if is_10bit { "high10" } else { "high" }.into(),
                "-pix_fmt".into(), pix.into(),
            ];
            audio_bitrate = Some("320k".into());
        }
        "youtube_av1" => {
            if hw == "nvidia" {
                video_codec = "av1_nvenc".into();
                preset = Some("p5".into());
                extra = v(&["-rc", "vbr", "-cq", "20", "-b:v", "0", "-pix_fmt", "yuv420p"]);
            } else {
                video_codec = "libsvtav1".into();
                preset = None;
                extra = v(&["-preset", "5", "-crf", "30", "-pix_fmt", "yuv420p"]);
            }
            audio_bitrate = Some("320k".into());
        }
        "social" => {
            preset = Some("medium".into());
            extra = v(&["-crf", "20", "-profile:v", "main", "-pix_fmt", "yuv420p"]);
        }
        other => {
            let is_h265 = other == "h265";
            if hw == "nvidia" {
                video_codec = if is_h265 { "hevc_nvenc" } else { "h264_nvenc" }.into();
                preset = Some("p4".into());
                let pix = if is_h265 { "p010le" } else { "yuv420p" };
                extra = vec![
                    "-rc".into(), "vbr".into(), "-cq".into(), (crf + 3).to_string(),
                    "-b:v".into(), "0".into(), "-multipass".into(), "qres".into(),
                    "-rc-lookahead".into(), "32".into(), "-spatial-aq".into(), "1".into(),
                    "-temporal-aq".into(), "1".into(), "-bf".into(), "3".into(),
                    "-refs".into(), "3".into(), "-profile:v".into(),
                    if is_h265 { "main" } else { "high" }.into(),
                ];
                if is_h265 {
                    extra.extend(v(&["-tag:v", "hvc1"]));
                }
                extra.extend(vec!["-pix_fmt".into(), pix.into()]);
            } else if hw == "amd" {
                video_codec = if is_h265 { "hevc_amf" } else { "h264_amf" }.into();
                preset = Some("balanced".into());
                extra = vec![
                    "-qp_i".into(), crf.to_string(), "-qp_p".into(), crf.to_string(),
                    "-qp_b".into(), crf.to_string(), "-pix_fmt".into(), "yuv420p".into(),
                ];
            } else if hw == "intel" {
                video_codec = if is_h265 { "hevc_qsv" } else { "h264_qsv" }.into();
                preset = Some("medium".into());
                extra = vec![
                    "-global_quality".into(), crf.to_string(),
                    "-pix_fmt".into(), "nv12".into(),
                ];
            } else {
                video_codec = if is_h265 { "libx265" } else { "libx264" }.into();
                let pix = if is_h265 && is_10bit { "yuv420p10le" } else { "yuv420p" };
                extra = vec!["-crf".into(), crf.to_string(), "-pix_fmt".into(), pix.into()];
                if is_h265 {
                    extra.extend(v(&["-tag:v", "hvc1", "-x265-params", "log-level=error"]));
                }
            }
        }
    }

    // Custom bitrate override (video only).
    if use_custom
        && !audio_only
        && !matches!(video_codec.as_str(), "copy" | "prores_ks" | "dnxhd")
    {
        let remove = [
            "-crf", "-cq", "-global_quality", "-qp_i", "-qp_p", "-qp_b",
        ];
        let mut filtered: Vec<String> = Vec::new();
        let mut skip = false;
        for arg in &extra {
            if skip {
                skip = false;
                continue;
            }
            if remove.contains(&arg.as_str()) {
                skip = true;
                continue;
            }
            filtered.push(arg.clone());
        }
        filtered.extend(vec![
            "-b:v".into(), format!("{custom_br}M"),
            "-maxrate".into(), format!("{}M", (custom_br as f64 * 1.5) as i64),
            "-bufsize".into(), format!("{}M", custom_br * 2),
        ]);
        extra = filtered;
    }

    Profile {
        codec_key,
        audio_only,
        video_codec,
        audio_codec,
        audio_bitrate,
        preset,
        extra,
        use_custom,
        custom_br,
        crf,
        hw_resolved: hw,
        hw_requested,
    }
}

pub fn build_ffmpeg_cmd(
    ffmpeg: &str,
    input_file: &str,
    output_file: &str,
    profile: &Profile,
    color_meta: &BTreeMap<String, String>,
    preserve_color: bool,
) -> Vec<String> {
    if profile.audio_only {
        let mut cmd = v(&[
            ffmpeg, "-hide_banner", "-nostdin", "-y", "-progress", "pipe:1",
            "-stats_period", "0.5", "-nostats", "-i", input_file, "-vn", "-map", "0:a:0",
            "-map_metadata", "0", "-c:a",
        ]);
        cmd.push(profile.audio_codec.clone());
        if let Some(br) = &profile.audio_bitrate {
            cmd.push("-b:a".into());
            cmd.push(br.clone());
        }
        cmd.push(output_file.to_string());
        return cmd;
    }

    let mut cmd: Vec<String> = vec![
        ffmpeg.to_string(), "-hide_banner".into(), "-nostdin".into(), "-y".into(),
        "-progress".into(), "pipe:1".into(), "-stats_period".into(), "0.5".into(),
        "-nostats".into(),
        "-fflags".into(), "+genpts".into(), "-i".into(), input_file.to_string(),
        "-map".into(), "0:v:0".into(), "-map".into(), "0:a?".into(),
        "-map_metadata".into(), "0".into(), "-map_chapters".into(), "0".into(),
    ];

    if profile.video_codec != "copy" && !profile.extra.iter().any(|a| a == "-pix_fmt") {
        cmd.push("-pix_fmt".into());
        cmd.push("yuv420p".into());
    }

    cmd.push("-c:v".into());
    cmd.push(profile.video_codec.clone());

    if !matches!(profile.video_codec.as_str(), "copy" | "prores_ks" | "dnxhd") {
        if let Some(p) = &profile.preset {
            cmd.push("-preset".into());
            cmd.push(p.clone());
        }
    }

    cmd.extend(profile.extra.iter().cloned());

    if profile.video_codec != "copy" && !color_meta.is_empty() && preserve_color {
        for (k, flag) in [
            ("color_primaries", "-color_primaries"),
            ("color_trc", "-color_trc"),
            ("color_space", "-colorspace"),
            ("color_range", "-color_range"),
        ] {
            if let Some(val) = color_meta.get(k) {
                cmd.push(flag.into());
                cmd.push(val.clone());
            }
        }
    }

    if matches!(
        profile.video_codec.as_str(),
        "libx264" | "libx265" | "libvpx-vp9" | "libsvtav1"
    ) {
        cmd.push("-threads".into());
        cmd.push("0".into());
    }

    if profile.video_codec == "copy" {
        cmd.push("-c:a".into());
        cmd.push("copy".into());
    } else {
        cmd.push("-c:a".into());
        cmd.push(profile.audio_codec.clone());
        if let Some(br) = &profile.audio_bitrate {
            cmd.push("-b:a".into());
            cmd.push(br.clone());
        }
        let pcm = ["copy", "pcm_s16le", "pcm_s32le", "pcm_f32le", "pcm_s24le"];
        if !pcm.contains(&profile.audio_codec.as_str()) {
            cmd.extend(v(&["-ar", "48000", "-ac", "2"]));
        }
    }

    let out_lower = output_file.to_lowercase();
    if (out_lower.ends_with(".mp4") || out_lower.ends_with(".m4v") || out_lower.ends_with(".mov"))
        && profile.video_codec != "copy"
    {
        cmd.push("-movflags".into());
        cmd.push("+faststart".into());
    }

    cmd.push(output_file.to_string());
    cmd
}

fn ffprobe_value(bin: &Binaries, args: &[&str]) -> Option<String> {
    let out = bin.command(&bin.ffprobe_path()).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn probe_duration(bin: &Binaries, file: &str) -> Option<f64> {
    ffprobe_value(
        bin,
        &[
            "-v", "quiet", "-show_entries", "format=duration", "-of",
            "default=noprint_wrappers=1:nokey=1", file,
        ],
    )
    .and_then(|s| s.parse().ok())
}

fn probe_pix_fmt(bin: &Binaries, file: &str) -> Option<String> {
    ffprobe_value(
        bin,
        &[
            "-v", "quiet", "-select_streams", "v:0", "-show_entries",
            "stream=pix_fmt", "-of", "default=noprint_wrappers=1:nokey=1", file,
        ],
    )
}

fn probe_color_meta(bin: &Binaries, file: &str) -> BTreeMap<String, String> {
    let mut meta = BTreeMap::new();
    if let Some(out) = ffprobe_value(
        bin,
        &[
            "-v", "quiet", "-select_streams", "v:0", "-show_entries",
            "stream=color_primaries,color_trc,color_space,color_range", "-of",
            "default=noprint_wrappers=1", file,
        ],
    ) {
        for line in out.lines() {
            if let Some((k, val)) = line.split_once('=') {
                let val = val.trim();
                let lo = val.to_lowercase();
                if !val.is_empty() && !["unknown", "n/a", "und", "reserved"].contains(&lo.as_str()) {
                    meta.insert(k.trim().to_string(), val.to_string());
                }
            }
        }
    }
    meta
}

/// The conversion worker (runs on its own thread).
///
/// The panic guard mirrors the download worker: without it a panic here would
/// leave the UI permanently "busy", with Convert greyed out until restart.
pub fn run_conversion(ctx: CvCtx, input_file: String, output_file: String, params: ConvertParams) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_conversion_inner(&ctx, &input_file, &output_file, &params);
    }));
    if result.is_err() {
        ctx.em.log(Task::Convert, ctx.t("cvw.panic"));
        ctx.em.status(Task::Convert, ctx.t("cvw.failed"));
    }
    *util::lock(&ctx.child) = None;
    ctx.em.busy(Task::Convert, false);
}

fn run_conversion_inner(
    ctx: &CvCtx,
    input_file: &str,
    output_file: &str,
    params: &ConvertParams,
) {
    let ffmpeg = ctx.bin.ffmpeg_path();
    let source_pix = probe_pix_fmt(&ctx.bin, input_file);
    let color_meta = if params.preserve_color {
        probe_color_meta(&ctx.bin, input_file)
    } else {
        BTreeMap::new()
    };
    let profile = build_convert_args(&ctx.bin, params, source_pix.as_deref());
    let duration = probe_duration(&ctx.bin, input_file);

    let cmd = build_ffmpeg_cmd(&ffmpeg, input_file, output_file, &profile, &color_meta, params.preserve_color);

    if profile.audio_only {
        ctx.em.log(
            Task::Convert,
            ctx.tf("cvw.codec_audio", &[&profile.codec_key, &profile.audio_codec]),
        );
    } else {
        ctx.em.log(
            Task::Convert,
            ctx.tf("cvw.codec", &[&profile.codec_key, &profile.video_codec]),
        );
        if profile.hw_requested == "auto" {
            ctx.em
                .log(Task::Convert, ctx.tf("cvw.hw_auto", &[&profile.hw_resolved]));
        }
        let mode = if profile.use_custom {
            ctx.tf("cvw.mode_custom", &[&profile.custom_br.to_string()])
        } else {
            ctx.tf("cvw.mode_crf", &[&profile.crf.to_string()])
        };
        ctx.em.log(Task::Convert, ctx.tf("cvw.mode", &[&mode]));
    }
    if let Some(p) = &source_pix {
        ctx.em.log(Task::Convert, ctx.tf("cvw.source", &[p]));
    }
    if !color_meta.is_empty() {
        let s: Vec<String> = color_meta.iter().map(|(k, v)| format!("{k}={v}")).collect();
        ctx.em.log(Task::Convert, ctx.tf("cvw.color", &[&s.join(", ")]));
    }
    if let Some(d) = duration {
        ctx.em
            .log(Task::Convert, ctx.tf("cvw.duration", &[&format!("{d:.1}")]));
    }
    ctx.em.log(Task::Convert, "-".repeat(44));

    let mut command = Command::new(&cmd[0]);
    command.args(&cmd[1..]);
    no_window(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            ctx.em
                .log(Task::Convert, ctx.tf("common.error", &[&e.to_string()]));
            ctx.em.status(Task::Convert, ctx.t("cvw.failed"));
            return;
        }
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        ctx.em.status(Task::Convert, ctx.t("cvw.failed"));
        return;
    };
    *util::lock(&ctx.child) = Some(child);

    // ffmpeg logs to stderr; keep the last lines for error reporting.
    let em_err = ctx.em.clone();
    let err_handle = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let t = line.trim();
            if !t.is_empty() && !t.contains("Press [q]") {
                em_err.log(Task::Convert, t.to_string());
            }
        }
    });

    let mut progress: BTreeMap<String, String> = BTreeMap::new();
    let started = Instant::now();
    let mut last_line = String::new();

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let (key, val) = match line.split_once('=') {
            Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
            None => continue,
        };
        progress.insert(key.clone(), val.clone());

        if key == "progress" {
            let mut cur_sec = parse_out_time(&progress);
            let bitrate = progress.get("bitrate").cloned().unwrap_or("N/A".into());
            let speed = progress.get("speed").cloned().unwrap_or("N/A".into());
            let fps = progress.get("fps").cloned().unwrap_or("N/A".into());
            let frame = progress.get("frame").cloned().unwrap_or("?".into());
            let q_val = progress.get("stream_0_0_q").cloned().unwrap_or("?".into());
            let tot_size = progress.get("total_size").cloned().unwrap_or("0".into());
            let out_time = progress.get("out_time").cloned().unwrap_or("00:00:00.00".into());

            let size_text = match tot_size.parse::<i64>() {
                Ok(n) => format!("{} KiB", n / 1024),
                Err(_) => "0 KiB".to_string(),
            };
            let elapsed = util::format_elapsed(started.elapsed().as_secs_f64());
            let compact = format!(
                "frame={:>5} fps={} q={} size={:>9} time={} bitrate={} speed={} elapsed={}",
                frame, fps, q_val, size_text, out_time, bitrate, speed, elapsed
            );
            if compact != last_line {
                ctx.em.log(Task::Convert, compact.clone());
                last_line = compact;
            }

            if cur_sec.is_none() {
                cur_sec = util::parse_hms_to_seconds(&out_time);
            }
            match (cur_sec, duration) {
                (Some(cur), Some(dur)) if dur > 0.0 => {
                    let pct = ((cur / dur) as f32).clamp(0.0, 1.0);
                    let eta = util::parse_speed_value(&speed)
                        .map(|sv| util::format_eta(Some((dur - cur) / sv)))
                        .unwrap_or_else(|| "--:--".to_string());
                    ctx.em.progress(Task::Convert, pct);
                    ctx.em.status(
                        Task::Convert,
                        ctx.tf(
                            "cvw.converting_pct",
                            &[&format!("{:.1}", pct * 100.0), &bitrate, &speed, &fps, &eta],
                        ),
                    );
                }
                _ => {
                    ctx.em.status(
                        Task::Convert,
                        ctx.tf("cvw.converting_plain", &[&bitrate, &speed, &fps]),
                    );
                }
            }
            if val == "end" {
                break;
            }
        }
    }
    err_handle.join().ok();

    // Bound to a local so the guard is dropped before the blocking `wait()`.
    let child = util::lock(&ctx.child).take();
    let code = match child {
        Some(mut c) => c.wait().ok().and_then(|s| s.code()).unwrap_or(-1),
        None => -1,
    };

    if ctx.cancel.load(Ordering::SeqCst) {
        ctx.em.status(Task::Convert, ctx.t("status.cancelled"));
        ctx.em.log(Task::Convert, ctx.t("cvw.cancelled_log"));
    } else if code == 0 {
        ctx.em.progress(Task::Convert, 1.0);
        ctx.em.status(Task::Convert, ctx.t("cvw.completed"));
        ctx.em.log(Task::Convert, ctx.t("cvw.success_log"));
        ctx.em.toast(ctx.t("cvw.success_toast"), false);
    } else {
        ctx.em.status(Task::Convert, ctx.t("cvw.failed"));
        ctx.em
            .log(Task::Convert, ctx.tf("cvw.failed_log", &[&code.to_string()]));
        ctx.em.toast(ctx.t("cvw.failed"), true);
    }
}

fn parse_out_time(progress: &BTreeMap<String, String>) -> Option<f64> {
    if let Some(ot) = progress.get("out_time") {
        if let Some(s) = util::parse_hms_to_seconds(ot) {
            return Some(s);
        }
    }
    let raw = progress
        .get("out_time_us")
        .or_else(|| progress.get("out_time_ms"))?;
    let n: i64 = raw.parse().ok()?;
    Some(if n > 10_000_000 {
        n as f64 / 1_000_000.0
    } else {
        n as f64 / 1000.0
    })
}

pub fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}
