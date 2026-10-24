#!/bin/sh
# Builds a release Folio.app in target/ with the icon, name and version.
# Usage: scripts/bundle.sh            (CARGO_FLAGS="--no-default-features" for folio-lite)
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/Folio.app

# shellcheck disable=SC2086  # CARGO_FLAGS is a list of flags
cargo build --release $CARGO_FLAGS

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/folio "$APP/Contents/MacOS/folio"
cp -R themes "$APP/Contents/Resources/themes"

# Turn the 1024px PNG into an .icns with every size macOS asks for.
ICONSET=target/Folio.iconset
rm -rf "$ICONSET" && mkdir "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Folio.icns"
rm -rf "$ICONSET"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Folio</string>
  <key>CFBundleDisplayName</key><string>Folio</string>
  <key>CFBundleIdentifier</key><string>app.folio.editor</string>
  <key>CFBundleExecutable</key><string>folio</string>
  <key>CFBundleIconFile</key><string>Folio</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Folio Document</string>
      <key>CFBundleTypeExtensions</key><array><string>folio</string></array>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>CFBundleTypeIconFile</key><string>Folio</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Ad-hoc signature: not a developer certificate, but Apple Silicon needs it to run.
codesign --force --deep --sign - "$APP"

echo "Built $APP (version $VERSION)"
