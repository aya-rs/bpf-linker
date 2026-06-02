#!/usr/bin/env bash

set -euxo pipefail

bazel_args=(
    --config=release
    --config=remote
)
if [[ -n "${BUILDBUDDY_API_KEY:-}" ]]; then
    bazel_args+=(--remote_header=x-buildbuddy-api-key="$BUILDBUDDY_API_KEY")
fi

bazel build //:release-archives "${bazel_args[@]}"

mkdir -p dist

while IFS= read -r src; do
    relative="${src#bazel-out/}"
    target="${relative%%-opt/*}"
    install -m 0644 "$src" "dist/bpf-linker-${target}.tar.zst"
done < <(bazel cquery //:release-archives --output=files "${bazel_args[@]}")
