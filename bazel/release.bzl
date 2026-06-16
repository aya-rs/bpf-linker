"""Bazel release archive definition for bpf-linker."""

load("@bazel_lib//lib:transitions.bzl", "platform_transition_filegroup")
load("@tar.bzl", "tar")
load("@with_cfg.bzl", "with_cfg")

# https://bazel.build/reference/command-line-reference#flag--fission
release_linux_filegroup, _release_linux_filegroup_internal = with_cfg(platform_transition_filegroup).set(
    "fission",
    ["opt"],
).build()

# https://bazel.build/reference/command-line-reference#flag--apple_generate_dsym
release_macos_filegroup, _release_macos_filegroup_internal = with_cfg(platform_transition_filegroup).set(
    "apple_generate_dsym",
    True,
).build()

# https://bazel.build/reference/command-line-reference#flag--features
# rules_cc checks generate_pdb_file before declaring the PDB output:
# https://github.com/bazelbuild/rules_cc/blob/0.2.19/cc/private/rules_impl/cc_binary_impl.bzl#L611-L616
release_windows_filegroup, _release_windows_filegroup_internal = with_cfg(platform_transition_filegroup).extend(
    "features",
    ["generate_pdb_file"],
).build()

def bpf_linker_release(name, binary):
    """Archives the binary and debug information as separate zstd archives."""
    debug_info = name + "_debug_info"

    native.filegroup(
        name = debug_info,
        srcs = [binary],
        # rules_rust creates these output groups:
        # dwp_file: https://github.com/hermeticbuild/rules_rust/blob/8a2219c1fcf2070120c26dc67eb8aeacfc5a3819/rust/private/rustc.bzl#L2083-L2104
        # dsym_folder: https://github.com/hermeticbuild/rules_rust/blob/8a2219c1fcf2070120c26dc67eb8aeacfc5a3819/rust/private/rustc.bzl#L1901-L1909
        # pdb_file: https://github.com/hermeticbuild/rules_rust/blob/8a2219c1fcf2070120c26dc67eb8aeacfc5a3819/rust/private/rustc.bzl#L2059-L2062
        output_group = select({
            "@platforms//os:linux": "dwp_file",
            "@platforms//os:macos": "dsym_folder",
            "@platforms//os:windows": "pdb_file",
        }),
        tags = ["manual"],
    )

    tar(
        name = name,
        srcs = [binary],
        args = [
            "--options",
            "zstd:compression-level=22",  # Max compression.
        ],
        compress = "zstd",
        include_runfiles = False,
        tags = ["manual"],
    )

    tar(
        name = name + "-debuginfo",
        srcs = [debug_info],
        args = [
            "--options",
            "zstd:compression-level=22",  # Max compression.
        ],
        compress = "zstd",
        tags = ["manual"],
    )
