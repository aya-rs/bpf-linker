"""Package the Bazel-built LLVM libraries and tools for use by Cargo builds."""

load("@bazel_lib//lib:copy_file.bzl", "copy_file")
load("@bazel_lib//lib:transitions.bzl", "platform_transition_filegroup")
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
    static_libraries = libraries.values()
    shared_libraries = ctx.attr.shared[DefaultInfo].files.to_list()
    cxxstdlibs = []
    for target, destination in ctx.attr.cxxstdlibs.items():
        files = target[DefaultInfo].files.to_list()
        if len(files) != 1:
            fail("{} must produce exactly one file".format(target.label))
        cxxstdlibs.append(struct(file = files[0], destination = destination))
    filecheck = ctx.file.filecheck
    inputs = [library.file for library in static_libraries + cxxstdlibs]
    inputs.extend(shared_libraries)
    inputs.append(filecheck)

    mtree = ctx.actions.declare_file(ctx.label.name + ".mtree")
    content = ctx.actions.args()
    content.set_param_file_format("multiline")
    content.add("#mtree")
    content.add(_mtree_line("bin", "dir"))
    content.add(_mtree_line("bin/FileCheck", "file", filecheck.path))
    content.add(_mtree_line("lib", "dir"))

    # Keep the mtree content stable so changes in the input order do not
    # invalidate Bazel's action cache.
    for library in sorted(static_libraries, key = lambda library: library.file.path):
        # libLLVM artifacts coming from Bazel, unlike those coming from CMake,
        # have a `lib` prefix instead of `libLLVM` (e.g. `libCore` instead of
        # `libLLVMCore`). Export the conventional CMake archive name for Cargo
        # consumers.
        basename = library.file.basename
        if library.is_llvm:
            basename = "libLLVM" + basename[len("lib"):]
        elif basename == "libzlib-ng.a":
            # Bazel names zlib-ng's zlib-compatible archive `libzlib-ng.a`, whereas
            # CMake installs it as `libz.a`. Use the CMake name for Cargo consumers.
            basename = "libz.a"
        content.add(_mtree_line(
            "lib/{}".format(basename),
            "file",
            library.file.path,
        ))
    for library in sorted(shared_libraries, key = lambda library: library.path):
        content.add(_mtree_line(
            "lib/{}".format(library.basename),
            "file",
            library.path,
        ))
    for library in sorted(cxxstdlibs, key = lambda library: library.destination):
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
        "filecheck": attr.label(allow_single_file = True, mandatory = True),
        "llvm": attr.label(mandatory = True, providers = [CcInfo]),
        "shared": attr.label(mandatory = True),
    },
)

def llvm_archive(name, cxxstdlibs, filecheck, llvm, shared):
    """Build a zstd archive from Bazel LLVM outputs.

    Args:
        name: Name of the archive target.
        cxxstdlibs: C++ runtime library targets mapped to archive filenames.
        filecheck: FileCheck executable target.
        llvm: LLVM library target supplying headers and static libraries.
        shared: Shared LLVM library target.
    """
    mtree = name + "-mtree"
    _llvm_archive_mtree(
        name = mtree,
        cxxstdlibs = cxxstdlibs,
        filecheck = filecheck,
        llvm = llvm,
        shared = shared,
        tags = ["manual"],
        # LLVM marks FileCheck as test-only, so the archive target chain must
        # also be test-only.
        testonly = True,
    )
    srcs = name + "-srcs"
    native.filegroup(
        name = srcs,
        srcs = [mtree],
        output_group = "srcs",
        tags = ["manual"],
        testonly = True,
    )
    tar(
        name = name,
        srcs = [srcs],
        compress = "zstd",
        mtree = mtree,
        tags = ["manual"],
        testonly = True,
    )

def llvm_archive_for_platform(name, archive, target_platform):
    """Build an LLVM archive for a target platform and give it a unique name."""
    transitioned = name + "-transitioned"
    platform_transition_filegroup(
        name = transitioned,
        srcs = [archive],
        target_platform = target_platform,
        tags = ["manual"],
        testonly = True,
    )
    copy_file(
        name = name,
        src = transitioned,
        out = name + ".tar.zst",
        allow_symlink = True,
        tags = ["manual"],
        testonly = True,
    )
