#!/bin/sh

# Creates a sysroot based on alpine-minirootfs that can be used to
# cross-compile LLVM and bpf-linker for musl targets.

set -eu

# Send all script output to stderr, except for the final variable emission.
exec 3>&1
exec 1>&2

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "Usage: $0 <architecture> <destination-directory> [<rust_version>]" >&2
  exit 1
fi

ARCH="$1"
DEST_DIR="$2"
RUST_VERSION="${3:-}"

BASE="https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/${ARCH}"
YAML="${BASE}/latest-releases.yaml"
if ! YAML_CONTENT="$(curl -fsSL "$YAML")"; then
  echo "Failed to download Alpine release manifest: ${YAML}" >&2
  exit 1
fi

RELEASES=$(printf '%s\n' "$YAML_CONTENT" \
  | grep -oE "alpine-minirootfs-[0-9]+\.[0-9]+\.[0-9]+-${ARCH}\.tar\.gz" || true)
if [ -z "$RELEASES" ]; then
  echo "Could not find any minirootfs archives for architecture ${ARCH} in" \
       "Alpine release manifest: ${YAML}: ${YAML_CONTENT}" >&2
  exit 1
fi

FNAME=$(printf '%s\n' "$RELEASES" | sort -Vu | tail -n1)
if [ -z "$FNAME" ]; then
  echo "Could not determine Alpine minirootfs archive name from Alpine " \
       "release manifest ${YAML}: ${YAML_CONTENT}" >&2
  exit 1
fi
MINIROOTFS_URL="${BASE}/${FNAME}"

mkdir -p "${DEST_DIR}"
curl -L --fail "${MINIROOTFS_URL}" | \
  tar -xpzf - --xattrs-include='*.*' --numeric-owner -C "${DEST_DIR}"

# Construct and set the `BPF_LINKER_SYSROOT_<arch>_LINUX_MUSL` variable needed
# by the `*-run` script.
ARCH_UPPER=$(printf '%s' "$ARCH" | tr '[:lower:]' '[:upper:]')
sysroot_var_name="BPF_LINKER_SYSROOT_${ARCH_UPPER}_LINUX_MUSL"
sysroot_var_value=$(realpath "${DEST_DIR}")
sysroot_var="${sysroot_var_name}=${sysroot_var_value}"
export "${sysroot_var}"

# Install necessary dependencies:
#
# - Clang, llvm-test-utils (that provides FileCheck), Rust toolchain (installed
#   via rustup) and btfdump are needed by compile tests. It's important that
#   they use an isolated musl-compatible toolchain instead of the host one.
# - Zlib and zstd are needed to build LLVM from source.
WRAPPER_DIR=$(dirname -- "$0")

"${WRAPPER_DIR}/${ARCH}-linux-musl-run" /bin/sh -l <<EOF
set -eu
apk update
apk add \
  clang \
  lld \
  llvm-test-utils \
  musl-dev \
  zlib-dev \
  zlib-static \
  zstd-dev \
  zstd-static
# Only install the toolchain and btfdump when a Rust version is provided.
if [ -n "$RUST_VERSION" ]; then
  apk add rustup
  rustup-init -y --default-toolchain "$RUST_VERSION" --component rust-src
  . "\$HOME/.cargo/env"
  cargo install btfdump
fi
EOF

# Emit the sysroot variable to the caller.
printf '%s\n' "${sysroot_var}" >&3

# Construct and emit the `CARGO_TARGET_<TARGET>_LINKER` and
# `CARGO_TARGET_<TARGET>_RUNNER` variables needed for cross builds.
WRAPPER_DIR_ABS=$(realpath "${WRAPPER_DIR}")
printf '%s\n' "CARGO_TARGET_${ARCH_UPPER}_UNKNOWN_LINUX_MUSL_LINKER=${WRAPPER_DIR_ABS}/${ARCH}-linux-musl-clang" >&3
printf '%s\n' "CARGO_TARGET_${ARCH_UPPER}_UNKNOWN_LINUX_MUSL_RUNNER=${WRAPPER_DIR_ABS}/${ARCH}-linux-musl-run" >&3
