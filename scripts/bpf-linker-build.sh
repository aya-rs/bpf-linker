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

LLVM_REPO_DIR=$1
TARGET=$(sanitize_target ${2:-""})

TOP_DIR="$(git rev-parse --show-toplevel)"

CONTAINER_RUNTIME=${CONTAINER_RUNTIME:-$(detect_container_runtime)}
CONTAINER_NAME="bpf_linker_build"
CONTAINER_IMAGE="docker.io/ubuntu:22.04"
