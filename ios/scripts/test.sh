#!/usr/bin/env bash
# Runs the iOS tests fast: build once, then the unit tests, then (only if they pass) the UI tests, each test with
# a time limit, and prints every test's time, slowest first.
#
#   ios/scripts/test.sh                      # build, unit tests, then UI tests
#   ios/scripts/test.sh --unit               # unit tests only (LoftTests, target: under 1 min)
#   ios/scripts/test.sh --ui                 # UI tests only (LoftUITests, target: under 2 min)
#   ios/scripts/test.sh --only LoftTests/StoreTests --only LoftUITests/LoftUITests/testOverview
#   ios/scripts/test.sh --no-build --unit    # reuse the last build (nothing changed since)
#   ios/scripts/test.sh --no-build --ui --only LoftUITests/LoftUITests/testOverview   # rerun one, no build
#
# Environment:
#   SIMULATOR=<udid or name>   the Simulator, already booted (default: the first booted iPhone). The script never
#                              creates, boots or erases one: whoever schedules test runs owns the Simulators.
#   DERIVED_DATA=<dir>         keep builds here between runs (default: ios/build)
#   RESULT=<dir>               write one .xcresult bundle per suite here (default: none)
#   EXIT_GRACE=<seconds>       stop xcodebuild this long after the tests finish if it hasn't exited (default 60)
#   xcodebuild's full output for each suite is kept in $DERIVED_DATA/test-logs/<suite>.log.
#   DEVELOPER_DIR=<Xcode>/Contents/Developer   which Xcode; found automatically when xcode-select points at the
#                              Command Line Tools
#
# Each test gets 60 s (Xcode rounds time limits up to whole minutes, so 30 s isn't possible). Tests run one at a
# time on the one Simulator, never on clones. UI tests are heavy: run one UI suite at a time on this Mac.
set -euo pipefail
self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
cd "$(dirname "$self")/.."

build=1
suites=(LoftTests LoftUITests)
only=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) build=0 ;;
    --unit) suites=(LoftTests) ;;
    --ui) suites=(LoftUITests) ;;
    --only) shift; only+=("-only-testing:$1") ;;
    -h|--help) sed -n '2,24p' "$self"; exit 0 ;;
    *) echo "Unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

# xcodebuild needs Xcode, not the Command Line Tools.
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p 2>/dev/null)" == *CommandLineTools* ]]; then
  for x in /Applications/Xcode.app /Applications/Xcode*.app /Volumes/*/Xcode.app /Volumes/*/xcode/Xcode.app; do
    if [[ -d "$x/Contents/Developer" ]]; then export DEVELOPER_DIR="$x/Contents/Developer"; break; fi
  done
  [[ -n "${DEVELOPER_DIR:-}" ]] || { echo "xcode-select points at the Command Line Tools and no Xcode was found; set DEVELOPER_DIR." >&2; exit 1; }
  echo "▸ Using $DEVELOPER_DIR"
fi

derived="${DERIVED_DATA:-build}"
sim="${SIMULATOR:-$(xcrun simctl list devices booted | grep -m1 -E 'iPhone' | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/' || true)}"
if [[ -z "$sim" ]]; then echo "No booted iPhone Simulator; boot one or set SIMULATOR." >&2; exit 1; fi
[[ "$sim" =~ ^[0-9A-F-]{36}$ ]] && dest="platform=iOS Simulator,id=$sim" || dest="platform=iOS Simulator,name=$sim"
common=(-project Loft.xcodeproj -scheme Loft -destination "$dest" -derivedDataPath "$derived")
limits=(-parallel-testing-enabled NO -test-timeouts-enabled YES
        -default-test-execution-time-allowance 60 -maximum-test-execution-time-allowance 60)
# xcodebuild's full output per suite stays here for a look after the run.
log_dir="$derived/test-logs"
mkdir -p "$log_dir"

if [[ $build -eq 1 ]]; then
  echo "▸ Building for testing ($dest, derived data in $derived)"
  b=$(date +%s)
  xcodebuild build-for-testing "${common[@]}" -quiet
  echo "▸ Built in $(( $(date +%s) - b ))s"
fi

# Prints each test's time from xcodebuild's "Test Case '-[Class test]' passed (1.234 seconds)." lines, slowest first.
timings() {
  sed -nE "s/^Test Case '-\[[A-Za-z]+\.([A-Za-z]+) ([A-Za-z0-9_]+)\]' (passed|failed|skipped) \(([0-9.]+) seconds\).*/\4 \3 \1.\2/p" "$1" \
    | sort -rn | awk '{ printf "  %6.1fs  %-7s %s\n", $1, $2, $3 }'
}

status=0
for suite in "${suites[@]}"; do
  sel=()
  if [[ ${#only[@]} -gt 0 ]]; then
    for o in "${only[@]}"; do [[ "$o" == "-only-testing:$suite"* ]] && sel+=("$o"); done
    [[ ${#sel[@]} -gt 0 ]] || continue
  else
    sel=("-only-testing:$suite")
  fi
  result=()
  if [[ -n "${RESULT:-}" ]]; then mkdir -p "$RESULT"; rm -rf "$RESULT/$suite.xcresult"; result=(-resultBundlePath "$RESULT/$suite.xcresult"); fi
  log="$log_dir/$suite.log"
  echo "▸ $suite (load $(sysctl -n vm.loadavg | awk '{print $2}'))"
  s=$(date +%s)
  suite_status=0
  xcodebuild test-without-building "${common[@]}" "${limits[@]}" "${sel[@]}" ${result[@]+"${result[@]}"} > "$log" 2>&1 &
  pid=$!
  # xcodebuild has sat for ~10 minutes after the last test had finished (#112, #119 runs: 240 s of tests,
  # 845 s in all). Once the suite's result line is out, give it EXIT_GRACE seconds (default 60) to exit, then
  # stop it and go by the log. A RESULT bundle may then be incomplete; the log and timings are not.
  finished_at=
  while kill -0 "$pid" 2> /dev/null; do
    if [[ -z "$finished_at" ]] && grep -qE "^Test Suite '(All|Selected) tests' (passed|failed)" "$log"; then
      finished_at=$(date +%s)
    fi
    if [[ -n "$finished_at" && $(( $(date +%s) - finished_at )) -ge ${EXIT_GRACE:-60} ]]; then
      echo "▸ xcodebuild was still running ${EXIT_GRACE:-60} s after the tests finished; stopped it (full log: $log)"
      kill "$pid" 2> /dev/null || true
      break
    fi
    sleep 1
  done
  wait "$pid" 2> /dev/null || suite_status=$?
  if [[ -n "$finished_at" && $suite_status -ne 0 ]]; then
    # Stopped (or exited badly) after the tests ran: their own result line decides.
    grep -qE "^Test Suite '(All|Selected) tests' passed" "$log" && suite_status=0 || suite_status=1
  fi
  took=$(( $(date +%s) - s ))
  timings "$log"
  if [[ $suite_status -ne 0 ]]; then
    grep -E "error: |Test Case .* failed|timed out|\*\* TEST" "$log" | head -40 || true
    echo "▸ $suite FAILED in ${took}s"
    status=$suite_status
    break  # UI tests only run once the unit tests pass.
  fi
  target=$([[ $suite == LoftTests ]] && echo 60 || echo 120)
  echo "▸ $suite passed in ${took}s$([[ $took -gt $target ]] && echo " (over the ${target}s target)")"
done
exit $status
