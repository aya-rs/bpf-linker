#!/usr/bin/env bash

set -euo pipefail

printf '<testsuites tests="0" failures="0" errors="0"></testsuites>\n' >"$XML_OUTPUT_FILE"

binary="$1"
shift
if [[ "$binary" != /* ]]; then
  binary="$PWD/$binary"
fi

exec "$binary" "$@"
