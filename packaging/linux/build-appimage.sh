#!/usr/bin/env bash
# Build is-alive-<arch>.AppImage from a release build.
#   packaging/linux/build-appimage.sh            # builds, then packages
#   APPIMAGETOOL=/path/to/appimagetool ...       # use a local appimagetool
# The binary links only glibc (GL, X11, Wayland are loaded at runtime), so
# nothing else is bundled. Build on the oldest glibc you want to support.
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
ln -s usr/share/applications/is-alive.desktop "$appdir/is-alive.desktop"
ln -s usr/share/icons/hicolor/scalable/apps/is-alive.svg "$appdir/is-alive.svg"
ln -s is-alive.svg "$appdir/.DirIcon"
cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
exec "$(dirname "$(readlink -f "$0")")/usr/bin/is-alive" "$@"
RUN
chmod 755 "$appdir/AppRun"

tool=${APPIMAGETOOL:-}
if [ -z "$tool" ]; then
    tool="$out/appimagetool-$arch.AppImage"
    if [ ! -x "$tool" ]; then
        curl -fsSL -o "$tool" \
            "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
        chmod 755 "$tool"
    fi
fi

# Extract-and-run: works without FUSE (containers, CI).
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tool" --no-appstream "$appdir" "$out/is-alive-$arch.AppImage"
echo "-> $out/is-alive-$arch.AppImage"
