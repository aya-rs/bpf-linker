#!/bin/sh

set -eux

sanitize_target() {
    local target=$1
    
    if [ -z $target ]; then
        # If no target was provided, retrieve the host triple.
        target=$(rustc --version --verbose | grep "host" | awk '{print $2}')
    fi

    if echo $target | grep -qE "aarch64.*linux.*gnu"; then
        echo "aarch64-linux-gnu"
    elif echo $target | grep -qE "aarch64.*linux.*musl"; then
        echo "aarch64-gentoo-linux-musl"
    elif echo $target | grep -qE "riscv.*linux.*gnu"; then
        echo "riscv64-linux-gnu"
    elif echo $target | grep -qE "x86_64.*linux.*gnu"; then
        echo "x86_64-linux-gnu"
    elif echo $target | grep -qE "x86_64.*linux.*musl"; then
        echo "x86_64-gentoo-linux-musl"
    else
        echo "Unsupported target: $target"
        exit 1
    fi
}

detect_container_runtime() {
    if command -v docker &> /dev/null; then
        echo "docker"
    elif command -v podman &> /dev/null; then
        echo "podman"
    else
        echo "Neither Docker nor Podman is installed."
        exit 1
    fi
}

container_image() {
    local target=$1

    if echo $target | grep -qE ".*linux.*gnu"; then
        echo "docker.io/ubuntu:22.04"
    elif echo $target | grep -qE ".*linux.*musl"; then
        echo "docker.io/gentoo/stage3:musl-llvm"
    fi
}

LLVM_REPO_DIR=$1
TARGET=$(sanitize_target ${2:-""})

TOP_DIR="$(git rev-parse --show-toplevel)"
BUILD_DIR="aya-build-$TARGET"
CONTAINER_RUNTIME=${CONTAINER_RUNTIME:-$(detect_container_runtime)}
CONTAINER_NAME="bpf_linker_llvm_build"
CONTAINER_IMAGE="docker.io/ubuntu:22.04"
C_COMPILER="clang"
CXX_COMPILER="clang++"
CXXFLAGS=""
LDFLAGS=""
SKIP_INSTALL_RPATH="OFF"
PROCESSOR=$(echo "$TARGET" | awk -F '-' '{print $1}')
ENABLE_LIBCXX="OFF"

if ! echo $TARGET | grep $(arch); then
    C_COMPILER="$TARGET-clang"
    CXX_COMPILER="$TARGET-clang++"
fi

# For musl targets:
#
# - Use Gentoo musl-llvm container image. Why Gentoo, not Alpine?
#
#   Gentoo offers crossdev, a tool for managing sysroots, which is well
#   documented[0] and creates cross wrappers for compilers which work out of
#   the box, without having to provide `--sysroot` and `--target` arguments in
#   multiple places. Gentoo musl-llvm stage3 also comes with the full LLVM
#   toolchain, CMake and Ninja, so we don't to install much. Installing
#   packages in the cross sysroot is also easy with a cross wrapper for
#   portage.
#
#   Alpine doesn't have any official way for setting up sysroots nor any
#   cross-compilation guide[1]. It ships GCC cross wrappers for `none-elf`
#   targets[2], but that toolchain doesn't come with libraries which we have to
#   link (musl, libstc++/libc++, compiler-rt etc.). Setting up a sysroot and
#   installing packages in it is a painful, manual process. So far the only
#   ways @vadorovsky was able to think of are:
#
#   - Using the `miniroot` tarball as a sysroot. Installing packages in that
#     sysroot requires using QEMU user-space emulator.
#   - Configuring APK to install packages in a sysroot. The only googlable
#     script is outdated and does not work[3].
#
# - Use LLVM libc++, compiler-rt and libunwind.
#
# [0] https://wiki.gentoo.org/wiki/Crossdev
# [1] https://wiki.alpinelinux.org/w/index.php?search=cross
# [2] https://pkgs.alpinelinux.org/package/edge/community/aarch64/gcc-aarch64-none-elf
# [3] https://gist.github.com/xentec/dbbf3cfdc3342a14000db4bedce193bd
if [[ $TARGET == *musl ]]; then
    CONTAINER_IMAGE="docker.io/gentoo/stage3:musl-llvm"
    CXXFLAGS+="-stdlib=libc++"
    LDFLAGS+="-rtlib=compiler-rt -unwindlib=libunwind -lc++ -lc++abi"
    SKIP_INSTALL_RPATH="ON"
    ENABLE_LIBCXX="ON"
fi

$CONTAINER_RUNTIME run -it --rm \
    -v "$LLVM_REPO_DIR:/root/llvm-project:z" \
    $CONTAINER_IMAGE \
    bash -c "$(cat << EOF
source /etc/profile
cd /root/llvm-project
rm -rf $BUILD_DIR
cmake -S llvm -B $BUILD_DIR -G Ninja \
      -DCMAKE_BUILD_TYPE=RelWithDebInfo \
      -DCMAKE_ASM_COMPILER=$C_COMPILER \
      -DCMAKE_C_COMPILER=$C_COMPILER \
      -DCMAKE_CXX_COMPILER=$CXX_COMPILER \
      -DCMAKE_CXX_FLAGS='$CXXFLAGS' \
      -DCMAKE_EXE_LINKER_FLAGS='$LDFLAGS' \
      -DCMAKE_SKIP_INSTALL_RPATH=$SKIP_INSTALL_RPATH \
      -DCMAKE_SHARED_LINKER_FLAGS='$LDFLAGS' \
      -DCMAKE_SYSTEM_NAME=Linux \
      -DCMAKE_SYSTEM_PROCESSOR=$PROCESSOR \
      -DLLVM_BUILD_EXAMPLES=OFF \
      -DLLVM_BUILD_STATIC=ON \
      -DLLVM_ENABLE_ASSERTIONS=ON \
      -DLLVM_ENABLE_LIBCXX=ON \
      -DLLVM_ENABLE_PROJECTS= \
      -DLLVM_ENABLE_RUNTIMES= \
      -DLLVM_HOST_TRIPLE=$TARGET \
      -DLLVM_INCLUDE_TESTS=OFF \
      -DLLVM_INCLUDE_TOOLS=ON \
      -DLLVM_INCLUDE_UTILS=OFF \
      -DLLVM_INSTALL_UTILS=OFF \
      -DLLVM_TARGETS_TO_BUILD=BPF \
      -DLLVM_USE_LINKER=lld
cmake --build $BUILD_DIR
EOF
)"
