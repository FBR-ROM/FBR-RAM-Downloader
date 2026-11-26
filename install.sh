#!/usr/bin/env bash
#
# install.sh — install FBR RAM Downloader.
#
# One command puts a single `fbr-ram-downloader` on your PATH, plus the
# root V100 agent and its sudoers rule. Everything it installs is self-
# contained: the cracktro ships its own glibc + ALSA (so it runs on any
# x86-64 Linux regardless of the system's glibc), and the agent is a static
# binary with no dependencies.
#
#   ./install.sh                 # install to /usr/local (uses sudo if needed)
#   ./install.sh --prefix /opt   # install to /opt instead
#   ./install.sh --dry-run       # show what would be done, change nothing
#   ./install.sh --no-sudoers    # skip the sudoers rule (manage sudo yourself)
#
# What it installs (under $PREFIX, default /usr/local):
#   bin/fbr-ram-downloader        the launcher, on your PATH
#   lib/fbr-ram-downloader/       the cracktro + its bundled glibc/ALSA + music
#   sbin/v100-unlock              the static V100 agent (root-owned)
#   /etc/sudoers.d/fbr-ram-downloader   lets your user run the agent as root
#
# The cracktro + launcher only need write access to $PREFIX, so a non-root
# user can install them to a writable prefix (e.g. ~/.local). The agent and
# the sudoers rule need root; without root they are skipped with a warning.
#
# After install:
#   fbr-ram-downloader                 the real hack, interactive
#   fbr-ram-downloader --demo          the simulated download, interactive
#   fbr-ram-downloader --headless      the real hack, plain text (scripts/systemd)
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

# --- args --------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)    PREFIX="${2:?--prefix needs a path}"; shift 2 ;;
    --prefix=*)  PREFIX="${1#--prefix=}"; shift ;;
    --dry-run)   DRY_RUN=1; shift ;;
    --no-sudoers) NO_SUDOERS=1; shift ;;
    -h|--help)   grep '^# ' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
done

# --- the sources we install --------------------------------------------------
SRC_BIN="$BUNDLE/fbr-ram-downloader"
SRC_LIB="$BUNDLE/lib"
SRC_MUSIC="$BUNDLE/music"
SRC_AGENT="$BUNDLE/v100-unlock"

# --- what we will install where ---------------------------------------------
APP_DIR="$PREFIX/lib/fbr-ram-downloader"
BIN_LINK="$PREFIX/bin/fbr-ram-downloader"
AGENT_DEST="$PREFIX/sbin/v100-unlock"
SUDOERS_FILE="/etc/sudoers.d/fbr-ram-downloader"

# --- sanity: the bundle must be present and complete -------------------------
missing=0
for f in "$SRC_BIN" "$SRC_AGENT" "$SRC_LIB/ld-linux-x86-64.so.2" "$SRC_LIB/libasound.so.2"; do
  if [[ ! -e "$f" ]]; then
    echo "error: missing $f" >&2
    echo "       the committed demo/ bundle is incomplete; check out the repo fully." >&2
    missing=1
  fi
done
[[ $missing -eq 0 ]] || exit 1

# --- privilege ---------------------------------------------------------------
# Root needs no escalation. A non-root user uses sudo only when it works
# without a password (so the install never hangs on a prompt); otherwise a
# writable prefix still gets the cracktro + launcher (the agent and sudoers
# rule are skipped with a warning).
IS_ROOT=0
SUDO=""
if [[ $EUID -eq 0 ]]; then
  IS_ROOT=1
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  SUDO="sudo"
fi
# The user who will run the cracktro (for the sudoers rule).
RUN_USER="${SUDO_USER:-$(id -un)}"
[[ "$RUN_USER" == "root" ]] && RUN_USER="$(id -un)"

# Can the current user write to $PREFIX without escalation?
prefix_writable() { [[ -d "$PREFIX" && -w "$PREFIX" ]] || [[ -w "$(dirname "$PREFIX")" ]]; }

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

# --- show the plan -----------------------------------------------------------
step "Installing FBR RAM Downloader"
say "  prefix:    $PREFIX"
say "  launcher:  $BIN_LINK"
say "  app:       $APP_DIR"
say "  agent:     $AGENT_DEST"
say "  sudoers:   $SUDOERS_FILE  (user: $RUN_USER)  $([[ $NO_SUDOERS -eq 1 ]] && echo '[skipped]')"
say "  source:    $BUNDLE"
[[ $DRY_RUN -eq 1 ]] && { say ""; say "(dry run: nothing will be changed)"; exit 0; }

# A non-root user without sudo can only install to a writable prefix.
if [[ $IS_ROOT -eq 0 && -z "$SUDO" ]] && ! prefix_writable; then
  echo "error: $PREFIX is not writable and sudo is not available." >&2
  echo "       re-run as root, or use --prefix to a writable location (e.g. ~/.local)." >&2
  exit 1
fi

