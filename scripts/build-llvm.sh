#!/usr/bin/env bash
set -euo pipefail

# Other steps in a consumer job still need the checkout's original module.
if [[ -n ${LLVM_MODULE:-} ]]; then
  backup=$(mktemp -d)
  cp -p MODULE.bazel MODULE.bazel.lock "$backup"
  trap 'mv "$backup"/MODULE.bazel{,.lock} .; rmdir "$backup"' EXIT
  printf '%s' "$LLVM_MODULE" | base64 -d > MODULE.bazel
fi

# These archives must contain native objects, not version-specific LLVM
# bitcode, so linkers using an older LLVM can read them.
bazel build \
  --config=release \
  --config=remote \
  --features=-thin_lto \
  --remote_timeout=120 \
  --remote_header=x-buildbuddy-api-key="$BUILDBUDDY_API_KEY" \
  "$@"
