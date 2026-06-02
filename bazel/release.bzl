load("@tar.bzl", "mtree_spec", "tar")

def bpf_linker_release(name, binary):
    dsym_name = name + "_dsym"
    mtree_name = name + "_mtree"

    native.filegroup(
        name = dsym_name,
        srcs = [binary],
        output_group = "dsym_folder",
        tags = ["manual"],
    )

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
