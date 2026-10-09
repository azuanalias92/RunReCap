#!/usr/bin/env bash
# Build target/RunReCap-<version>.dmg: a drag-to-Applications installer.
#
#   packaging/make-dmg.sh
#       Ad-hoc signed. Works, but users confirm once in Privacy & Security
#       (instructions are included in the DMG).
#
#   RUNRECAP_UPDATE_URL=https://github.com/<you>/<repo>/releases/latest/download/latest.json \
#   NOTES="What's new" packaging/make-dmg.sh
#       Bakes the update feed into the app and writes target/latest.json.
#       Upload BOTH the DMG and latest.json to the release. The app checks the
#       feed daily and offers "Update to x.y.z" (download, verify, reinstall).
#
#   SIGN_ID="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=runrecap packaging/make-dmg.sh
#       Signed + notarized + stapled: opens with no warnings.
#       One-time setup for the profile:
#         xcrun notarytool store-credentials runrecap --apple-id you@example.com \
#           --team-id TEAMID --password <app-specific password>
set -euo pipefail
cd "$(dirname "$0")/.."

# Releases live on GitHub; `latest/download/` always points at the newest release.
export RUNRECAP_UPDATE_URL="${RUNRECAP_UPDATE_URL:-https://github.com/azuanalias92/RunReCap/releases/latest/download/latest.json}"

packaging/bundle.sh

VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" target/RunReCap.app/Contents/Info.plist)
STAGE=target/dmg-stage
DMG="target/RunReCap-$VERSION.dmg"

rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R target/RunReCap.app "$STAGE/"
ln -s /Applications "$STAGE/Applications"
if [[ -z "${NOTARY_PROFILE:-}" ]]; then
  cp packaging/dmg-readme.txt "$STAGE/If RunReCap won't open.txt"
fi

hdiutil create -quiet -volname "RunReCap $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
rm -rf "$STAGE"

if [[ -n "${SIGN_ID:-}" ]]; then
  codesign --force --timestamp --sign "$SIGN_ID" "$DMG"
fi
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
  [[ -n "${SIGN_ID:-}" ]] || { echo "NOTARY_PROFILE needs SIGN_ID (Developer ID Application)" >&2; exit 1; }
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

hdiutil verify -quiet "$DMG"
SHA=$(shasum -a 256 "$DMG" | cut -d' ' -f1)

# Release feed for in-app updates; `url` is relative to the feed's location.
VERSION="$VERSION" DMG_NAME="$(basename "$DMG")" SHA="$SHA" NOTES="${NOTES:-}" python3 -c '
import json, os
feed = {k: os.environ[e] for k, e in [("version", "VERSION"), ("url", "DMG_NAME"), ("sha256", "SHA"), ("notes", "NOTES")]}
print(json.dumps(feed, indent=2))' > target/latest.json

echo "Built $DMG ($(du -h "$DMG" | cut -f1))"
echo "sha256 $SHA"
echo "Feed   target/latest.json (upload it together with the DMG)"
if [[ -z "${RUNRECAP_UPDATE_URL:-}" ]]; then
  echo "Note: built without RUNRECAP_UPDATE_URL, so this version can't check for updates on its own."
fi
