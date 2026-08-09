# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-08-09

### Added

- Neutral file names for Instagram downloads. Instagram titles are built from
  the uploader's account name, which ended up in the saved file name. Downloads
  from `instagram.com` are now saved as `video-<1-9999>.mp4` with the number
  drawn at random and checked against the target folder, so an existing file is
  never overwritten.
- **Re-check** button in the Setup tab, which re-reads the yt-dlp / FFmpeg /
  JS-runtime versions on demand.
- The update check now retries transient GitHub failures (timeouts, 5xx) twice
  before reporting an error, and its message no longer repeats the API URL.

### Changed

- Startup is roughly 3 seconds faster. The version probe used to run yt-dlp and
  FFmpeg one after another (~3.6 s in total, during which the Download button
  stayed disabled). The probes now run in parallel, and their result is cached
  against the size and modification time of the binaries — so every start that
  recognises the files it probed last time skips the probe entirely. The cache
  is invalidated automatically when a binary is installed or updated.
- Instagram downloads start faster: because the random name is already known to
  be free, the extra yt-dlp run that resolves the target file name up front is
  skipped.

## [0.1.2] - 2026-07-30

### Added

- **In-app updater.** The app checks this repository for a newer release on
  start (can be turned off) and on demand from the Setup tab, shows what's new,
  and installs it in place. Downloads are verified against the SHA-256 digest
  and file size GitHub reports for the asset; without a valid digest nothing is
  installed and the app only offers to open the release page. During the swap
  the running executable is moved aside to `video_tool.exe.old`, rolled back if
  anything fails, and cleaned up on the next start. Individual versions can be
  skipped.
- English / German / French user interface with English as the default,
  switchable at any time from the header and remembered in the config. *(From
  the unreleased 0.1.1, first shipped here.)*

### Fixed

- The folder button under Save Location was pushed out of its row by the
  adjacent text field and could not be clicked. *(0.1.1)*

### Security

- yt-dlp downloads now fail closed: an unreachable `SHA2-256SUMS` or a missing
  entry previously installed the binary unverified. *(0.1.1)*
- The URL scheme is re-checked after redirects, so a redirect to `http://`
  can no longer downgrade the transport for an executable that is then run.
  *(0.1.1)*
- Windows device names are escaped in file names, and truncation happens before
  trimming so a cut cannot leave a trailing dot or space. *(0.1.1)*

### Changed

- Robustness and performance work from 0.1.1: recovery from mutex poisoning,
  panic guards around the conversion worker, a responsive Stop button, coalesced
  config writes, and a virtualised log view.

## [0.1.0] - 2026-07-26

### Added

- First public release of the Rust/egui port: a single, self-contained Windows
  executable that downloads videos with yt-dlp and converts them with FFmpeg,
  with no Python or runtime required.
- Download tab with quality presets, cookies, existing-file handling,
  SponsorBlock, thumbnail/metadata/subtitle embedding, impersonation and a live
  progress log.
- Convert tab with the full codec matrix (H.264/H.265/AV1/VP9, ProRes, DNxHR,
  Vegas Sync Fix, YouTube/social presets, MP3/WAV), hardware encoder selection,
  CRF or custom bitrate and HDR-aware color preservation.
- Setup tab that installs and updates yt-dlp (stable/nightly/master), FFmpeg and
  Deno into `~/.video_tool_v3/`, shared with the Python version of the app.

[0.1.3]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.3
[0.1.2]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.2
[0.1.0]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.0
