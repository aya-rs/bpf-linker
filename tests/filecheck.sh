#!/usr/bin/env bash

set -euo pipefail

artifact="$1"
checks="$2"
filecheck="$3"
mode="$4"
check_prefixes="$5"

input="$artifact"
if [[ "$mode" == "btf" ]]; then
  input="$TEST_TMPDIR/btf.txt"
  "$6" dump "$artifact" >"$input"
fi

args=(
  --allow-unused-prefixes
  --input-file "$input"
)
if [[ "$check_prefixes" != "-" ]]; then
  args+=(--check-prefixes "$check_prefixes")
fi

"$filecheck" "${args[@]}" "$checks"
