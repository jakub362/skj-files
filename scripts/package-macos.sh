#!/usr/bin/env bash
# Build SKJ Files for macOS (universal: Apple Silicon + Intel) and package "SKJ Files.app".
#
# Two ways in, picked automatically:
#
#   On a Mac (what CI uses) - no zig, no SDK, just the Apple toolchain:
#       bash scripts/package-macos.sh
#
#   Cross-compiled from Linux - needs cargo-zigbuild and a macOS SDK:
#       rustup target add aarch64-apple-darwin x86_64-apple-darwin
#       pip install "ziglang==0.16.0" cargo-zigbuild    # keep zig at 0.16: zig 0.17 writes duplicate
#                                                       # framework load commands and dyld refuses to start the app
#       SDKROOT=/path/to/MacOSX14.5.sdk bash scripts/package-macos.sh
#
# Set DMG=1 to also build a .dmg (only possible on a Mac - hdiutil is macOS-only).
set -euo pipefail
cd "$(dirname "$0")/.."

export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=${OUT:-dist}
NATIVE=0
[ "$(uname -s)" = "Darwin" ] && NATIVE=1

if [ "$NATIVE" = "1" ]; then
    echo "Building natively on macOS (both architectures, then lipo)"
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p target/universal2-apple-darwin/release
    BIN=target/universal2-apple-darwin/release/skj-files
    lipo -create -output "$BIN" \
        target/aarch64-apple-darwin/release/skj-files \
        target/x86_64-apple-darwin/release/skj-files
else
    : "${SDKROOT:?On Linux, set SDKROOT to a macOS SDK folder (or run this on a Mac)}"
    export SDKROOT
    cargo zigbuild --release --target universal2-apple-darwin
    BIN=target/universal2-apple-darwin/release/skj-files
fi

APP="$OUT/macos/SKJ Files.app"
rm -rf "$OUT/macos" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/skj-files"
chmod 755 "$APP/Contents/MacOS/skj-files"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Icon: iconutil on a Mac makes a proper multi-resolution .icns; Pillow is the fallback off-Mac.
if [ "$NATIVE" = "1" ]; then
    ICONSET=$(mktemp -d)/skj-files.iconset
    mkdir -p "$ICONSET"
    for s in 16 32 64 128 256 512; do
        sips -z $s $s assets/skj-files.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
        d=$((s * 2))
        sips -z $d $d assets/skj-files.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/skj-files.icns"
    rm -rf "$(dirname "$ICONSET")"
else
    python3 - "$APP/Contents/Resources/skj-files.icns" <<'EOF'
import sys
from PIL import Image
Image.open("assets/skj-files.png").convert("RGBA").save(sys.argv[1], format="ICNS")
EOF
fi

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>pl</string></array>
  <key>CFBundleDisplayName</key><string>SKJ Files</string>
  <key>CFBundleName</key><string>SKJ Files</string>
  <key>CFBundleExecutable</key><string>skj-files</string>
  <key>CFBundleIconFile</key><string>skj-files</string>
  <key>CFBundleIdentifier</key><string>com.skjtech.files</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>${MACOSX_DEPLOYMENT_TARGET}</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSRequiresAquaSystemAppearance</key><false/>
  <key>NSHumanReadableCopyright</key><string>© 2026 SKJ Tech</string>
</dict>
</plist>
EOF

# An ad-hoc signature is required on Apple Silicon or the app will not start at all.
# It is not a Developer ID: users still get Gatekeeper's warning on first run.
if [ "$NATIVE" = "1" ]; then
    codesign --force --deep --sign - "$APP"
    codesign --verify --deep --strict "$APP" && echo "ad-hoc signature ok"
fi

ZIP="$PWD/$OUT/SKJ-Files-${VERSION}-macos-universal.zip"
rm -f "$ZIP"
(cd "$OUT/macos" && zip -qry "$ZIP" "SKJ Files.app")
echo "$ZIP"

if [ "${DMG:-0}" = "1" ]; then
    if [ "$NATIVE" != "1" ]; then
        echo "DMG=1 needs a Mac (hdiutil is macOS-only) - skipping the .dmg" >&2
    else
        STAGE=$(mktemp -d)/dmg
        mkdir -p "$STAGE"
        cp -R "$APP" "$STAGE/"
        ln -s /Applications "$STAGE/Applications"
        DMGFILE="$PWD/$OUT/SKJ-Files-${VERSION}-macos-universal.dmg"
        rm -f "$DMGFILE"
        hdiutil create -volname "SKJ Files ${VERSION}" -srcfolder "$STAGE" \
            -ov -format UDZO -quiet "$DMGFILE"
        rm -rf "$(dirname "$STAGE")"
        echo "$DMGFILE"
    fi
fi
