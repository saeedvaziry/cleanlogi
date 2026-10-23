#!/bin/sh
# Removes the root-owned parts of Logi Options+, Logi Plugin Service and
# Logi RightSight. Run with: sudo sh scripts/remove-logi.sh
# Leaves /Applications/logioptionsplus.app for you to drag to the Trash.
set -u
[ "$(id -u)" = 0 ] || { echo "run with sudo"; exit 1; }

launchctl bootout system/com.logi.optionsplus.updater 2>/dev/null
launchctl disable system/com.logi.optionsplus.updater
pkill -f logioptionsplus_ 2>/dev/null
pkill -f LogiPluginService 2>/dev/null
pkill -f LogiRightSight 2>/dev/null

rm -fv /Library/LaunchDaemons/com.logi.optionsplus.updater.plist \
       /Library/LaunchAgents/com.logi.optionsplus.plist \
       /Library/LaunchAgents/com.logitech.LogiRightSight.Agent.plist
rm -rf "/Library/Application Support/Logitech.localized" \
       "/Library/Application Support/Logi" \
       "/Applications/Utilities/Logi Options+ Driver Installer.bundle" \
       "/Applications/Utilities/LogiPluginService.app" \
       /tmp/logi.optionsplus.updater.log /tmp/logitech_kiros_updater \
       /tmp/devio_semaphore_logi_hpp_OptionsPlus_*
for pkg in com.logitech.LogiRightSightForWebcams.pkg \
           com.logi.installer.pluginservice.media.package \
           com.logi.installer.pluginservice.package; do
    pkgutil --forget "$pkg" >/dev/null 2>&1
done

echo
echo "Remaining Logitech processes:"
ps -axo pid,comm | grep -i -E "logi(options|plugin|rightsight)" | grep -v grep || echo "  none"
echo "Done. Drag /Applications/logioptionsplus.app to the Trash."
