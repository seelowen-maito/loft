#!/usr/bin/env bash
# Archive the app and upload it to TestFlight: what Xcode's Product → Archive → Distribute does, in one command.
#
# Signs and uploads with the Apple account signed in to Xcode (Settings → Accounts), so no API key
# is needed. For headless runs (CI), set ASC_KEY_PATH, ASC_KEY_ID and ASC_ISSUER_ID to an
# App Store Connect API key instead.
#
#   ./testflight.sh               tests, archive, upload
#   ./testflight.sh --fast        skip the tests
#   ./testflight.sh --no-upload   archive and export an .ipa to ios/build/testflight/export
#   VERSION=0.2.0 ./testflight.sh set the version shown in TestFlight (default: the project's)
#
# Needs LOFT_TEAM_ID and LOFT_BUNDLE_ID, in ios/Config/Local.xcconfig or the environment.
set -euo pipefail

cd "$(dirname "$0")/.."

for arg in "$@"; do
  case "$arg" in
    --fast) SKIP_TESTS=1 ;;
    --no-upload) UPLOAD=0 ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $arg (see --help)" >&2; exit 2 ;;
  esac
done

# Every upload needs a new build number; a UTC timestamp always increases.
BUILD_NUMBER="${BUILD_NUMBER:-$(date -u +%Y%m%d%H%M)}"
OUT=build/testflight
ARCHIVE="$OUT/Loft.xcarchive"
LOG="$OUT/xcodebuild.log"
mkdir -p "$OUT"
: > "$LOG"

# Build settings passed only when set, so the project's own values apply otherwise.
settings=(CURRENT_PROJECT_VERSION="$BUILD_NUMBER")
[[ -n "${VERSION:-}" ]] && settings+=(MARKETING_VERSION="$VERSION")

# Always has one element, so it's safe to expand under `set -u` on macOS's bash 3.2.
auth=(-allowProvisioningUpdates)
if [[ -n "${ASC_KEY_PATH:-}" ]]; then
  auth+=(-authenticationKeyPath "$ASC_KEY_PATH" -authenticationKeyID "${ASC_KEY_ID:?}" -authenticationKeyIssuerID "${ASC_ISSUER_ID:?}")
fi

# Runs a step quietly; on failure shows the end of the log instead of thousands of lines.
step() {
  local title="$1"; shift
  echo "▸ $title"
  if ! "$@" >>"$LOG" 2>&1; then
    echo "✗ $title failed. Last lines of $PWD/$LOG:" >&2
    grep -E "error:|Error|failed" "$LOG" | tail -15 >&2 || tail -30 "$LOG" >&2
    exit 1
  fi
}

# The signing ids live outside the repo (Config/Signing.xcconfig); check they're set before a long build.
ids=$(xcodebuild -project Loft.xcodeproj -target Loft -configuration Release -showBuildSettings 2>/dev/null)
team=$(awk -F' = ' '$1 ~ /^ +DEVELOPMENT_TEAM$/ {print $2}' <<<"$ids")
bundle=$(awk -F' = ' '$1 ~ /^ +PRODUCT_BUNDLE_IDENTIFIER$/ {print $2}' <<<"$ids")
if [[ -z "$team" || "$bundle" == com.example.* ]]; then
  echo "Set LOFT_TEAM_ID and LOFT_BUNDLE_ID in ios/Config/Local.xcconfig (see Local.xcconfig.example) or the environment." >&2
  exit 2
fi

started=$(date +%s)

if [[ "${SKIP_TESTS:-0}" != 1 ]]; then
  sim=$(xcrun simctl list devices available -j | python3 -c 'import json,sys; d=json.load(sys.stdin)["devices"]; print(next(x["udid"] for k,v in sorted(d.items(), reverse=True) if "iOS" in k for x in v if x["name"].startswith("iPhone")))')
  # A test that hangs (e.g. waiting on a speech model) fails after 2 minutes instead of blocking the release.
  step "Running tests" xcodebuild test -project Loft.xcodeproj -scheme Loft -destination "id=$sim" -derivedDataPath build/test \
    -test-timeouts-enabled YES -default-test-execution-time-allowance 120 -maximum-test-execution-time-allowance 180
fi

rm -rf "$ARCHIVE"
step "Archiving build $BUILD_NUMBER" xcodebuild archive -project Loft.xcodeproj -scheme Loft -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$ARCHIVE" "${auth[@]}" "${settings[@]}"

options=Config/ExportOptions.plist
if [[ "${UPLOAD:-1}" == 0 ]]; then
  options="$OUT/ExportOptions-local.plist"
  sed 's#<string>upload</string>#<string>export</string>#' Config/ExportOptions.plist > "$options"
  step "Exporting the .ipa" xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportOptionsPlist "$options" \
    -exportPath "$OUT/export" "${auth[@]}"
  echo "✓ Build $BUILD_NUMBER exported to $PWD/$OUT/export ($(( $(date +%s) - started ))s)"
  exit 0
fi

step "Uploading to App Store Connect" xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportOptionsPlist "$options" \
  -exportPath "$OUT/export" "${auth[@]}"

version=$(/usr/libexec/PlistBuddy -c 'Print :ApplicationProperties:CFBundleShortVersionString' "$ARCHIVE/Info.plist")
echo "✓ Loft $version ($BUILD_NUMBER) uploaded in $(( $(date +%s) - started ))s."
echo "  It appears in TestFlight after Apple processes it (usually 5–15 min): https://appstoreconnect.apple.com/apps"
osascript -e "display notification \"Loft $version ($BUILD_NUMBER) uploaded to TestFlight\" with title \"Loft\"" 2>/dev/null || true
