"""Package the Bazel-built LLVM libraries for use by Cargo builds."""

load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")
load("@tar.bzl", "tar")

def _mtree_line(path, type, content = None):
    line = "{} uid=0 gid=0 time=1672560000 mode=0755 type={} nlink=1".format(
        path.replace(" ", "\\040"),
        type,
    )
    if content:
        line += " content={}".format(content.replace(" ", "\\040"))
    return line

def _llvm_archive_mtree_impl(ctx):
    llvm_package = ctx.attr._llvm_package.label
    libraries = {}
    for linker_input in ctx.attr.llvm[CcInfo].linking_context.linker_inputs.to_list():
        is_llvm = (
            linker_input.owner.repo_name == llvm_package.repo_name and
            linker_input.owner.package == llvm_package.package
        )
        for library in linker_input.libraries:
            archive = library.static_library or library.pic_static_library
            if archive:
                libraries[archive.path] = struct(
                    file = archive,
                    is_llvm = is_llvm,
                )
    # Keep the action arguments stable so changes in their order do not
    # invalidate Bazel's action cache.
    static_libraries = sorted(libraries.values(), key = lambda library: library.file.path)
    shared_libraries = ctx.attr.shared[DefaultInfo].files.to_list()
    cxxstdlibs = []
    for target, destination in ctx.attr.cxxstdlibs.items():
        files = target[DefaultInfo].files.to_list()
        if len(files) != 1:
            fail("{} must produce exactly one file".format(target.label))
        cxxstdlibs.append(struct(file = files[0], destination = destination))
    cxxstdlibs = sorted(cxxstdlibs, key = lambda library: library.destination)
    inputs = [library.file for library in static_libraries + cxxstdlibs] + shared_libraries

    mtree = ctx.actions.declare_file(ctx.label.name + ".mtree")
    content = ctx.actions.args()
    content.set_param_file_format("multiline")
    content.add("#mtree")
    content.add(_mtree_line("lib", "dir"))
    content.add(_mtree_line("lib/llvm", "dir"))
    content.add(_mtree_line("lib/llvm/lib", "dir"))
    for library in static_libraries:
        # libLLVM artifacts coming from Bazel, unlike those coming from
        # traditional packaging, have a `lib` prefix instead of `libLLVM`
        # (e.g. `libCore` instead of `libLLVMCore`). To make it easier for
        # build.rs to distinguish them from non-LLVM libraries such as
        # `libzstd`, keep them in a separate directory.
        directory = "lib/llvm/lib" if library.is_llvm else "lib"
        content.add(_mtree_line(
            "{}/{}".format(directory, library.file.basename),
            "file",
            library.file.path,
        ))
    for library in shared_libraries:
        content.add(_mtree_line(
            "lib/llvm/lib/{}".format(library.basename),
            "file",
            library.path,
        ))
    for library in cxxstdlibs:
        content.add(_mtree_line(
            "lib/{}".format(library.destination),
            "file",
            library.file.path,
        ))
    ctx.actions.write(mtree, content)

    return [
        DefaultInfo(files = depset([mtree])),
        OutputGroupInfo(srcs = depset(inputs)),
    ]

_llvm_archive_mtree = rule(
    implementation = _llvm_archive_mtree_impl,
    attrs = {
        "_llvm_package": attr.label(default = Label("@llvm-project//llvm:Support")),
        "cxxstdlibs": attr.label_keyed_string_dict(mandatory = True),
        "llvm": attr.label(mandatory = True, providers = [CcInfo]),
        "shared": attr.label(mandatory = True),
    },
)

def llvm_archive(name, cxxstdlibs, llvm, shared):
    """Build a zstd archive from Bazel LLVM outputs."""
    mtree = name + "-mtree"
    _llvm_archive_mtree(
        name = mtree,
        cxxstdlibs = cxxstdlibs,
        llvm = llvm,
        shared = shared,
        tags = ["manual"],
    )
    srcs = name + "-srcs"
    native.filegroup(
        name = srcs,
        srcs = [mtree],
        output_group = "srcs",
        tags = ["manual"],
    )
    tar(
        name = name,
        srcs = [srcs],
        compress = "zstd",
        mtree = mtree,
        tags = ["manual"],
    )
