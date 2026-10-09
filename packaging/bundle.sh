#!/usr/bin/env bash
# Build RunReCap.app (universal: Apple Silicon + Intel) into target/RunReCap.app.
#
#   packaging/bundle.sh            # ad-hoc signed, for local use
#   SIGN_ID="Developer ID Application: Your Name (TEAMID)" packaging/bundle.sh
#
# For a downloadable installer use packaging/make-dmg.sh (signs + notarizes the DMG).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="recap-app"))')
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null 2>&1 || true
  cargo build --release -p recap-app --target "$t"
done

APP=target/RunReCap.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/runrecap" \
  "target/aarch64-apple-darwin/release/runrecap" "target/x86_64-apple-darwin/release/runrecap"
sed "s/__VERSION__/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"
cp assets/fonts/Inter-LICENSE.txt "$APP/Contents/Resources/"
cp packaging/AppIcon.icns "$APP/Contents/Resources/"

if [[ -n "${SIGN_ID:-}" ]]; then
  # Developer ID: hardened runtime + secure timestamp (both required for notarization).
  codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$APP"
else
  codesign --force --options runtime --timestamp=none --sign - "$APP"
fi
echo "Built $APP ($VERSION)"