# --- 1. the cracktro bundle --------------------------------------------------
step "Installing the cracktro (self-contained: bundled glibc + ALSA)"
run $SUDO mkdir -p "$APP_DIR"
run $SUDO cp -f "$SRC_BIN"   "$APP_DIR/fbr-ram-downloader"
run $SUDO cp -rf "$SRC_LIB"  "$APP_DIR/lib"
if [[ -d "$SRC_MUSIC" ]]; then
  run $SUDO cp -rf "$SRC_MUSIC" "$APP_DIR/music"
fi
run $SUDO chmod 755 "$APP_DIR/fbr-ram-downloader"

# --- 2. the launcher on PATH -------------------------------------------------
step "Installing the launcher on PATH"
run $SUDO mkdir -p "$PREFIX/bin"
# The launcher is relocatable: it finds the app relative to its own location
# and points the cracktro at the installed agent via FBR_V100_AGENT.
LAUNCHER_CONTENT="#!/bin/sh
# FBR RAM Downloader launcher (installed by install.sh).
ROOT=\"\$(cd \"\$(dirname \"\$(readlink -f \"\$0\")\")/..\" && pwd)\"
APP=\"\$ROOT/lib/fbr-ram-downloader\"
export FBR_V100_AGENT=\"\$ROOT/sbin/v100-unlock\"
exec \"\$APP/lib/ld-linux-x86-64.so.2\" --library-path \"\$APP/lib\" \"\$APP/fbr-ram-downloader\" \"\$@\"
"
if [[ $DRY_RUN -eq 0 ]]; then
  printf '%s' "$LAUNCHER_CONTENT" | $SUDO tee "$BIN_LINK" >/dev/null
  $SUDO chmod 755 "$BIN_LINK"
fi

# --- 3. the static V100 agent (needs root) -----------------------------------
if [[ $IS_ROOT -eq 1 || -n "$SUDO" ]]; then
  step "Installing the V100 agent (static, root-owned)"
  run $SUDO mkdir -p "$PREFIX/sbin"
  run $SUDO install -o root -g root -m 755 "$SRC_AGENT" "$AGENT_DEST"
else
  step "Skipping the V100 agent (needs root)"
  say "  install it later as root:"
  say "    sudo install -o root -g root -m 755 $SRC_AGENT $AGENT_DEST"
fi

# --- 4. the sudoers rule (needs root) ----------------------------------------
if [[ $NO_SUDOERS -eq 0 ]]; then
  if [[ $IS_ROOT -eq 1 || -n "$SUDO" ]]; then
    step "Installing the sudoers rule (lets $RUN_USER run the agent as root)"
    RULE="$RUN_USER ALL=(root) NOPASSWD: $AGENT_DEST"
    if [[ $DRY_RUN -eq 0 ]]; then
      # Write to a temp file, validate it, then move it in — never leave a
      # broken sudoers file behind.
      TMP="$($SUDO mktemp /etc/sudoers.d/.fbr.XXXXXX 2>/dev/null || mktemp)"
      printf '%s\n' "$RULE" > "$TMP"
      if command -v visudo >/dev/null 2>&1; then
        if ! visudo -cf "$TMP" >/dev/null 2>&1; then
          rm -f "$TMP"
          echo "error: the generated sudoers rule failed validation; not installed." >&2
          exit 1
        fi
      fi
      $SUDO install -m 440 -o root -g root "$TMP" "$SUDOERS_FILE"
      rm -f "$TMP"
    else
      printf '  [dry-run] sudoers rule: %s\n' "$RULE"
    fi
  else
    step "Skipping the sudoers rule (needs root)"
    say "  add it later as root:"
    say "    echo '$RUN_USER ALL=(root) NOPASSWD: $AGENT_DEST' | sudo tee /etc/sudoers.d/fbr-ram-downloader"
  fi
fi

# --- 5. verify ---------------------------------------------------------------
step "Verifying"
if [[ $DRY_RUN -eq 0 ]]; then
  if [[ -x "$AGENT_DEST" ]]; then
    # A simulated headless run needs no root and no hardware: it proves the
    # launcher, the bundled libraries, and the agent all work together.
    if "$BIN_LINK" --headless --demo >/dev/null 2>&1; then
      say "  ok: fbr-ram-downloader --headless --demo ran cleanly"
    else
      say "  warning: verification run failed; check the install." >&2
      exit 1
    fi
  else
    # No agent (skipped, no root): verify the cracktro itself runs.
    if "$BIN_LINK" --dump 1 80 24 >/dev/null 2>&1; then
      say "  ok: fbr-ram-downloader runs (agent not installed; add it as root for the real hack)"
    else
      say "  warning: the cracktro did not run; check the install." >&2
      exit 1
    fi
  fi
fi

step "Done"
say "  fbr-ram-downloader                  the real hack, interactive"
say "  fbr-ram-downloader --demo           the simulated download"
say "  fbr-ram-downloader --headless       the real hack, plain text"
say "  fbr-ram-downloader --headless --dry-run   survey, write nothing"
say ""
say "To target another machine: fbr-ram-downloader --host <name>"
