#!/bin/sh
# Build a release Bamboo.app and wrap it in a drag-to-Applications disk image.
set -eu
cd "$(dirname "$0")/.."
unset CARGO_TARGET_DIR

version=$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)
cargo build --release

app="dist/Bamboo.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/bamboo "$app/Contents/MacOS/bamboo"
chmod 755 "$app/Contents/MacOS/bamboo"
cp assets/images/bamboo.icns "$app/Contents/Resources/bamboo.icns"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Bamboo</string>
  <key>CFBundleDisplayName</key>
  <string>Bamboo</string>
  <key>CFBundleIdentifier</key>
  <string>com.christopherwk210.bamboo</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleExecutable</key>
  <string>bamboo</string>
  <key>CFBundleIconFile</key>
  <string>bamboo</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>Copyright © 2018 Chris Anselmo. Rust rewrite © 2026 Tamás Varga and Grok. MIT License.</string>
</dict>
</plist>
EOF

# Use a Developer ID if one is installed. Otherwise sign just for this Mac.
identity=$(security find-identity -v -p codesigning 2>/dev/null | awk -F '"' '/Developer ID Application/ { print $2; exit }')
if [ -n "$identity" ]; then
  codesign --force --sign "$identity" "$app"
else
  codesign --force --sign - "$app"
fi
codesign --verify --strict --verbose=2 "$app"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/Bamboo.app"
ln -s /Applications "$stage/Applications"
dmg="dist/Bamboo-${version}.dmg"
rm -f "$dmg"
hdiutil create -volname Bamboo -srcfolder "$stage" -ov -format UDZO "$dmg"
echo "Wrote $dmg"
