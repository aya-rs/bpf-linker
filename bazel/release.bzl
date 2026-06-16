"""Bazel release archive definition for bpf-linker."""

load("@bazel_lib//lib:transitions.bzl", "platform_transition_filegroup")
load("@tar.bzl", "tar")
load("@with_cfg.bzl", "with_cfg")

release_linux_filegroup, _release_linux_filegroup_internal = with_cfg(platform_transition_filegroup).set(
    "fission",
    ["opt"],
).build()

release_macos_filegroup, _release_macos_filegroup_internal = with_cfg(platform_transition_filegroup).set(
    "apple_generate_dsym",
    True,
).build()

release_windows_filegroup, _release_windows_filegroup_internal = with_cfg(platform_transition_filegroup).extend(
    "features",
    ["generate_pdb_file"],
).build()

def _archive_input_impl(ctx):
    return DefaultInfo(files = ctx.attr.src[DefaultInfo].files)

_archive_input = rule(
    implementation = _archive_input_impl,
    attrs = {
        "src": attr.label(mandatory = True),
    },
)

def bpf_linker_release(name, binary):
    """Archives the binary and debug information as separate zstd archives."""
    binary_archive_input = name + "_binary_archive_input"
    debug_info = name + "_debug_info"

    _archive_input(
        name = binary_archive_input,
        src = binary,
        tags = ["manual"],
    )

    native.filegroup(
        name = debug_info,
        srcs = [binary],
        # The dsym is not part of the default output group, see
        # https://github.com/hermeticbuild/rules_rust/blob/23d5138a095ac094f5c928fa73e5a767d92dce78/rust/private/rustc.bzl#L2183
        output_group = select({
            "@platforms//os:linux": "dwp_file",
            "@platforms//os:macos": "dsym_folder",
            "@platforms//os:windows": "pdb_file",
        }),
        tags = ["manual"],
    )

    tar(
        name = name,
        srcs = [binary_archive_input],
        args = [
            "--options",
            "zstd:compression-level=22",  # Max compression.
        ],
        compress = "zstd",
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
