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

# The tool and the runtime are pinned by SHA-256. The runtime becomes the
# first bytes of every AppImage released, and runs on every user's machine,
# so it's held to what was reviewed, like the actions in build.yml are
# pinned by commit. A file whose hash differs is refused, even one already
# downloaded. appimagetool is a numbered release; type2-runtime only
# publishes `continuous`, so when upstream rebuilds it this stops the build
# until someone reviews the new runtime and updates RUNTIME_SHA256 here.
# Set APPIMAGETOOL or APPIMAGE_RUNTIME to use a file of your own instead.
APPIMAGETOOL_VERSION=1.9.1
case "$arch" in
x86_64)
    APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
    RUNTIME_SHA256=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46
    ;;
*)
    APPIMAGETOOL_SHA256=
    RUNTIME_SHA256=
    ;;
esac

# check FILE SHA256 WHAT: refuse FILE unless it has that hash.
check() {
    if [ -z "$2" ]; then
        echo "No pinned hash for $3 on $arch: set APPIMAGETOOL and APPIMAGE_RUNTIME to files you've checked" >&2
        exit 1
    fi
    got=$(sha256sum "$1" | cut -d' ' -f1)
    if [ "$got" != "$2" ]; then
        echo "$3: $1 has SHA-256 $got, not the pinned $2." >&2
        echo "If upstream published a new build, review it and update the hash in $0." >&2
        rm -f "$1"
        exit 1
    fi
}

tool=${APPIMAGETOOL:-}
if [ -z "$tool" ]; then
    tool="$out/appimagetool-$APPIMAGETOOL_VERSION-$arch.AppImage"
    [ -s "$tool" ] || fetch "$tool" \
        "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$arch.AppImage"
    check "$tool" "$APPIMAGETOOL_SHA256" "appimagetool $APPIMAGETOOL_VERSION"
    chmod 755 "$tool"
fi

# The runtime appimagetool would fetch itself, without retrying or
# checking: fetched here instead, checked, and handed to it.
runtime=${APPIMAGE_RUNTIME:-}
if [ -z "$runtime" ]; then
    runtime="$out/runtime-$arch"
    [ -s "$runtime" ] || fetch "$runtime" \
        "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-$arch"
    check "$runtime" "$RUNTIME_SHA256" "type2-runtime"
fi

# Extract-and-run: works without FUSE (containers, CI).
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tool" --no-appstream --runtime-file "$runtime" \
    "$appdir" "$out/is-alive-$arch.AppImage"
echo "-> $out/is-alive-$arch.AppImage"
