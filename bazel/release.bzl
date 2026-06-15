"""Bazel release archive definition for bpf-linker."""

load("@tar.bzl", "tar")

def bpf_linker_release(name, binary):
    """Archives binary and its macOS dSYM directory as a zstd tar archive.

    rules_rust and rules_cc emit <binary>.dSYM; the archive preserves that
    basename.
    """
    dsym = name + "_dsym"

    native.filegroup(
        name = dsym,
        srcs = [binary],
        # The dsym is not part of the default output group, see
        # https://github.com/hermeticbuild/rules_rust/blob/23d5138a095ac094f5c928fa73e5a767d92dce78/rust/private/rustc.bzl#L2183
        output_group = "dsym_folder",
        tags = ["manual"],
    )

    # Release archives contain the binary at the archive root. macOS archives
    # also contain the dSYM directory produced by the binary target.
    # TODO(zbarsky): Confirm the dSYM archive layout, package PDB files for
    # Windows, and package split debug information for Linux. Debug information
    # should probably use a separate archive.
    srcs = [binary] + select({
        "@platforms//os:macos": [dsym],
        "//conditions:default": [],
    })

    tar(
        name = name,
        srcs = srcs,
        args = [
            "--options",
            "zstd:compression-level=22",  # Max compression.
        ],
        compress = "zstd",
        tags = ["manual"],
    )
