#!/bin/sh
# Regenerates assets/AppIcon.icns from assets/make-icon.swift.
set -eu
cd "$(dirname "$0")"
set_dir=$(mktemp -d)/AppIcon.iconset
mkdir -p "$set_dir"
for size in 16 32 128 256 512; do
    swift make-icon.swift "$set_dir/icon_${size}x${size}.png" "$size"
    swift make-icon.swift "$set_dir/icon_${size}x${size}@2x.png" "$((size * 2))"
done
iconutil -c icns -o AppIcon.icns "$set_dir"
rm -rf "$(dirname "$set_dir")"
echo "wrote assets/AppIcon.icns"
