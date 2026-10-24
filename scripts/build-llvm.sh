#!/usr/bin/env bash
set -euo pipefail

# Restore the checkout's original module before the consumer runs Cargo.
: "${LLVM_MODULE:?missing the selected LLVM module}"
backup=$(mktemp -d)
cp -p MODULE.bazel MODULE.bazel.lock "$backup"
trap 'mv "$backup"/MODULE.bazel{,.lock} .; rmdir "$backup"' EXIT
printf '%s' "$LLVM_MODULE" | base64 -d > MODULE.bazel

# These archives must contain native objects, not version-specific LLVM
# bitcode, so linkers using an older LLVM can read them.
bazel build \
  --config=release \
  --config=remote \
  --features=-thin_lto \
  --remote_timeout=120 \
  --remote_header=x-buildbuddy-api-key="$BUILDBUDDY_API_KEY" \
  "$@"
