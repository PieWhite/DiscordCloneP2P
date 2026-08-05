#!/bin/bash
# Bouwt FitCommunication.app en verpakt hem als zip — de mac-tweeling van "losse exe
# in een zip". Geen tauri-cli, geen Node: een release-build, een Info.plist, een
# icoon en een ad-hoc handtekening.
#
#   ./scripts/bundle-mac.sh          # target/bundle/FitCommunication.app + .zip
#
# Waarom een .app en niet een losse binary: de TCC-permissies (microfoon,
# schermopname) plakken aan een bundel-identiteit; een losse binary erft ze van de
# terminal die hem start en dat wil je vrienden niet uitleggen.
#
# Ad-hoc signing betekent: elke nieuwe build heeft een andere cdhash, dus macOS
# vraagt de Screen-Recording-permissie na elke update opnieuw. Developer-ID-signing
# is de oplossing; dat besluit is uitgesteld (zie docs/OVERDRACHT.md).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
uit="$root/target/bundle"
app="$uit/FitCommunication.app"
versie=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

echo "Bouwen (release, v$versie)..."
cargo build --release -p fitcom

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/fitcom" "$app/Contents/MacOS/fitcom"

# Icoon: .icns uit het bestaande PNG, met de standaardmaten die iconutil verwacht.
iconset="$uit/icon.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for maat in 16 32 128 256 512; do
  sips -z "$maat" "$maat" "$root/crates/app/icons/icon.png" \
    --out "$iconset/icon_${maat}x${maat}.png" >/dev/null
  dubbel=$((maat * 2))
  sips -z "$dubbel" "$dubbel" "$root/crates/app/icons/icon.png" \
    --out "$iconset/icon_${maat}x${maat}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/icon.icns"
rm -rf "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleIdentifier</key><string>nl.fitcommunication.app</string>
  <key>CFBundleName</key><string>FitCommunication</string>
  <key>CFBundleDisplayName</key><string>FitCommunication</string>
  <key>CFBundleExecutable</key><string>fitcom</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundleShortVersionString</key><string>$versie</string>
  <key>CFBundleVersion</key><string>$versie</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- Verplicht: zonder deze tekst crasht een gebundelde app hard zodra hij de
       microfoon aanraakt. -->
  <key>NSMicrophoneUsageDescription</key>
  <string>FitCommunication gebruikt de microfoon voor spraakgesprekken met je peers.</string>
</dict>
</plist>
PLIST

# Ad-hoc handtekening: vereist op Apple Silicon; goed genoeg voor eigen gebruik.
codesign --force --deep -s - "$app"

zipnaam="$uit/FitCommunication-$versie-macos.zip"
rm -f "$zipnaam"
ditto -c -k --keepParent "$app" "$zipnaam"

echo ""
echo "Klaar:"
echo "  $app"
echo "  $zipnaam"
echo ""
echo "Eerste start op een andere Mac: rechtsklik → Open (Gatekeeper), daarna"
echo "microfoon en schermopname toestaan in Systeeminstellingen → Privacy."
