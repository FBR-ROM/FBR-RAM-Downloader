#!/usr/bin/env bash
# Self-contained demo launcher.
#
# Runs the cracktro using ONLY the libraries bundled in demo/lib — no system
# Rust, Nix, or ALSA required. The bundled glibc loader is invoked directly
# with --library-path, so the binary and its whole audio stack (glibc + ALSA)
# come from this directory. Music is bundled in demo/music and is found next
# to the executable, so this works from any working directory.
#
#   ./demo/run.sh            # interactive (press q to quit)
#   ./demo/run.sh --dump 60  # headless: print 60 frames to stdout
set -euo pipefail
cd "$(dirname "$0")"

LOADER="lib/ld-linux-x86-64.so.2"
BIN="fbr-ram-downloader"

if [[ ! -x "$LOADER" || ! -x "$BIN" ]]; then
  echo "error: missing $LOADER or $BIN in $(pwd)" >&2
  exit 1
fi

# Run through the bundled loader, resolving all shared libs from demo/lib.
exec "$LOADER" --library-path "$PWD/lib" "$PWD/$BIN" "$@"
