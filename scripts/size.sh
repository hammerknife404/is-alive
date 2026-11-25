#!/usr/bin/env bash
# What the release binary is made of (#146, #163): code by crate, the
# biggest functions, the binary's sections, and the files built into it.
# Run from the repository root, on Linux or in Git Bash on Windows (CI's
# `size` workflow runs it on both). Writes size-report.md and prints it.
#
# Needs cargo-bloat (`cargo install cargo-bloat --locked --version 0.12.1`)
# and llvm-tools (`rustup component add llvm-tools`).
#
# The build is the release profile with symbols kept (cargo-bloat needs
# them), so its file is bigger than the one shipped; the shipped size is
# measured from a stripped copy (Linux) or the .exe itself (Windows keeps
# symbols in the .pdb beside it).
set -euo pipefail

out=${1:-size-report.md}
export CARGO_PROFILE_RELEASE_STRIP=false
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    exe=target/release/is-alive.exe
    os=Windows
    # Symbols for cargo-bloat, in the .pdb (the .exe doesn't grow).
    export CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
    ;;
  *)
    exe=target/release/is-alive
    os=Linux
    ;;
esac

crates=$(cargo bloat --release --locked --crates -n 60 2>/dev/null)
functions=$(cargo bloat --release --locked -n 40 2>/dev/null)

host=$(rustc -vV | sed -n 's/^host: //p')
bin="$(rustc --print sysroot)/lib/rustlib/$host/bin"
shipped=$exe
if [[ $os == Linux ]]; then
  shipped=$(mktemp)
  "$bin/llvm-objcopy" --strip-all "$exe" "$shipped"
fi
bytes() { wc -c < "$1" | tr -d ' '; }
mb() { awk -v b="$1" 'BEGIN { printf "%.2f MB", b / 1048576 }'; }
sections=$("$bin/llvm-size" -A -d "$shipped" | awk 'NR > 2 && $1 != "Total" && $2 > 65536 { printf "| %s | %s |\n", $1, $2 }' | sort -t'|' -k3 -rn)

# Files built into the binary (include_bytes!/include_str! and build.rs's
# skills/ list). Their bytes sit in the data sections, which cargo-bloat's
# per-crate figures (code only) don't count.
assets=$(
  for f in assets/oui.txt.gz assets/icons.ttf assets/icon-256.png docs/manual/MANUAL.md FOLLOW-ALONG.md; do
    [[ -f $f ]] && printf '| `%s` | %s |\n' "$f" "$(bytes "$f")"
  done
  printf '| `skills/` (all files) | %s |\n' "$(find skills -type f -exec cat {} + | wc -c | tr -d ' ')"
)

{
  echo "# is-alive $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1): size on $os"
  echo
  echo "Shipped binary: **$(mb "$(bytes "$shipped")")** ($(bytes "$shipped") bytes). Crates in the build: $(cargo tree --locked -e normal --prefix none 2>/dev/null | sed 's/ (\*)$//' | sort -u | wc -l | tr -d ' ')."
  echo
  echo "## Code by crate"
  echo
  echo "cargo-bloat's share of the code section. With LTO, code inlined across crates counts where it ended up, so treat small differences loosely."
  echo
  echo '```'
  echo "$crates"
  echo '```'
  echo
  echo "## Biggest functions"
  echo
  echo '```'
  echo "$functions"
  echo '```'
  echo
  echo "## Sections over 64 KB (shipped binary)"
  echo
  echo "| Section | Bytes |"
  echo "|---|---|"
  echo "$sections"
  echo
  echo "## Files built in"
  echo
  echo "| File | Bytes |"
  echo "|---|---|"
  echo "$assets"
} > "$out"
cat "$out"
