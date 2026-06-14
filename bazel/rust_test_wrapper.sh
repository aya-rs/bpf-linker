#!/usr/bin/env bash

set -euo pipefail

# Write a dummy junit.xml; Bazel otherwise generates an extra action to run generate-xml.sh after the test.
# That fallback does not work properly when a Windows host uses Linux remote exec.
# This can be removed once https://github.com/dzbarsky/bazel/pull/1 and its prerequisite changes land.
printf '<testsuites tests="0" failures="0" errors="0"></testsuites>\n' >"$XML_OUTPUT_FILE"

binary="$1"
shift
if [[ "$binary" != /* ]]; then
  binary="$PWD/$binary"
fi

exec "$binary" "$@"
