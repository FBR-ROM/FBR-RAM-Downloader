#!/usr/bin/env bash
#
# install.sh — install or uninstall FBR RAM Downloader.
#
# One command puts `fbr-ram-downloader` on your PATH, installs the static V100
# agent root-owned, and writes the sudoers rule that lets your user run it as
# root. Everything it installs is self-contained: the cracktro ships its own
# glibc + ALSA (so it runs on any x86-64 Linux regardless of system glibc), and
# the agent is a static binary with no dependencies.
#
#   sudo ./install.sh                   # install to /usr/local
#   sudo ./install.sh --prefix /opt     # install to /opt instead
#   sudo ./install.sh --user arikami    # grant the sudoers rule to arikami
#   ./install.sh --dry-run              # show what would be done (no root needed)
#   sudo ./install.sh --no-sudoers      # skip the sudoers rule
#   sudo ./install.sh --uninstall       # remove installed files and sudoers rule
#
# What it installs (under $PREFIX, default /usr/local):
#   bin/fbr-ram-downloader              the launcher, on your PATH
#   lib/fbr-ram-downloader/             the cracktro + bundled glibc/ALSA + music
#   sbin/v100-unlock                    the static V100 agent (root-owned)
#   /etc/sudoers.d/fbr-ram-downloader   lets your user run the agent as root
#
# Root is required: the agent is installed root-owned and the sudoers rule
# lives in /etc/sudoers.d. Run it as root, or as a user with passwordless
# sudo (the script escalates with `sudo -n` and never prompts).
#
# After install:
#   fbr-ram-downloader                        the real hack, interactive
#   fbr-ram-downloader --demo                 the simulated download, interactive
#   fbr-ram-downloader --headless             the real hack, plain text (scripts/systemd)
#   fbr-ram-downloader --headless --dry-run   survey, write nothing
#
set -euo pipefail

# --- locate the repo (this script lives at the repo root) -------------------
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUNDLE="$ROOT/demo"

# --- defaults ----------------------------------------------------------------
PREFIX="/usr/local"
DRY_RUN=0
NO_SUDOERS=0
UNINSTALL=0
RUN_USER=""

# --- args --------------------------------------------------------------------
ORIG_ARGS="$*"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)      PREFIX="${2:?--prefix needs a path}"; shift 2 ;;
    --prefix=*)    PREFIX="${1#--prefix=}"; shift ;;
    --user)        RUN_USER="${2:?--user needs a name}"; shift 2 ;;
    --user=*)      RUN_USER="${1#--user=}"; shift ;;
    --dry-run)     DRY_RUN=1; shift ;;
    --no-sudoers)  NO_SUDOERS=1; shift ;;
    --uninstall)   UNINSTALL=1; shift ;;
    -h|--help)     grep '^# ' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
done

# --- platform guard ----------------------------------------------------------
OS="$(uname -s)"
ARCH="$(uname -m)"
if [[ "$OS" != "Linux" || "$ARCH" != "x86_64" ]]; then
  echo "error: install.sh installs the prebuilt x86_64-linux bundle (this machine is $OS/$ARCH)." >&2
  echo "       build from source with \`nix build .\` or \`cargo build --release\` (see README)." >&2
  exit 1
fi

