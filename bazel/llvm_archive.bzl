"""Package the Bazel-built LLVM libraries for use by Cargo builds."""

load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")
load("@tar.bzl", "mutate", "tar")

def _llvm_install_impl(ctx):
    libraries = {}
    for linker_input in ctx.attr.llvm[CcInfo].linking_context.linker_inputs.to_list():
        for library in linker_input.libraries:
            archive = library.static_library or library.pic_static_library
            if archive:
                libraries[archive.path] = archive

    static_libraries = sorted(libraries.values(), key = lambda file: file.path)
    shared_libraries = ctx.attr.shared[DefaultInfo].files.to_list()
    inputs = static_libraries + shared_libraries
    mappings = []
    for library in static_libraries:
        name = library.basename
        if name == "libzlib.a":
            name = "libz.a"
        elif name != "libzstd.a":
            name = "libLLVM" + name[3:]
        mappings.append("{}={}".format(library.path, name))
    mappings.extend([
        "{}={}".format(library.path, library.basename)
        for library in shared_libraries
    ])
    output = ctx.actions.declare_directory(ctx.label.name)
    args = ctx.actions.args()
    args.add(output.path)
    args.add_all(mappings)
    ctx.actions.run_shell(
        arguments = [args],
        command = """
set -eu
output="$1"
shift
mkdir -p "$output/lib"
for mapping in "$@"; do
  library=${mapping%%=*}
  name=${mapping#*=}
  cp -L "$library" "$output/lib/$name"
done
""",
        inputs = inputs,
        mnemonic = "LlvmInstall",
        outputs = [output],
        progress_message = "Creating %{label}",
    )
    return DefaultInfo(files = depset([output]))

_llvm_install = rule(
    implementation = _llvm_install_impl,
    attrs = {
        "llvm": attr.label(mandatory = True, providers = [CcInfo]),
        "shared": attr.label(mandatory = True),
    },
)

def llvm_archive(name, llvm, shared):
    """Build an install-shaped zstd archive from Bazel LLVM outputs."""
    install = name + "-files"
    _llvm_install(
        name = install,
        llvm = llvm,
        shared = shared,
        tags = ["manual"],
    )
    tar(
        name = name,
        srcs = [install],
        compress = "zstd",
        mutate = mutate(
            strip_prefix = install,
            tags = ["manual"],
        ),
        tags = ["manual"],
    )
