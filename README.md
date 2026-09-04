# Video Tool (Rust / egui)

A native Rust port of the original Flet/Python `video_tool.py`. It downloads
videos with **yt-dlp** and converts them with **FFmpeg**, wrapped in a native
[egui](https://github.com/emilk/egui) desktop GUI that compiles to a single,
self-contained `.exe` — no Python or runtime required.

The app manages its own copies of `yt-dlp`, `ffmpeg`/`ffprobe` and (optionally)
Deno in `~/.video_tool_v3/` — the **same folder the Python version uses**, so
binaries installed by either app are shared. yt-dlp downloads are verified
against the release-signed `SHA2-256SUMS` and discarded if the checksum cannot
be fetched or does not match; all downloads are HTTPS-only, including after
redirects.

## Download

Grab the latest Windows build from the
[**Releases**](https://github.com/SystemFunction/video-tool-rust/releases/latest)
page and run `video_tool.exe` — no installation needed. On first launch, open the
**Setup** tab to install yt-dlp and FFmpeg. From then on the app offers new
versions of itself on start and installs them for you.

See the [changelog](CHANGELOG.md) for what changed in each release.

## Features

- **Languages** — English (default), German and French, switchable at any time
  from the picker in the header. The choice is remembered in the config.
- **Themes** — dark or light, from the button in the header or the Setup tab.
- **Download tab** — quality presets (best / AV1 / 4K…480p / audio MP3·WAV·Opus),
  a **queue** that works any number of links off one after another, whole
  **playlists** (optionally only a range) into a folder of their own, cookies
  (browser or `cookies.txt`), existing-file handling (ask / auto-rename /
  overwrite / skip) with an up-front filename probe, a folder picker for the save
  location, and advanced options: impersonate (anti-bot), SponsorBlock,
  thumbnail/metadata/chapter embedding, subtitles, a speed limit, and
  PO-token / bgutil support. Live progress log. Instagram downloads are saved
  under a neutral `video-1234.mp4` name, since their title carries the
  uploader's account name.
- **Convert tab** — full codec matrix (H.264/H.265/AV1/VP9, ProRes, DNxHR,
  Vegas Sync Fix, YouTube/social delivery presets, MP3/WAV audio), hardware
  encoder selection (NVENC / AMF / QSV / auto-detect), CRF or custom bitrate,
  **trimming** to a section between two timestamps, an ffprobe summary of the
  source file (length, resolution, frame rate, codecs, size) with a size
  estimate for custom bitrates, HDR-aware color-metadata preservation, and live
  FFmpeg progress. Browse and Save-as file pickers for input/output.
- **History tab** — every finished download and conversion, with the file, the
  URL it came from and the preset it ran with. Open the file or its folder,
  copy the path or link, or load the URL back into the Download tab.
- **Setup tab** — install/update yt-dlp (Stable/Nightly/Master channels),
  install FFmpeg (via ffbinaries) and Deno, with status readout. The versions
  shown are cached against the binaries on disk so starting the app costs no
  process launches; **Re-check** re-reads them on demand.
- **Info tab** — feature overview.
- **Keyboard** — `Ctrl`+`1`..`5` switch tabs, `Ctrl`+`Enter` starts the visible
  tab, `Esc` stops what is running.
- Config and history persisted as JSON in `~/.video_tool_v3/config.json`.

## Build from source

Requires the Rust toolchain (1.75+). On Windows the executable icon is embedded
at build time via `build.rs` + `winresource` (using `windres` on the GNU
toolchain or `rc.exe` on MSVC).

```bash
cargo build --release
```

The finished binary lands at `target/release/video_tool.exe`.

Run directly during development:

```bash
cargo run
```

## Intentional differences from the Python version

- The Python app's **source self-update** (fetching and swapping `video_tool.py`
  from GitHub) is dropped — it makes no sense for a compiled binary. yt-dlp and
  FFmpeg can still be updated from the Setup tab.

## Project layout

```
assets/
  icon.ico      Windows executable / window icon
build.rs        embeds the icon into the .exe on Windows
src/
  main.rs       entry point + window setup
  app.rs        egui UI (all five tabs)
  binaries.rs   yt-dlp/ffmpeg/deno management + SHA-256 verification
  download.rs   yt-dlp command building + download worker
  convert.rs    FFmpeg command building + conversion worker
  history.rs    the finished-jobs list behind the History tab
  config.rs     JSON settings store
  consts.rs     app constants and option tables
  i18n.rs       English/German/French translation table
  emit.rs       worker -> UI message helper
  types.rs      shared message/state types
  util.rs       small pure helpers
```

## License

Released under the [MIT License](LICENSE).
