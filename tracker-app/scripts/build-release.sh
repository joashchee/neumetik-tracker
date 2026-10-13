#!/bin/sh
# Builds a release Neumetik Tracker without the build machine's paths in
# it. rustc bakes source file paths into panic messages (the tracker's
# crates above this folder, and every dependency under ~/.cargo), which
# would publish the builder's home folder; --remap-path-prefix rewrites
# them. When several prefixes match, rustc applies the last one, so the
# more specific paths come after $HOME.
#
# Arguments go on to `tauri build`, e.g. `--bundles app`, or for phones
# `android build` / `ios build` in place of it (TAURI_COMMAND). Afterwards
# the built app is searched for $HOME, and the script fails if it's there.
set -eu

APP_DIR=$(cd "$(dirname "$0")/.." && pwd)
ROOT=$(cd "$APP_DIR/.." && pwd)
cd "$APP_DIR"

export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=~ --remap-path-prefix=$ROOT=."
if [ -n "${CARGO_HOME:-}" ]; then
  RUSTFLAGS="$RUSTFLAGS --remap-path-prefix=$CARGO_HOME=~/.cargo"
fi
export CFLAGS="${CFLAGS:-} -ffile-prefix-map=$HOME=~ -ffile-prefix-map=$ROOT=."
export CXXFLAGS="${CXXFLAGS:-} -ffile-prefix-map=$HOME=~ -ffile-prefix-map=$ROOT=."

npx tauri ${TAURI_COMMAND:-build} "$@"

TARGET="${CARGO_TARGET_DIR:-$APP_DIR/target}/release"
LEAKS=$(grep -r -l -a -F "$HOME" "$TARGET/neumetik-tracker-app" "$TARGET/bundle/macos" "$APP_DIR/dist" 2>/dev/null || true)
if [ -n "$LEAKS" ]; then
  echo "The build still contains $HOME in:" >&2
  echo "$LEAKS" >&2
  exit 1
fi
echo "Checked: no $HOME paths in the release build."
