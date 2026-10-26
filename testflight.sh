#!/usr/bin/env bash
# Ship the iOS app to TestFlight. See ios/scripts/testflight.sh --help.
exec "$(dirname "$0")/ios/scripts/testflight.sh" "$@"
