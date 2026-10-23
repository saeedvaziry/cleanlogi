#!/bin/sh
# Builds cleanlogi (com.cleanlogi.app), installs it to /Applications and
# (re)starts it at login.
set -eu
cd "$(dirname "$0")/.."
app=$(scripts/build-app.sh)
domain="gui/$(id -u)"
launchctl bootout "$domain/com.cleanlogi.app" 2>/dev/null || true
pkill -f "/Applications/cleanlogi.app/Contents/MacOS/cleanlogi" 2>/dev/null || true
rm -rf /Applications/cleanlogi.app
cp -R "$app" /Applications/
/Applications/cleanlogi.app/Contents/MacOS/cleanlogi install >/dev/null
launchctl bootstrap "$domain" "$HOME/Library/LaunchAgents/com.cleanlogi.app.plist"
echo "installed /Applications/cleanlogi.app"
