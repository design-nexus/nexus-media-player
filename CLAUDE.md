# Nexus Media Player — notes for working on this repo

- The command is `media-player`; everything else (crate, config/data/cache folders,
  MPRIS name) is `nexus-media-player`.

- GTK4 (gtk4-rs 0.11) + Rust + libmpv (hand-written FFI, `player/mpv.rs`, links
  `libmpv.so.2`). No libadwaita. The window is one flat, monospace surface split by
  hairlines: a top bar (sidebar toggle, `Media Player / <page>`, search, settings,
  close), the sidebar, the page, the player bar and a status bar (`F1 Shortcuts` ·
  library size); fullscreen hides all of it. Pages have no title header. Settings is a
  card over the window (`settings_dialog.rs`) listing the settings page's groups;
  `navigate("settings")` opens it. The equalizer is a card too (`panel_dialog.rs`), from the top bar or `navigate("equalizer")`. Theme, window, widgets,
  stylesheet, queue, playlists and MPRIS started as copies of Nexus Music
  (`~/Projects/nexus-music`). Every colour is a `@theme_*` token; `@theme_video` (derived
  in `theme::palette_css`) is the dark surround of the picture.
- mpv: `Mpv::new` sets `LC_NUMERIC=C` first (mpv refuses other numeric locales). The
  wakeup callback only pokes a channel; events are drained on the GTK main loop, so all
  player state lives on the UI thread (`player::mod`). `keep-open=yes`: the end of a
  file is `eof-reached`, not END_FILE.
- Video (`player/video.rs`): mpv's render API draws into GL textures we own, in a
  surfaceless `GdkGLContext` from the display, handed to GTK as `GdkGLTexture`s through
  one shared `VideoPaintable`. Released textures come back through the release func and
  are reused. The render context must be freed (`video::shutdown`) before mpv. Frames
  are drawn at mpv's `dwidth`×`dheight`; GTK scales them.
- `player::attach` runs on window realize: it creates the render context (before any
  file loads, or mpv disables video) and then restores the queue.
- Equalizer: an `af` chain `@eq:lavfi=[equalizer@b0…b9,volume@pre]`, adjusted live with
  `af-command eq g <dB> equalizer@bN` so faders never interrupt sound.
- Library: `scan.rs` walks the roots, diffs (mtime, size) against SQLite, and per file
  runs `parse` (names from paths) → `nfo` → `probe` (ffprobe) → pictures beside it; then
  grabs frames for videos with no picture (ffmpeg), then TMDB if a key is set. Watch
  state is its own table keyed by path, so rescans never lose it. `.nfo` beats TMDB;
  local posters beat TMDB posters.
- Seek previews (`player/scrub.rs`): one ffmpeg contact sheet per video (keyframes
  only, ≤100 cells), cut up by `FramePaintable`.
- Widgets subscribe with `player::subscribe(&widget, …)` and `store::subscribe`. Never
  hold a `with(...)` borrow while calling `emit`.
- Checks: `cargo clippy --all-targets -- -D warnings`, `cargo test` (one test starts
  libmpv). Layout check: `NMP_SNAPSHOT=/tmp/x.png media-player --section movies`
  (quit any running instance first; it's single-instance). Snapshots render offscreen,
  which GL textures don't survive, so they skip video; check video in a real window
  (`grim -g` on the window's geometry from `hyprctl clients -j`). For tests, point
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` at scratch dirs with
  `library_folders` set (an unreadable settings file falls back to `~/Videos`), and use
  `NMP_AO=null` to play silently. `mode = "theme"`, `theme = "catppuccin-latte"` checks
  light mode.
