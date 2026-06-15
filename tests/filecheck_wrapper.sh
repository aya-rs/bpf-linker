#!/usr/bin/env bash

set -euo pipefail

input="$ARTIFACT"
if [[ "$MODE" == "btf" ]]; then
  input="$TEST_TMPDIR/btf.txt"
  "$BTFDUMP" dump "$ARTIFACT" >$input
fi

args=(
  --allow-unused-prefixes
  --input-file "$input"
)
if [[ "$CHECK_PREFIXES" != "-" ]]; then
  args+=(--check-prefixes "$CHECK_PREFIXES")
fi

"$FILECHECK" "${args[@]}" "$CHECKS"
