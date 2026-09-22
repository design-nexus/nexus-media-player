# Nexus Media Player

A compact Omarchy themed media player using Quickshell, Qt 6, and libmpv.

## Requirements

- Quickshell and a Wayland desktop
- Qt 6 Core, Gui, Quick, Qml, and OpenGL development packages
- libmpv and its development files
- CMake, a C++17 compiler, and pkg-config

On an Omarchy system, install missing packages with `omarchy pkg add quickshell qt6-base qt6-declarative mpv cmake pkgconf`.

## Build and run

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j
./run [file-or-URL]
```

`./run` builds automatically on its first launch. A relative file path is resolved from the directory where you invoke `run`. Drop a file onto the window, or use the top-right menu to open a file or enter a path or URL. Press Enter to open a typed source. Sources supported by your libmpv installation can be played.

## Install

To build and install the player for your user, including a desktop launcher:

One-line install from GitHub:

```sh
tmp=$(mktemp -d) && curl -fsSL https://github.com/design-nexus/nexus-media-player/archive/refs/heads/main.tar.gz | tar -xz -C "$tmp" && "$tmp/nexus-media-player-main/install.sh"; status=$?; rm -rf "$tmp"; exit "$status"
```

For a local checkout:

```sh
./install.sh
nexus-media-player [file-or-URL]
```

The script installs under `~/.local/share/nexus-media-player` and creates a
launcher at `~/.local/bin/nexus-media-player`. It does not require root.

## Controls

| Action | Control |
| --- | --- |
| Play or pause | Single click the video, Space, or play button |
| Seek | Seek bar, Left/Right arrows (five seconds), or horizontal two finger scroll over the video or seek bar |
| Volume and mute | Click the volume icon for a vertical slider and mute button, or press M |
| Fullscreen | Double click the video, F, the fullscreen icon, or Escape to leave fullscreen |

Fullscreen controls hide after three seconds and reappear when you move the pointer or use a shortcut. Arrow keys retain their normal editing behavior while the source field has focus. The player reads the active Omarchy `colors.toml` palette and fontconfig monospace font while running, so changing the Omarchy theme or monospace font updates the interface.

## Notes

The video is rendered through libmpv's OpenGL render API in a Qt Quick framebuffer item. `run` sets `QSG_RHI_BACKEND=opengl` for that renderer. Network streams require working network access and any protocol support required by your mpv build.

MIT licensed. See [LICENSE](LICENSE).
