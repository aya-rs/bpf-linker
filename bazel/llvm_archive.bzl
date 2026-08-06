"""Package the Bazel-built LLVM libraries for use by Cargo builds."""

load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")
load("@tar.bzl", "mutate", "tar")

def _llvm_archive_files_impl(ctx):
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
    output = ctx.actions.declare_directory(ctx.label.name)
    args = ctx.actions.args()
    args.add(output.path)
    for library in static_libraries:
        # libLLVM artifacts coming from Bazel, unlike those coming from
        # traditional packaging, have a `lib` prefix instead of `libLLVM`
        # (e.g. `libCore` instead of `libLLVMCore`). To make it easier for
        # build.rs to distinguish them from non-LLVM libraries such as
        # `libzstd`, keep them in a separate directory.
        directory = "lib/llvm/lib" if library.is_llvm else "lib"
        args.add(library.file)
        args.add("{}/{}".format(directory, library.file.basename))
    for library in shared_libraries:
        args.add(library)
        args.add("lib/llvm/lib/{}".format(library.basename))
    for library in cxxstdlibs:
        args.add(library.file)
        args.add("lib/{}".format(library.destination))
    ctx.actions.run_shell(
        arguments = [args],
        command = """
set -eu
output="$1"
shift
mkdir -p "$output/lib/llvm/lib"
while [ "$#" -ne 0 ]; do
  library="$1"
  destination="$2"
  shift 2
  cp -L "$library" "$output/$destination"
done
""",
        inputs = inputs,
        mnemonic = "LlvmArchiveFiles",
        outputs = [output],
        progress_message = "Creating %{label}",
    )
    return DefaultInfo(files = depset([output]))

_llvm_archive_files = rule(
    implementation = _llvm_archive_files_impl,
    attrs = {
        "_llvm_package": attr.label(default = Label("@llvm-project//llvm:Support")),
        "cxxstdlibs": attr.label_keyed_string_dict(mandatory = True),
        "llvm": attr.label(mandatory = True, providers = [CcInfo]),
        "shared": attr.label(mandatory = True),
    },
)

def llvm_archive(name, cxxstdlibs, llvm, shared):
    """Build a zstd archive from Bazel LLVM outputs."""
    archive_files = name + "-files"
    _llvm_archive_files(
        name = archive_files,
        cxxstdlibs = cxxstdlibs,
        llvm = llvm,
        shared = shared,
        tags = ["manual"],
    )
    tar(
        name = name,
        srcs = [archive_files],
        compress = "zstd",
        mutate = mutate(
            strip_prefix = archive_files,
            tags = ["manual"],
        ),
        tags = ["manual"],
    )
