#!/bin/sh
# Builds a release: universal app signed for notarization, packed in a signed
# disk image at build/cleanlogi-<version>.dmg. Notarizing is the workflow's job.
#
#   CLEANLOGI_SIGN_IDENTITY="Developer ID Application: ..." scripts/build-release.sh <version>
set -eu
cd "$(dirname "$0")/.."
version=${1:?usage: scripts/build-release.sh <version>}
identity=${CLEANLOGI_SIGN_IDENTITY:?set CLEANLOGI_SIGN_IDENTITY to a Developer ID Application identity}

app=$(scripts/build-app.sh --universal --release --version "$version" --output build/release)
image="build/cleanlogi-$version.dmg"
rm -f "$image"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
if command -v create-dmg >/dev/null 2>&1; then
    # create-dmg (npm) lays out the window nicely; it also tries to sign, which we do below.
    create-dmg --overwrite "$app" "$stage" >/dev/null 2>&1 || true
    mv "$stage"/*.dmg "$image"
else
    mkdir "$stage/cleanlogi"
    cp -R "$app" "$stage/cleanlogi/"
    ln -s /Applications "$stage/cleanlogi/Applications"
    hdiutil create -quiet -volname cleanlogi -srcfolder "$stage/cleanlogi" -format UDZO -ov "$image"
fi
codesign --force --sign "$identity" --timestamp "$image"
echo "$image"
