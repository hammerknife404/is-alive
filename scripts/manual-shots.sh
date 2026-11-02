#!/usr/bin/env bash
# Screenshots for the user manual (docs/manual/MANUAL.md).
#
#   scripts/manual-shots.sh                  # full releases only: writes docs/manual/img/
#   scripts/manual-shots.sh --out DIR        # any version: a preview in DIR
#   scripts/manual-shots.sh --here [--out DIR]
#                                            # no container: cargo, Xvfb and python3
#                                            # on this machine (CI)
#
# The manual's pictures change only with full releases (x.y.0): run this
# after the version bump, look at every picture, and commit docs/manual/img
# with the release. On a patch version (x.y.z, z > 0) it refuses to touch
# docs/manual/img; --out still renders a preview anywhere.
#
# The tour renders into <out>/.incoming first. Only when every scene's run
# worked (a failed one is tried once more) and every picture the manual
# links to was taken do they replace the ones in <out>; otherwise <out> is
# left as it was, the script fails, and .incoming holds the run to look at.
#
# It builds the release binary and runs the app's manual tour
# (IS_ALIVE_SHOTS_MANUAL, see src/shots.rs) under a virtual display, with a
# settings file and notes of its own. In the default container mode
# (podman, image $ISALIVE_IMAGE, default isalive-dev) the network is pasta
# with a made-up LAN (192.168.1.50/24, gateway 192.168.1.1) and a few TCP
# listeners on this "machine", so the pictures show no real network. Runs
# go only to 1.1.1.1, example.com and Cloudflare's speed test file (a 10 s
# download); nothing is swept or traced, and the public IP is never looked up.
#
# Environment: ISALIVE_IMAGE, ISALIVE_TARGET (cargo target dir, default
# ~/.cache/isalive-target), ISALIVE_CARGO (cargo home, default
# ~/.cache/isalive-cargo).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
here=false
out=""
while (($#)); do
    case $1 in
    --out)
        out=${2:?--out needs a directory}
        shift 2
        ;;
    --here)
        here=true
        shift
        ;;
    --inside) # the container's half (internal)
        here=inside
        shift
        ;;
    -h | --help)
        sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'
        exit 0
        ;;
    *)
        echo "manual-shots: unknown argument $1 (see --help)" >&2
        exit 2
        ;;
    esac
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n1)

# The pictures in docs/manual/img match a full release: x.y.0 only.
if [[ -z $out ]]; then
    if [[ ! $version =~ ^[0-9]+\.[0-9]+\.0$ ]]; then
        echo "manual-shots: Cargo.toml says $version, a patch or pre-release." >&2
        echo "The manual's screenshots are updated only for full releases (x.y.0)," >&2
        echo "so docs/manual/img is left alone. To preview: --out <dir>." >&2
        exit 1
    fi
    out="$root/docs/manual/img"
fi
mkdir -p "$out"
out=$(cd "$out" && pwd)

# The tour itself: build, then run the app's manual tour into $out.
tour() {
    local target=${CARGO_TARGET_DIR:-$root/target}
    cargo build --locked --release --manifest-path "$root/Cargo.toml"

    tmp=$(mktemp -d)
    trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$tmp"' EXIT

    # Settings and notes of its own: the defaults (Dark) at the default
    # window size. Settings shows the folder, so the container names it
    # like a technician's (MANUAL_SHOTS_HOME). The tour never looks up the
    # public IP or checks for updates, so Settings shows both off and the
    # footer agrees ("WAN not looked up").
    export XDG_CONFIG_HOME="${MANUAL_SHOTS_HOME:-$tmp/home}/.config"
    mkdir -p "$XDG_CONFIG_HOME/is-alive"
    printf 'theme=dark\nsize=1120x720\nwan_on_start=0\nupdates_on_start=0\n' \
        >"$XDG_CONFIG_HOME/is-alive/settings"
    cat >"$XDG_CONFIG_HOME/is-alive/notes.md" <<'EOF'
## Site visit: Harbor Dental, front office

**Reported:** phones drop calls in the afternoon ☎️

- [x] Full scan from the front desk PC
- [x] Speed test: bufferbloat **C** while the backup runs 🔥
- [ ] Enable SQM on the firewall, shaped to 90% of the line
- [ ] Re-test after 3 pm ⏰

> Front desk printer: ping OK, port 9100 closed.

Switch in the comms cupboard is `SW-FRONT-01`, uplink on port 24 ✅
EOF

    # Something to find on this machine's own address: a few services a
    # small office server might run (port numbers below 1024 need root or
    # a container; any that can't be opened are left out).
    python3 - <<'EOF' &
import socket, threading, time
socks = []
for port in (22, 80, 443, 445, 3389, 8080):
    try:
        s = socket.socket()
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("0.0.0.0", port))
        s.listen(16)
        socks.append(s)
    except OSError:
        pass
