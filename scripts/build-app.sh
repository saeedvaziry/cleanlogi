#!/bin/sh
# Builds the app bundle.
#
#   scripts/build-app.sh [--dev] [--universal] [--release] [--version X.Y.Z] [--output DIR]
#
#   --dev        "cleanlogi Dev" (com.cleanlogi.dev): its own permissions and login item
#   --universal  arm64 + x86_64
#   --release    sign with a secure timestamp, as notarization requires
#   --version    CFBundleShortVersionString (default: Cargo.toml version)
#   --output     folder for the .app (default: build)
#
# Signs with $CLEANLOGI_SIGN_IDENTITY, else the first Apple Development identity
# in the keychain, else ad-hoc. Prints the path of the built app.
set -eu
cd "$(dirname "$0")/.."

dev="" universal="" release="" output=build
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
while [ $# -gt 0 ]; do
    case $1 in
        --dev) dev=1; shift ;;
        --universal) universal=1; shift ;;
        --release) release=1; shift ;;
        --version) version=$2; shift 2 ;;
        --output) output=$2; shift 2 ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done

if [ -n "$dev" ]; then
    name="cleanlogi Dev" bundle_id=com.cleanlogi.dev profile=dev target_dir=debug
    export CLEANLOGI_DEV=1
else
    name=cleanlogi bundle_id=com.cleanlogi.app profile=release target_dir=release
fi
build_number=$(git rev-list --count HEAD 2>/dev/null || echo 1)
export MACOSX_DEPLOYMENT_TARGET=15.0

if [ -n "$universal" ]; then
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --profile "$profile" --target "$t" >&2
    done
    bin=$(mktemp -d)/cleanlogi
    lipo -create -output "$bin" \
        "target/aarch64-apple-darwin/$target_dir/cleanlogi" \
        "target/x86_64-apple-darwin/$target_dir/cleanlogi"
else
    cargo build --profile "$profile" >&2
    bin="target/$target_dir/cleanlogi"
fi

app="$output/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/cleanlogi"
cp assets/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleExecutable</key><string>cleanlogi</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$build_number</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSUIElement</key><true/>
  <key>NSHumanReadableCopyright</key><string>Licensed under the GNU AGPL v3</string>
</dict>
</plist>
PLIST

# A stable signing identity keeps macOS privacy grants across rebuilds;
# ad-hoc signatures change every build and need re-granting.
identity=${CLEANLOGI_SIGN_IDENTITY:-$(security find-identity -v -p codesigning |
    sed -n 's/.*"\(Apple Development[^"]*\)".*/\1/p' | head -1)}
set -- --force --options runtime --sign "${identity:--}"
[ -n "$release" ] && set -- "$@" --timestamp
codesign "$@" "$app" >&2
echo "$app"
