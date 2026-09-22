#!/usr/bin/env bash
set -euo pipefail

for program in cmake pkg-config quickshell; do
    command -v "$program" >/dev/null || { echo "Missing dependency: $program" >&2; exit 1; }
done
pkg-config --exists mpv Qt6Quick || {
    echo "Missing development packages: mpv and Qt6Quick (pkg-config)." >&2
    exit 1
}

if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" && -f "$(dirname -- "${BASH_SOURCE[0]}")/CMakeLists.txt" ]]; then
    source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
else
    for program in curl tar; do
        command -v "$program" >/dev/null || { echo "Missing dependency: $program" >&2; exit 1; }
    done
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT
    curl -fsSL https://github.com/design-nexus/nexus-media-player/archive/refs/heads/main.tar.gz | tar -xz -C "$tmp_dir" --strip-components=1
    source_dir="$tmp_dir"
fi

install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/nexus-media-player"
bin_dir="$HOME/.local/bin"
desktop_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

cmake -S "$source_dir" -B "$source_dir/build" -DCMAKE_BUILD_TYPE=Release
cmake --build "$source_dir/build" --parallel

mkdir -p "$install_dir/qml/Nexus/Mpv" "$install_dir/lib" "$bin_dir" "$desktop_dir"
install -m 644 "$source_dir/qml/shell.qml" "$install_dir/shell.qml"
install -m 644 "$source_dir/qml/NexusButton.qml" "$install_dir/NexusButton.qml"
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
