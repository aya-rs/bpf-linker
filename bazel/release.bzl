"""Bazel release archive definition for bpf-linker."""

load("@tar.bzl", "mtree_spec", "tar")

def bpf_linker_release(name, binary):
    """Archives binary and its macOS dSYM directory as a zstd tar archive.

    rules_rust and rules_cc emit <binary>.dSYM; the archive preserves that basename.
    """
    dsym_name = name + "_dsym"
    mtree_name = name + "_mtree"

    native.filegroup(
        name = dsym_name,
        srcs = [binary],
        output_group = "dsym_folder",
        tags = ["manual"],
    )

    # Release archives contain the binary at the archive root. macOS archives
    # also contain the dSYM directory produced by the binary target.
    srcs = [binary] + select({
        "@platforms//os:macos": [dsym_name],
        "//conditions:default": [],
    })

    mtree_spec(
        name = mtree_name,
        srcs = srcs,
        include_runfiles = False,
        tags = ["manual"],
    )

    tar(
        name = name,
        srcs = srcs,
        args = [
            "--options",
            "zstd:compression-level=22",
        ],
        compress = "zstd",
        mtree = mtree_name,
        tags = ["manual"],
    )
