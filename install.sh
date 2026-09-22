#!/usr/bin/env bash
set -euo pipefail

source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/nexus-media-player"
bin_dir="$HOME/.local/bin"
desktop_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

for program in cmake pkg-config quickshell; do
    command -v "$program" >/dev/null || { echo "Missing dependency: $program" >&2; exit 1; }
done
pkg-config --exists mpv Qt6Quick || {
    echo "Missing development packages: mpv and Qt6Quick (pkg-config)." >&2
    exit 1
}

cmake -S "$source_dir" -B "$source_dir/build" -DCMAKE_BUILD_TYPE=Release
cmake --build "$source_dir/build" --parallel

mkdir -p "$install_dir/qml/Nexus/Mpv" "$install_dir/lib" "$bin_dir" "$desktop_dir"
install -m 644 "$source_dir/qml/shell.qml" "$install_dir/shell.qml"
install -m 755 "$source_dir/run" "$install_dir/run"
install -m 644 "$source_dir/build/qml/Nexus/Mpv/qmldir" "$install_dir/qml/Nexus/Mpv/qmldir"
install -m 755 "$source_dir/build/qml/Nexus/Mpv/libnexusmpvplugin.so" "$install_dir/qml/Nexus/Mpv/"
ln -sfn "$install_dir/run" "$bin_dir/nexus-media-player"

cat > "$desktop_dir/nexus-media-player.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Nexus Media Player
Comment=Omarchy themed mpv player
Exec=$bin_dir/nexus-media-player %u
Icon=multimedia-video-player
Terminal=false
Categories=AudioVideo;Video;Player;
EOF

echo "Installed Nexus Media Player to $install_dir"
echo "Run: $bin_dir/nexus-media-player [file-or-URL]"
