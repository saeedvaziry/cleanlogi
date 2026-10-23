#!/bin/sh
# Builds and runs "CleanLogi Dev" (com.cleanlogi.dev) with debug logging.
# An installed CleanLogi is paused meanwhile, so the two don't both drive the
# mouse, and resumed when this exits (Ctrl-C).
set -eu
cd "$(dirname "$0")/.."
app=$(scripts/build-app.sh --dev)

domain="gui/$(id -u)"
release_plist="$HOME/Library/LaunchAgents/com.cleanlogi.app.plist"
resume=""
if launchctl print "$domain/com.cleanlogi.app" >/dev/null 2>&1; then
    launchctl bootout "$domain/com.cleanlogi.app"
    resume=1
fi
pkill -if "/CleanLogi.app/Contents/MacOS/cleanlogi" 2>/dev/null || true

log=build/dev.log
: > "$log"
cleanup() {
    pkill -f "$app/Contents/MacOS/cleanlogi" 2>/dev/null || true
    if [ -n "$resume" ]; then
        launchctl bootstrap "$domain" "$release_plist" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM
# Launched through LaunchServices so macOS attributes permissions to the
# dev bundle rather than to the terminal.
open -n --env CLEANLOGI_DEBUG=1 --stderr "$PWD/$log" "$app"
echo "running $app (Ctrl-C to stop)"
tail -f "$log"
