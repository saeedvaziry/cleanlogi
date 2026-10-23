#!/bin/sh
# Builds CleanLogi (com.cleanlogi.app), installs it to /Applications and
# (re)starts it at login.
set -eu
cd "$(dirname "$0")/.."
app=$(scripts/build-app.sh)
domain="gui/$(id -u)"
launchctl bootout "$domain/com.cleanlogi.app" 2>/dev/null || true
pkill -if "/Applications/CleanLogi.app/Contents/MacOS/cleanlogi" 2>/dev/null || true
rm -rf /Applications/CleanLogi.app
cp -R "$app" /Applications/
/Applications/CleanLogi.app/Contents/MacOS/cleanlogi install >/dev/null
launchctl bootstrap "$domain" "$HOME/Library/LaunchAgents/com.cleanlogi.app.plist"
echo "installed /Applications/CleanLogi.app"
