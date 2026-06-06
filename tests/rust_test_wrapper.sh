#!/usr/bin/env bash

set -euo pipefail

# Avoid Bazel's host-derived generate-xml.sh fallback after remote Linux tests.
# The wrapped Rust test still provides the real output and exit status.
printf '<testsuites tests="0" failures="0" errors="0"></testsuites>\n' >"$XML_OUTPUT_FILE"

binary="$1"
shift
if [[ "$binary" != /* ]]; then
  binary="$PWD/$binary"
fi

exec "$binary" "$@"
