"""BPF linker fixture helpers."""

load("@platforms//host:constraints.bzl", "HOST_CONSTRAINTS")
load("@rules_rs//rs:rust_binary.bzl", "rust_binary")
load("@rules_rs//rs:rust_library.bzl", "rust_library")
load("@rules_rs//rs:rust_shared_library.bzl", "rust_shared_library")
load("@rules_rs//rs:rust_test.bzl", "rust_test")
load("@rules_shell//shell:sh_test.bzl", "sh_test")

# TODO: Remove rust_test_with_junit after
# https://github.com/dzbarsky/bazel/pull/1 and its prerequisite changes land.
def rust_test_with_junit(name, **kwargs):
    """Wraps a Rust test so the test writes Bazel's JUnit output itself.

    Bazel otherwise runs generate-xml.sh after the test. That fallback is not
    cross-host safe when a Windows client executes the test remotely on Linux:
    https://github.com/bazelbuild/bazel/issues/19587
    """
    binary_name = name + "-binary"
    rust_test(
        name = binary_name,
        tags = ["manual"],
        **kwargs
    )
    sh_test(
        name = name,
        srcs = ["//tests:rust_test_wrapper.sh"],
        args = ["$(rootpath :{})".format(binary_name)],
        data = [binary_name],
    )

def _bpf_transition_impl(_, attr):
    # Compile srcs for the requested BPF target with the alloc crate available.
    return {
        "//command_line_option:platforms": str(attr.target_platform),
        "@rules_rust//rust/settings:no_std": "alloc",
    }

_bpf_transition = transition(
    implementation = _bpf_transition_impl,
    inputs = [],
    outputs = [
        "//command_line_option:platforms",
        "@rules_rust//rust/settings:no_std",
    ],
)

def _transition_filegroup_impl(ctx):
    # Forward files and default_runfiles without modifying them.
    default_info = ctx.attr.src[0][DefaultInfo]
    return [DefaultInfo(
        files = default_info.files,
        runfiles = default_info.default_runfiles,
    )]

bpf_transition_filegroup = rule(
    doc = "Builds src for target_platform and forwards its DefaultInfo.",
    implementation = _transition_filegroup_impl,
    attrs = {
        "src": attr.label(
            cfg = _bpf_transition,
            mandatory = True,
        ),
        "target_platform": attr.label(mandatory = True),
    },
)

def _crate_name(src):
    return src.split("/")[-1].replace(".rs", "").replace("-", "_")

def bpf_aux_library(
        name,
        src,
        rustc_flags = []):
    """Declares a Rust library used by a BPF linker fixture."""
    rust_library(
        name = name,
        srcs = [src],
        crate_name = _crate_name(src),
        crate_root = src,
        edition = "2021",
        rustc_flags = rustc_flags,
    )

def bpf_assembly_test(
        name,
        src,
        crate_type = "bin",
        deps = [],
        rustc_flags = [],
        check_prefixes = "",
        emit = "asm",
        panic_handler = ":assembly-loop-panic-handler"):
    """Checks the assembly emitted for a BPF Rust fixture."""
    _bpf_filecheck_test(
        name = name,
        src = src,
        check_prefixes = check_prefixes,
        crate_type = crate_type,
        deps = deps + ([panic_handler] if panic_handler else []),
        rustc_flags = rustc_flags + ["-Clink-arg=--emit=" + emit],
    )

def bpf_btf_test(
        name,
        src,
        crate_type = "bin",
        deps = [],
        compile_data = [],
        rustc_flags = [],
        check_file = None,
        panic_handler = ":btf-loop-panic-handler"):
    """Checks the BTF emitted for a BPF Rust fixture."""
    _bpf_filecheck_test(
        name = name,
        src = src,
        btf = True,
        check_file = check_file,
        compile_data = compile_data,
        crate_type = crate_type,
        deps = deps + ([panic_handler] if panic_handler else []),
        rustc_flags = [
            "-Cdebuginfo=2",
            "-Clink-arg=--btf",
        ] + rustc_flags + ["-Clink-arg=--emit=obj"],
    )

def _bpf_filecheck_test(
        name,
        src,
        crate_type = "bin",
        deps = [],
        compile_data = [],
        rustc_flags = [],
        btf = False,
        check_file = None,
        check_prefixes = ""):
    fixture_name = name + "-fixture"
    bpf_name = fixture_name + "-bpfel"
    check_file = check_file or src

    # Exercise the native bpf-linker on each CI host. Its dependencies may
    # still build remotely, but this rustc invocation must execute locally.
    rust_kwargs = {
        "name": fixture_name,
        "crate_name": _crate_name(src),
        "crate_root": src,
        "compile_data": compile_data,
        "deps": deps,
        "edition": "2021",
        "exec_compatible_with": HOST_CONSTRAINTS,
        "rustc_flags": rustc_flags,
        "srcs": [src],
        "tags": [
            "manual",
            "no-remote-exec",
        ],
    }
    if crate_type == "cdylib":
        rust_shared_library(**rust_kwargs)
    else:
        rust_binary(
            crate_type = crate_type,
            **rust_kwargs
        )

    bpf_transition_filegroup(
        name = bpf_name,
        src = fixture_name,
        tags = ["manual"],
        target_platform = "@rules_rs//rs/platforms:bpfel-unknown-none",
    )

    data = [
        bpf_name,
        check_file,
        "@llvm-project//llvm:FileCheck",
    ]
    args = [
        "$(rootpath {})".format(bpf_name),
        "$(rootpath {})".format(check_file),
        "$(rootpath @llvm-project//llvm:FileCheck)",
        "btf" if btf else "assembly",
        check_prefixes or "-",
    ]
    if btf:
        data.append("@btfdump_crates//:btfdump__btf")
        args.append("$(rootpath @btfdump_crates//:btfdump__btf)")

    sh_test(
        name = name + "-test",
        srcs = ["filecheck.sh"],
        args = args,
        data = data,
    )
