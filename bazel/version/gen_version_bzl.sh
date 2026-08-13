#!/bin/bash
# Generate version.bzl from Cargo.toml
# Usage: gen_version_bzl.sh > version.bzl

CARGO_TOML="$(dirname "$0")/../..//Cargo.toml"
VERSION=$(grep '^version' "$CARGO_TOML" | sed 's/version = "\([^"]*\)"/\1/')

cat <<EOF
"""Version constants for bpf-linker.

WARNING: This file is auto-generated from Cargo.toml.
Do not edit manually - run gen_version_bzl.sh instead.
"""

BPF_LINKER_VERSION = "$VERSION"
EOF
