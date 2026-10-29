#!/bin/sh
# Builds a release text9000.app in target/ with the icon, name and version.
# Usage: scripts/bundle.sh            (CARGO_FLAGS="--no-default-features" for text9000 Lite)
#        UNIVERSAL=1 scripts/bundle.sh  one app for both Apple Silicon and Intel Macs
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/text9000.app

# shellcheck disable=SC2086  # CARGO_FLAGS is a list of flags
if [ "$UNIVERSAL" = 1 ]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release $CARGO_FLAGS --target aarch64-apple-darwin
  cargo build --release $CARGO_FLAGS --target x86_64-apple-darwin
  mkdir -p target/release
  lipo -create -output target/release/text9000 \
    target/aarch64-apple-darwin/release/text9000 target/x86_64-apple-darwin/release/text9000
else
  cargo build --release $CARGO_FLAGS
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/text9000 "$APP/Contents/MacOS/text9000"
cp -R themes "$APP/Contents/Resources/themes"

# Turn the 1024px PNG into an .icns with every size macOS asks for.
ICONSET=target/text9000.iconset
rm -rf "$ICONSET" && mkdir "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/text9000.icns"
rm -rf "$ICONSET"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>text9000</string>
  <key>CFBundleDisplayName</key><string>text9000</string>
  <key>CFBundleIdentifier</key><string>app.text9000.editor</string>
  <key>CFBundleExecutable</key><string>text9000</string>
  <key>CFBundleIconFile</key><string>text9000</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>text9000 link</string>
      <key>CFBundleURLSchemes</key><array><string>text9000</string><string>folio</string></array>
    </dict>
  </array>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>text9000 Document</string>
      <key>CFBundleTypeExtensions</key><array><string>t9k</string></array>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>CFBundleTypeIconFile</key><string>text9000</string>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Folio Document</string>
      <key>CFBundleTypeExtensions</key><array><string>folio</string></array>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>CFBundleTypeIconFile</key><string>text9000</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Ad-hoc signature: not a developer certificate, but Apple Silicon needs it to run.
codesign --force --deep --sign - "$APP"

echo "Built $APP (version $VERSION)"
