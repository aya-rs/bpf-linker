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
        # The dsym is not part of the default output group.
        output_group = "dsym_folder",
        tags = ["manual"],
    )

    # Release archives contain the binary at the archive root. macOS archives
    # also contain the dSYM directory produced by the binary target.
    # TODO(zbarsky): We should confirm this works and also package pdb for Windows
    # and split debuginfo for linux. We probably want a separate debuginfo archive also.
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
        include_runfiles = False,
        tags = ["manual"],
    )
