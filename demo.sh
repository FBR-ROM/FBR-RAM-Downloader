#!/usr/bin/env bash
# One-command demo: builds the release binary if needed, then runs the TUI.
#   ./demo.sh            # interactive preview (press q to quit)
#   ./demo.sh --dump     # headless: print frames to stdout
set -euo pipefail
cd "$(dirname "$0")"

BIN="target/release/fbr-ram-downloader"

# Build if the binary is missing or older than the sources.
needs_build=0
if [[ ! -x "$BIN" ]]; then
  needs_build=1
else
  # Rebuild if any source is newer than the binary.
  if [[ -n "$(find src Cargo.toml -newer "$BIN" 2>/dev/null | head -1)" ]]; then
    needs_build=1
  fi
fi

if [[ "$needs_build" == 1 ]]; then
  echo "building..." >&2
  if command -v cargo >/dev/null 2>&1; then
    cargo build --release
  else
    nix-shell -p rustc cargo --run "cargo build --release"
  fi
fi

# Music is embedded (rodio), so no external audio player is needed.
exec "$BIN" "$@"
