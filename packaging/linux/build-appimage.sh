#!/usr/bin/env bash
# Build is-alive-<arch>.AppImage from a release build.
#   packaging/linux/build-appimage.sh            # builds, then packages
#   APPIMAGETOOL=/path/to/appimagetool ...       # use a local appimagetool
# The binary links only glibc; GL, X11 and Wayland are loaded at runtime from
# the host. libxkbcommon-x11 (and its libxcb-xkb dependency) is bundled: winit
# needs it on X11 sessions and stock Ubuntu desktops don't ship it.
# Build on the oldest glibc you want to support.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/packaging/linux"
arch=${ARCH:-$(uname -m)}
out="$root/target/appimage"
appdir="$out/is-alive.AppDir"

cargo build --locked --release --manifest-path "$root/Cargo.toml"

rm -rf "$appdir"
install -Dm755 "$root/target/release/is-alive" "$appdir/usr/bin/is-alive"
install -Dm644 "$here/is-alive.desktop" "$appdir/usr/share/applications/is-alive.desktop"
install -Dm644 "$here/is-alive.svg" "$appdir/usr/share/icons/hicolor/scalable/apps/is-alive.svg"
install -Dm644 "$root/assets/icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/is-alive.png"
ln -s usr/share/applications/is-alive.desktop "$appdir/is-alive.desktop"
# appimagetool wants a PNG for the top-level icon and .DirIcon (it rejects SVG).
ln -s usr/share/icons/hicolor/256x256/apps/is-alive.png "$appdir/is-alive.png"
ln -s is-alive.png "$appdir/.DirIcon"

for lib in libxkbcommon-x11.so.0 libxcb-xkb.so.1; do
    src=$(ldconfig -p | awk -v l="$lib" -v a="$arch" '$1 == l && $NF ~ a { print $NF; exit }')
    if [ -z "$src" ]; then
        echo "error: $lib not found; install libxkbcommon-x11-0 and libxcb-xkb1" >&2
        exit 1
    fi
    install -Dm644 "$(readlink -f "$src")" "$appdir/usr/lib/$lib"
done

cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/is-alive" "$@"
RUN
chmod 755 "$appdir/AppRun"

# GitHub's release downloads fail now and then (a 504 for minutes at a
# time): try again, waiting twice as long each time (1 s, 2 s … 128 s:
# about four minutes in all) rather than fail the build.
fetch() {
    curl -fsSL --retry 8 --retry-all-errors --retry-max-time 600 -o "$1.part" "$2"
    mv "$1.part" "$1"
}

tool=${APPIMAGETOOL:-}
if [ -z "$tool" ]; then
    tool="$out/appimagetool-$arch.AppImage"
    if [ ! -x "$tool" ]; then
        fetch "$tool" \
            "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
        chmod 755 "$tool"
    fi
fi

# The runtime appimagetool would fetch itself, without retrying: fetched
# here instead, and handed to it.
runtime=${APPIMAGE_RUNTIME:-$out/runtime-$arch}
if [ ! -s "$runtime" ]; then
    fetch "$runtime" \
        "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-$arch"
fi

# Extract-and-run: works without FUSE (containers, CI).
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tool" --no-appstream --runtime-file "$runtime" \
    "$appdir" "$out/is-alive-$arch.AppImage"
echo "-> $out/is-alive-$arch.AppImage"
