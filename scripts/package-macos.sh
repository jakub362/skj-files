#!/usr/bin/env bash
# Build SKJ Files for macOS (universal: Apple Silicon + Intel) from Linux and package "SKJ Files.app" as a zip.
#
# One-time setup:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   pip install "ziglang==0.16.0" cargo-zigbuild     # keep zig at 0.16: zig 0.17 writes duplicate
#                                                    # framework load commands and dyld refuses to start the app
#   a macOS SDK folder (MacOSX14.x.sdk) -> export SDKROOT=/path/to/MacOSX14.5.sdk
#
# On a Mac you can skip zig: cargo build --release --target aarch64-apple-darwin (and x86_64), then lipo.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${SDKROOT:?set SDKROOT to a macOS SDK folder}"
export SDKROOT MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=${OUT:-dist}

cargo zigbuild --release --target universal2-apple-darwin
BIN=target/universal2-apple-darwin/release/skj-files

APP="$OUT/macos/SKJ Files.app"
rm -rf "$OUT/macos" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/skj-files"
chmod 755 "$APP/Contents/MacOS/skj-files"
printf 'APPL????' > "$APP/Contents/PkgInfo"
python3 - "$APP/Contents/Resources/skj-files.icns" <<'EOF'
import sys
from PIL import Image
Image.open("assets/skj-files.png").convert("RGBA").save(sys.argv[1], format="ICNS")
EOF

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

ZIP="$PWD/$OUT/SKJ-Files-${VERSION}-macos-universal.zip"
rm -f "$ZIP"
(cd "$OUT/macos" && zip -qry "$ZIP" "SKJ Files.app")
echo "$ZIP"
