#!/bin/sh
# Measures the on-device model's jobs on this Mac (macOS 26+ with Apple Intelligence on), with the same
# job code the app uses. Prints each miss, the pass rate and latency; fails if a set is below its minimum.
#   ios/Evaluations/run.sh                      # every set in cases/
#   ios/Evaluations/run.sh cases/yes_no.json    # one set
# Sets about your own agents (the route set, with local/agents.json) go in local/ (gitignored), and run too.
set -e
cd "$(dirname "$0")"
out="${TMPDIR:-/tmp}/loft-evaluations"
app=../Loft
swiftc -O -parse-as-library -o "$out" main.swift Pipelines.swift $app/Core/AI/OnDeviceJobs.swift $app/Core/AI/PermissionExplainer.swift $app/Core/AI/GrillUnderstanding.swift \
    $app/Core/Voice/GrillMatcher.swift $app/Core/Voice/VoiceText.swift $app/Domain/*.swift
if [ $# -eq 0 ]; then set -- cases/*.json $(ls local/cases/*.json 2>/dev/null); fi
"$out" "$@"
