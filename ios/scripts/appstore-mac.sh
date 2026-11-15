#!/usr/bin/env bash
# The Mac App Store screenshots: six PNGs at exactly 2880 × 1800 px, drawn by the app from the demo's sample data
# (no network, no personal data, no demo banner), dark theme, ready to drag into App Store Connect.
#
#   ./appstore-mac.sh        build the Mac app (Debug, unsigned) and write ios/build/appstore-mac/*.png
#
# Needs no team or signing: the unsigned Debug build runs straight from its build folder.
set -euo pipefail

cd "$(dirname "$0")/.."

OUT=build/appstore-mac
DERIVED=build/appstore-mac-derived
LOG="$DERIVED/xcodebuild.log"
mkdir -p "$DERIVED"
rm -rf "$OUT" && mkdir -p "$OUT"

echo "Building the Mac app…"
if ! xcodebuild -project Loft.xcodeproj -scheme LoftMac -configuration Debug -destination 'platform=macOS' \
     -derivedDataPath "$DERIVED" -IDEBuildLocationStyle=Unique CODE_SIGNING_ALLOWED=NO build > "$LOG" 2>&1; then
  grep -E "error:" "$LOG" | head -20 >&2
  echo "Build failed; see ios/$LOG" >&2
  exit 1
fi

echo "Drawing the screenshots…"
# The arguments are read for this launch only, so nothing changes the app's saved settings.
"$DERIVED/Build/Products/Debug/Loft.app/Contents/MacOS/Loft" -demo -render-app-store "$PWD/$OUT" -fixture-specs \
  -appearance dark -boardType all -boardShowClosed NO -boardShowDropped NO -ApplePersistenceIgnoreState YES > /dev/null 2>&1 &
pid=$!
for _ in $(seq 1 90); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
if kill -0 "$pid" 2>/dev/null; then kill "$pid"; echo "The app didn't finish within 90 s." >&2; exit 1; fi

shots=("$OUT"/*.png)
for f in "${shots[@]}"; do
  size=$(sips -g pixelWidth -g pixelHeight "$f" | awk '/pixel/{print $2}' | paste -sd x -)
  [[ "$size" == 2880x1800 ]] || { echo "$f is $size, not 2880x1800" >&2; exit 1; }
done
[[ ${#shots[@]} -eq 6 ]] || { echo "Expected 6 screenshots, got ${#shots[@]}" >&2; exit 1; }
echo "Done: ${#shots[@]} screenshots at 2880 × 1800 in ios/$OUT"
[[ -t 1 ]] && open "$OUT" || true