# --- validate prefix ---------------------------------------------------------
if [[ "$PREFIX" != /* || "$PREFIX" =~ [[:space:][:cntrl:]] ]]; then
  echo "error: --prefix must be an absolute path with no whitespace: $PREFIX" >&2
  exit 2
fi
PREFIX="${PREFIX%/}"
[[ -z "$PREFIX" ]] && PREFIX="/"

# --- the sources we install --------------------------------------------------
SRC_BIN="$BUNDLE/fbr-ram-downloader"
SRC_LIB="$BUNDLE/lib"
SRC_MUSIC="$BUNDLE/music"
SRC_AGENT="$BUNDLE/v100-unlock"

# --- what we will install where ---------------------------------------------
APP_DIR="$PREFIX/lib/fbr-ram-downloader"
BIN_LINK="$PREFIX/bin/fbr-ram-downloader"
AGENT_DEST="$PREFIX/sbin/v100-unlock"
SUDOERS_DIR="${FBR_SUDOERS_DIR:-/etc/sudoers.d}"
SUDOERS_FILE="$SUDOERS_DIR/fbr-ram-downloader"

# --- sanity: the bundle must be present and complete (install only) ----------
if [[ $UNINSTALL -eq 0 ]]; then
  missing=0
  for f in \
    "$SRC_BIN" \
    "$SRC_AGENT" \
    "$SRC_LIB/ld-linux-x86-64.so.2" \
    "$SRC_LIB/libc.so.6" \
    "$SRC_LIB/libm.so.6" \
    "$SRC_LIB/libpthread.so.0" \
    "$SRC_LIB/librt.so.1" \
    "$SRC_LIB/libgcc_s.so.1" \
    "$SRC_LIB/libasound.so.2"; do
    if [[ ! -e "$f" ]]; then
      echo "error: missing $f" >&2
      echo "       the committed demo/ bundle is incomplete; check out the repo fully." >&2
      missing=1
    fi
  done
  [[ $missing -eq 0 ]] || exit 1
fi

# --- privilege ---------------------------------------------------------------
# Root is required to install or uninstall: the agent is root-owned and the
# sudoers rule lives in /etc/sudoers.d. Run as root, or as a user with
# passwordless sudo (the script escalates with `sudo -n` and never prompts).
# --dry-run changes nothing, so it is allowed without root.
SUDO=""
if [[ $EUID -eq 0 ]]; then
  :
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  SUDO="sudo"
elif [[ $DRY_RUN -eq 1 ]]; then
  :   # preview only; nothing is written
else
  echo "error: this action needs root (the agent is root-owned and the" >&2
  echo "       sudoers rule lives in $SUDOERS_DIR)." >&2
  echo "       re-run as:  sudo $0 $ORIG_ARGS" >&2
  exit 1
fi

# --- helpers -----------------------------------------------------------------
say()  { printf '%s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }
run()  {  # run a command, honouring --dry-run
  if [[ $DRY_RUN -eq 1 ]]; then
    printf '  [dry-run] %s\n' "$*"
  else
    "$@"
  fi
}

# --- uninstall path ----------------------------------------------------------
if [[ $UNINSTALL -eq 1 ]]; then
  step "Uninstalling FBR RAM Downloader"
  say "  launcher:  $BIN_LINK"
  say "  app:       $APP_DIR"
  say "  agent:     $AGENT_DEST"
  say "  sudoers:   $SUDOERS_FILE  $([[ $NO_SUDOERS -eq 1 ]] && echo '[skipped]')"
  [[ $DRY_RUN -eq 1 ]] && { say ""; say "(dry run: nothing will be changed)"; exit 0; }

  run $SUDO rm -f "$BIN_LINK"
  run $SUDO rm -rf "$APP_DIR"
  run $SUDO rm -f "$AGENT_DEST"
  if [[ $NO_SUDOERS -eq 0 ]]; then
    run $SUDO rm -f "$SUDOERS_FILE"
  fi
  step "Done"
  say "  removed FBR RAM Downloader from $PREFIX"
  exit 0
fi

# --- validate sudoers user ---------------------------------------------------
if [[ -z "$RUN_USER" ]]; then
  RUN_USER="${SUDO_USER:-$(id -un)}"
fi
if [[ $NO_SUDOERS -eq 0 ]]; then
  if [[ "$RUN_USER" == "root" ]]; then
    echo "error: the sudoers rule needs a non-root user to grant." >&2
    echo "       re-run with:  sudo $0 $ORIG_ARGS --user <name>" >&2
    exit 1
  fi
  if [[ ! "$RUN_USER" =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.-]*[$]?$ ]]; then
    echo "error: invalid user name for --user: $RUN_USER" >&2
    exit 2
  fi
fi

# --- show the plan -----------------------------------------------------------
step "Installing FBR RAM Downloader"
say "  prefix:    $PREFIX"
say "  launcher:  $BIN_LINK"
say "  app:       $APP_DIR"
say "  agent:     $AGENT_DEST"
say "  sudoers:   $SUDOERS_FILE  (user: $RUN_USER)  $([[ $NO_SUDOERS -eq 1 ]] && echo '[skipped]')"
say "  source:    $BUNDLE"
[[ $DRY_RUN -eq 1 ]] && { say ""; say "(dry run: nothing will be changed)"; exit 0; }

# --- 1. the cracktro bundle --------------------------------------------------
step "Installing the cracktro (self-contained: bundled glibc + ALSA)"
run $SUDO mkdir -p "$APP_DIR"
run $SUDO cp -f "$SRC_BIN" "$APP_DIR/fbr-ram-downloader"
run $SUDO rm -rf "$APP_DIR/lib"
run $SUDO cp -rf "$SRC_LIB" "$APP_DIR/lib"
if [[ -d "$SRC_MUSIC" ]]; then
  run $SUDO rm -rf "$APP_DIR/music"
  run $SUDO cp -rf "$SRC_MUSIC" "$APP_DIR/music"
fi
run $SUDO chmod 755 "$APP_DIR/fbr-ram-downloader" "$APP_DIR/lib/ld-linux-x86-64.so.2"

# --- 2. the launcher on PATH -------------------------------------------------
step "Installing the launcher on PATH"
run $SUDO mkdir -p "$PREFIX/bin"
# The launcher is relocatable: it finds the app relative to its own location,
# points the cracktro at the installed agent via FBR_V100_AGENT, and runs the
# bundled loader hermetically (--inhibit-cache --inhibit-rpath '').
LAUNCHER_CONTENT="#!/bin/sh
# FBR RAM Downloader launcher (installed by install.sh).
ROOT=\"\$(cd \"\$(dirname \"\$(readlink -f \"\$0\")\")/..\" && pwd)\"
APP=\"\$ROOT/lib/fbr-ram-downloader\"
export FBR_V100_AGENT=\"\$ROOT/sbin/v100-unlock\"
export FBR_BUNDLED_ALSA=1
exec \"\$APP/lib/ld-linux-x86-64.so.2\" --inhibit-cache --inhibit-rpath '' --library-path \"\$APP/lib\" \"\$APP/fbr-ram-downloader\" \"\$@\"
"
printf '%s' "$LAUNCHER_CONTENT" | $SUDO tee "$BIN_LINK" >/dev/null
$SUDO chmod 755 "$BIN_LINK"

# --- 3. the static V100 agent ------------------------------------------------
step "Installing the V100 agent (static, root-owned)"
run $SUDO mkdir -p "$PREFIX/sbin"
run $SUDO install -o root -g root -m 755 "$SRC_AGENT" "$AGENT_DEST"

# --- 4. the sudoers rule -----------------------------------------------------
if [[ $NO_SUDOERS -eq 0 ]]; then
  step "Installing the sudoers rule (lets $RUN_USER run the agent as root)"
  RULE="$RUN_USER ALL=(root) NOPASSWD: $AGENT_DEST"
  # Stage in a user-writable temp file, validate with visudo, then install
  # atomically with mode 0440 — works both as root and under passwordless sudo.
  TMP="$(mktemp)"
  trap 'rm -f "${TMP:-}"' EXIT
  printf '%s\n' "$RULE" > "$TMP"
  VISUDO=""
  if command -v visudo >/dev/null 2>&1; then
    VISUDO="$(command -v visudo)"
  elif [[ -x /usr/sbin/visudo ]]; then
    VISUDO="/usr/sbin/visudo"
  fi
  if [[ -n "$VISUDO" ]]; then
    if ! $SUDO "$VISUDO" -cf "$TMP" >/dev/null 2>&1; then
      echo "error: the generated sudoers rule failed validation; not installed." >&2
      exit 1
    fi
  fi
  run $SUDO mkdir -p "$SUDOERS_DIR"
  $SUDO install -m 440 -o root -g root "$TMP" "$SUDOERS_FILE"
  rm -f "$TMP"
  trap - EXIT
fi

# --- 5. verify ---------------------------------------------------------------
step "Verifying"
# A simulated headless run needs no root and no hardware: it proves the
# launcher, the bundled libraries, and the agent all work together.
if "$BIN_LINK" --headless --demo >/dev/null 2>&1; then
  say "  ok: fbr-ram-downloader --headless --demo ran cleanly"
else
  say "  warning: verification run failed; check the install." >&2
  exit 1
fi

step "Done"
say "  fbr-ram-downloader                        the real hack, interactive"
say "  fbr-ram-downloader --demo                 the simulated download"
say "  fbr-ram-downloader --headless             the real hack, plain text"
say "  fbr-ram-downloader --headless --dry-run   survey, write nothing"
say ""
say "To target another machine: fbr-ram-downloader --host <name>"