def serve(s):
    while True:
        c, _ = s.accept()
        c.close()
for s in socks:
    threading.Thread(target=serve, args=(s,), daemon=True).start()
time.sleep(900)
EOF

    # A screen much bigger than the window, which opens at its top left:
    # the pointer starts in the middle of the screen, off the window, so no
    # picture has a hover in it.
    if [[ -z ${DISPLAY:-} ]]; then
        Xvfb :97 -screen 0 2400x1600x24 -nolisten tcp &
        export DISPLAY=:97
        sleep 2
    fi
    # Into a folder of its own first: a failed run never replaces a good set.
    local stage="$out/.incoming" want n ok=true
    want=$(grep -o '](img/[a-z-]*\.png)' "$root/docs/manual/MANUAL.md" | sort -u | wc -l)
    rm -rf "$stage"
    mkdir -p "$stage"
    echo "manual-shots: is-alive $version, pictures to $out"
    LIBGL_ALWAYS_SOFTWARE=1 IS_ALIVE_SHOTS="$stage" IS_ALIVE_SHOTS_MANUAL=1 \
        timeout 900 "$target/release/is-alive" || ok=false
    n=$(find "$stage" -maxdepth 1 -name '*.png' | wc -l)
    if [[ $ok != true || $n -ne $want ]]; then
        echo "manual-shots: the tour failed ($n of $want pictures; why is above)." >&2
        echo "$out is left as it was; this run's pictures are in ${stage#"$root"/}" >&2
        echo "(delete it when you've looked)." >&2
        return 1
    fi
    mv -f "$stage"/*.png "$out"/
    rmdir "$stage"
    echo "manual-shots: $n pictures in $out"
}

case $here in
inside | true)
    tour
    exit
    ;;
esac

# Container mode: the same script, inside, on a made-up network.
image=${ISALIVE_IMAGE:-isalive-dev}
target=${ISALIVE_TARGET:-$HOME/.cache/isalive-target}
cargo_home=${ISALIVE_CARGO:-$HOME/.cache/isalive-cargo}
mkdir -p "$target" "$cargo_home"
inner_out=/out
mounts=(-v "$root:/src:Z" -v "$target:/t:Z" -v "$cargo_home:/c:z")
case $out/ in
"$root"/*) inner_out="/src/${out#"$root"/}" ;;
*) mounts+=(-v "$out:/out:Z") ;;
esac
# The container's root is this user outside it (rootless podman), so the
# files it writes are yours; inside, root may ping and listen on low ports.
exec podman run --rm \
    --network 'pasta:-a,192.168.1.50,-n,24,-g,192.168.1.1,--no-map-gw' \
    -e HOME=/tmp -e CARGO_TARGET_DIR=/t -e CARGO_HOME=/c -e MANUAL_SHOTS_HOME=/home/tech \
    "${mounts[@]}" -w /src "$image" \
    bash scripts/manual-shots.sh --inside --out "$inner_out"
