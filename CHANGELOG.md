# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-09-04

### Added

- **A download queue.** Links can be lined up with **➕ Add to queue** and are
  then worked off one after another, each with its own state in the list.
  Entries can be added while the queue is already running, removed before
  their turn comes, and cleared once they are done. **Stop** stops the whole
  list rather than just the transfer in flight, and the entry it interrupted
  goes back to waiting so a second **Start** picks up where it left off. With
  nothing queued the tab behaves exactly as before — the list only appears
  once there is something in it.
- **Playlists.** `--no-playlist` used to be wired in, so a playlist or channel
  URL only ever produced its first video. **Download the whole playlist** now
  fetches every entry into a folder named after the list, optionally narrowed
  to a range such as `1-10` or `3,5,7`. The up-front filename probe is skipped
  in this mode, because "the" target name only ever described the first entry.
- **A speed limit** in the advanced options, for a download that should not
  take the whole line.
- **Trim in the Convert tab.** Two timestamps ("1:30" or plain seconds) cut the
  section to convert out of the source. Both bounds are passed as input
  options, so the end stays a position in the *source* rather than an offset
  into the trimmed result, and progress counts against the section instead of
  the whole file. A trimmed file drops the source's chapter marks, which
  describe a timeline it no longer has.
- **A source summary in the Convert tab.** Choosing an input reads its length,
  resolution, frame rate, codecs and size with ffprobe — the tool that was
  already being asked for the pixel format. In custom-bitrate mode the summary
  also estimates how large the result will be.
- **A History tab.** Every finished download and conversion is recorded with
  its file, the URL it came from and the preset it ran with. Entries open the
  file or its folder, copy the path or the link, or load the URL straight back
  into the Download tab. The list is filterable, capped at 200 entries and
  stored in the same config file as everything else.
- **A light theme**, switchable from the ☀/🌙 button in the header or from
  **Appearance** in the Setup tab. The log colours have a second palette so
  they stay readable on a pale background.
- **Keyboard shortcuts**: `Ctrl`+`1`..`5` switch tabs, `Ctrl`+`Enter` starts
  the visible tab, `Esc` stops what is running. A dialog on screen owns the
  keyboard, so `Esc` there still just closes it.
- **Copy** and **Save** buttons above both live logs.

### Changed

- The window title carries the running job's progress, so a minimised window
  still says how far along it is.
- The tab list shows a mark next to a tab whose job is still running.
- The file a download produced is read off yt-dlp's own destination, merge and
  move lines, which is what makes "open this file" in the history possible.
- Spacing, corner rounding and the accent colour were pulled into one place so
  both themes are laid out identically.

## [0.1.5] - 2026-08-19

### Changed

- A YouTube transfer that is rejected mid-download (*"HTTP Error 403:
  Forbidden"*) now drops the web player clients on the very next attempt
  instead of the third. Re-extracting from the same clients hands back the same
  SABR formats, so that middle run was spending a retry on an attempt that had
  already been shown not to work. Plain network failures (500/502/503, reset
  connections, timeouts) still keep their formats and only fetch fresh links.
- Retries fetch the fragments one at a time. Eight parallel range requests on a
  URL the CDN is already unhappy with tend to earn another rejection; the first
  attempt keeps its eight.

## [0.1.4] - 2026-08-16

### Fixed

- WAV downloads no longer fail at the very end with *"Supported filetypes for
  thumbnail embedding are: mp3, mkv/mka, ogg/opus/flac, m4a/mp4/m4v/mov"*. A
  WAV file has nowhere to put cover art, so with **Embed thumbnail/metadata**
  switched on the last post-processing step aborted the run even though the
  audio had already been written. Thumbnail embedding is now skipped for WAV
  only; metadata and chapters are still written, and every other format keeps
  its cover art.
- YouTube downloads that died part-way through with *"unable to download video
  data: HTTP Error 403: Forbidden"* are retried automatically. yt-dlp's own
  retries reuse the media URL that just stopped working, so they cannot get
  past this; the download is now repeated with a freshly extracted URL, keeping
  what the interrupted attempt already wrote, and a third attempt leaves out
  the player clients whose links YouTube tends to cut off.

### Changed

- YouTube's format list is now assembled from yt-dlp's own client rotation
  first. `web_safari` used to be asked first even though YouTube serves that
  client over SABR without a PO token — most of its formats arrive without a
  usable link, and the few that do not are exactly the ones that stop mid
  transfer.
- The failure diagnosis in the log also reads yt-dlp's error output, not just
  its progress output, so the existing hints (Instagram bug, missing formats,
  missing JS runtime) actually appear — plus a new one for a rejected transfer.

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

[0.1.5]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.5
[0.1.4]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.4
[0.1.3]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.3
[0.1.2]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.2
[0.1.0]: https://github.com/SystemFunction/video-tool-rust/releases/tag/v0.1.0
