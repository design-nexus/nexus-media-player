# Nexus Media Player

A video player for [Omarchy](https://omarchy.org). It keeps your movies and TV shows in
a library, remembers where you stopped, and plays them with mpv. It takes its colours
from your Omarchy theme and fits a half-screen tile.

## What it does

- **Library:** scans your video folders (MKV, MP4, WebM, AVI, MOV, WMV, TS, MPEG, OGV and
  more) into a local database. Later scans only read new or changed files, and it picks
  up videos added, removed or renamed within a minute.
  - **Home:** Continue watching, the next episode of each show you're following, and
    what was added lately.
  - **Movies:** a poster grid, sorted by title, year, date added or rating, with
    watched and unwatched filters. Each movie has a page with its details and file info.
  - **TV shows:** shows, seasons and episodes, found from file names
    (`Show S01E02.mkv`, `Show 1x02.mkv`) or folders (`Show/Season 1/02 - Title.mkv`).
  - **All videos** in one sortable table, and **Folders** as they're laid out on disk.
  - **Search** (<kbd>Ctrl</kbd>+<kbd>F</kbd>) across titles, shows, episodes and plots.
- **Details and pictures:** names from file and folder names, then Kodi-style `.nfo`
  files (`movie.nfo`, `tvshow.nfo`, `<name>.nfo`), then optionally
  [TMDB](https://www.themoviedb.org) with your own free API key. Posters and thumbnails
  come from images beside the videos (`poster.jpg`, `<name>-poster.jpg`,
  `<name>-thumb.jpg`), from TMDB, or a frame grabbed from the video. **Fix match**
  picks the right TMDB entry when the guess is wrong.
- **Playback** with mpv: hardware decoding, every format mpv plays, and speed from
  0.25× to 3× with the pitch kept.
  - **Resume:** each video starts where it was stopped and is marked watched near the
    end. The next episode starts when one ends.
  - **Seeking:** <kbd>←</kbd>/<kbd>→</kbd>, two-finger swipes on a touchpad, and a
    seek bar that previews the frame under the pointer.
  - **Subtitles:** picked up from files beside the video (`.srt`, `.ass`, `.vtt`…) or
    loaded by hand, with styled ASS, a delay you can nudge, and a size setting. Each
    video remembers its subtitle and sound tracks.
  - Fullscreen, frame stepping, and controls that fade away while the video plays.
- **Queue and playlists:** play next or add to the queue from any video's right-click
  menu, make playlists, drag videos onto them in the sidebar, and import or export M3U.
- **Equalizer:** ten bands and a preamp, with presets and your own saved ones.
- **Media keys and the bar:** MPRIS, so media keys, `playerctl` and status bars control
  it and show what's playing.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nexus-media-player/main/install.sh | bash
```

This installs GTK 4, mpv and ffmpeg if they're missing, builds with Cargo, and installs
`media-player` to `~/.local/bin` along with a launcher entry. Add `-s -- --default`
after `bash` to also make it the default app for video files.

To remove it, run the same line with `uninstall.sh` in place of `install.sh`. Add
`-s -- --purge` to also remove its settings, library database and playlists. Your video
files are never touched.

## Usage

```
media-player [OPTIONS] [FILES…]
```

| Option | What |
| --- | --- |
| `FILES…` | Play these videos, folders or `.m3u` playlists |
| `--enqueue` | Add `FILES` to the queue instead |
| `--fullscreen` | Play `FILES` in fullscreen |
| `--section ID` | Open a page: `home`, `movies`, `tv`, `videos`, `folders`, `now-playing`, `queue`, `equalizer`, `settings`; or a show (`tv:NAME`) or a movie (`movie:PATH`) |
| `--toggle` | Close the window if it's open, otherwise open it |
| `--play-pause`, `--play`, `--pause`, `--stop`, `--next`, `--previous` | Control playback without raising the window |

| Key | What |
| --- | --- |
| <kbd>Space</kbd> | Play or pause |
| <kbd>←</kbd> / <kbd>→</kbd> | Back / forward 5 seconds (<kbd>Shift</kbd>: 30 seconds) |
| <kbd>Ctrl</kbd>+<kbd>←</kbd> / <kbd>→</kbd> | Previous / next video |
| <kbd>↑</kbd> / <kbd>↓</kbd> | Louder / quieter |
| <kbd>F</kbd>, <kbd>Esc</kbd> | Fullscreen, leave fullscreen |
| <kbd>M</kbd> | Mute |
| <kbd>S</kbd> / <kbd>A</kbd> | Next subtitles / next sound track |
| <kbd>Z</kbd> / <kbd>X</kbd> | Subtitles earlier / later |
| <kbd>[</kbd> / <kbd>]</kbd> | Slower / faster |
| <kbd>,</kbd> / <kbd>.</kbd> | Back / forward one frame |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Open files |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search the library |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Close |

On a touchpad, swipe sideways over the video to seek and up or down to change the
volume. Click to pause; double-click for fullscreen.

## Files

| Path | What |
| --- | --- |
| `~/.config/nexus-media-player/settings.toml` | Preferences: library folders, playback, subtitles, TMDB, equalizer, theme |
| `~/.config/nexus-media-player/themes/*.toml` | Your own themes |
| `~/.local/share/nexus-media-player/library.db` | The library, watch progress, playlists and TMDB answers |
| `~/.local/share/nexus-media-player/state.json` | The queue, for resuming |
| `~/.cache/nexus-media-player/art/` | Posters and frames, scaled |
| `~/.cache/nexus-media-player/scrub/` | Seek bar previews |

This product uses the TMDB API but is not endorsed or certified by TMDB.
