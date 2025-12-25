#!/usr/bin/env bash
#
# Print the Rust CI URL for the `rust-dev` tarball matching the current `rustc`
# toolchain and the given target triple.
#
# This mirrors rust-lang/rust bootstrap naming:
# - nightly: rust-dev-nightly-<target>.tar.xz
# - beta:    rust-dev-beta-<target>.tar.xz
# - stable:  rust-dev-<version>-<target>.tar.xz
#
# Usage:
#   scripts/rust-ci-rust-dev-url.sh --target <triple>
#
set -euo pipefail

target_triple=""

usage() {
  cat <<'EOF' >&2
Usage: scripts/rust-ci-rust-dev-url.sh --target <triple>

Prints the https://ci-artifacts.rust-lang.org `rust-dev` URL for the current
`rustc` (as reported by `rustc -Vv`).

Options:
  --target <triple>  Rust target triple (e.g. x86_64-unknown-linux-gnu)

Environment:
  RUSTUP_TOOLCHAIN  Optional; if set, rustup will select that toolchain when
                    running rustc via rustup shims.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      target_triple="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$target_triple" ]]; then
  usage
  exit 2
fi

if ! command -v rustc >/dev/null 2>&1; then
  echo "error: rustc not found in PATH" >&2
  exit 1
fi

rustc_info="$(rustc -Vv)"
rustc_sha="$(printf '%s\n' "$rustc_info" | awk '/^commit-hash: /{print $2}')"
rustc_release="$(printf '%s\n' "$rustc_info" | awk -F': ' '/^release: /{print $2}')"
if [[ -z "$rustc_sha" || -z "$rustc_release" ]]; then
  echo "error: failed to parse rustc version info" >&2
  printf '%s\n' "$rustc_info" >&2
  exit 1
fi

if [[ "$rustc_release" == *-nightly ]]; then
  rust_dev_version_part=nightly
elif [[ "$rustc_release" == *-beta* ]]; then
  rust_dev_version_part=beta
else
  rust_dev_version_part="$rustc_release"
fi

url="https://ci-artifacts.rust-lang.org/rustc-builds/${rustc_sha}/rust-dev-${rust_dev_version_part}-${target_triple}.tar.xz"

printf '%s\n' "$url"
