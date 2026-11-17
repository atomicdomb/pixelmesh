#!/bin/bash
# Build PixelMesh.app for macOS (Apple Silicon + Intel), sign it and, if asked, notarize it.
#
#   scripts/macos-package.sh
#       Ad-hoc signed app in dist/. Fine on the Mac that built it.
#
#   SIGN_ID="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=notary scripts/macos-package.sh
#       Developer ID signed, notarized and stapled: opens with a normal double-click on any Mac.
#
# One-time setup for the second form is described in the README ("Building a Mac app").
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="PixelMesh"
BUNDLE_ID="${BUNDLE_ID:-studio.butterflyknights.pixelmesh}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
SIGN_ID="${SIGN_ID:--}"                 # "-" means ad-hoc
NOTARY_PROFILE="${NOTARY_PROFILE:-}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

DIST="dist"
APP="$DIST/$APP_NAME.app"
ZIP="$DIST/$APP_NAME-$VERSION-mac.zip"

echo "==> Building universal binary (version $VERSION)"
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

echo "==> Assembling $APP"
rm -rf "$APP" "$ZIP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/pixelmesh" \
    target/aarch64-apple-darwin/release/pixelmesh \
    target/x86_64-apple-darwin/release/pixelmesh

# App icon: assets/app-icon.png (1024x1024) -> AppIcon.icns
ICON_KEY=""
if [ -f assets/app-icon.png ]; then
    ICONSET="$(mktemp -d)/AppIcon.iconset"
    mkdir -p "$ICONSET"
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" assets/app-icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
        sips -z "$((size * 2))" "$((size * 2))" assets/app-icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
    ICON_KEY="<key>CFBundleIconFile</key><string>AppIcon</string>"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>pixelmesh</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
    <key>NSHighResolutionCapable</key><true/>
    $ICON_KEY
</dict>
</plist>
PLIST

if [ "$SIGN_ID" = "-" ]; then
    echo "==> Signing ad hoc"
    codesign --force --sign - "$APP"
else
    echo "==> Signing as: $SIGN_ID"
    # Hardened runtime and a secure timestamp are both required for notarization.
    codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"

if [ -n "$NOTARY_PROFILE" ]; then
    if [ "$SIGN_ID" = "-" ]; then
        echo "NOTARY_PROFILE is set but SIGN_ID is not: notarization needs a Developer ID signature." >&2
        exit 1
    fi
    echo "==> Notarizing (this usually takes a few minutes)"
    ditto -c -k --keepParent "$APP" "$ZIP"
    xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$APP"
    rm "$ZIP"
    spctl --assess --type execute --verbose=2 "$APP"
fi

# The zip to hand over. ditto keeps the signature and the stapled ticket intact.
ditto -c -k --keepParent "$APP" "$ZIP"
echo "==> Done: $APP"
echo "    To share: $ZIP"
