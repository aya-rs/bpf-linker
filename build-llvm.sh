#!/usr/bin/env sh

set -eu

LLVM_BRANCH="rustc/20.1-2025-02-13"

help() {
  echo "Usage: $0 --install-dir <INSTALL_DIR> --target <TARGET> [--source-dir <SOURCE_DIR>]"
}

INSTALL_DIR=""
SOURCE_DIR=""
TARGET=""

while [ $# -gt 0 ]; do
  case $1 in
    --help)
      help
      exit 0
      ;;
    --install-dir)
      INSTALL_DIR=$2
      shift 2
      ;;
    --source-dir)
      SOURCE_DIR=$2
      shift 2
      ;;
    --target)
      TARGET=$2
      shift 2
      ;;
    *)
      help
      exit 1
      ;;
  esac
done

if [ -z "${INSTALL_DIR}" ] || [ -z "${TARGET}" ]; then
  echo "Arguments --install-dir and --target are required."
  help
  exit 1
fi

# Fetches the LLVM source code from our fork on GitHub.
fetch_llvm_src() {
  mkdir "${SOURCE_DIR}"
  trap "rm -rf ${SOURCE_DIR}" EXIT
  curl -L --output - \
    "https://github.com/aya-rs/llvm-project/archive/${LLVM_BRANCH}.tar.gz" \
    | tar --strip-components=1 -C "${SOURCE_DIR}" -xzf -
}

# Builds LLVM for musl/Linux targets using icedragon.
build_linux_musl() {
  which icedragon || cargo install --git https://github.com/exein-io/icedragon --branch persistent-rustup
  icedragon cmake \
    --container-image ghcr.io/exein-io/icedragon:persistent-rustup \
    --target "${TARGET}" \
    -- \
    -S llvm \
    -B build \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DCMAKE_INSTALL_PREFIX=/install \
    -DLLVM_BUILD_EXAMPLES=OFF \
    -DLLVM_ENABLE_ASSERTIONS=ON \
    -DLLVM_ENABLE_BINDINGS=OFF \
    -DLLVM_ENABLE_LIBCXX=ON \
    -DLLVM_ENABLE_LIBXML2=OFF \
    -DLLVM_ENABLE_PROJECTS= \
    -DLLVM_ENABLE_RUNTIMES= \
    "-DLLVM_HOST_TRIPLE=${TARGET}" \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLLVM_TARGETS_TO_BUILD=BPF \
    -DLLVM_USE_LINKER=lld
  icedragon cmake \
    --container-image ghcr.io/exein-io/icedragon:persistent-rustup \
    --target "${TARGET}" \
    --volume "${INSTALL_DIR}:/install" \
    -- \
    --build build \
    --target install-llvm-config \
    --target install-llvm-libraries
}

# Builds LLVM for any other target. Expects CMake and all dependencies to be
# present on the host system.
build() {
  cmake \
    -S llvm \
    -B build \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    "-DCMAKE_INSTALL_PREFIX=${INSTALL_DIR}" \
    -DLLVM_BUILD_EXAMPLES=OFF \
    -DLLVM_ENABLE_ASSERTIONS=ON \
    -DLLVM_ENABLE_BINDINGS=OFF \
    -DLLVM_ENABLE_LIBXML2=OFF \
    -DLLVM_ENABLE_PROJECTS= \
    -DLLVM_ENABLE_RUNTIMES= \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLLVM_TARGETS_TO_BUILD=BPF \
    -DLLVM_USE_LINKER=lld
  cmake \
    --build build \
    --target install-llvm-config \
    --target install-llvm-libraries
}

# Create the `INSTALL_DIR` if it doesn't exist.
if [ ! -d "${INSTALL_DIR}" ]; then
  mkdir -p "${INSTALL_DIR}"
fi

# Fetch the LLVM source if `SOURCE_DIR` was not specified.
if [ -z "${SOURCE_DIR}" ]; then
  SOURCE_DIR="/tmp/aya-llvm-$(uuidgen)"
  fetch_llvm_src
fi
cd "${SOURCE_DIR}"

if [ -z "${TARGET##*musl}" ]; then
  build_linux_musl
else
  build
fi

echo "LLVM was successfully installed in ${INSTALL_DIR}"
