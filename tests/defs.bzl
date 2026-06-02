"""BPF linker fixture helpers."""

load("@platforms//host:constraints.bzl", "HOST_CONSTRAINTS")
load("@rules_rs//rs:rust_binary.bzl", "rust_binary")
load("@rules_rs//rs:rust_library.bzl", "rust_library")
load("@rules_rs//rs:rust_shared_library.bzl", "rust_shared_library")
load("@rules_shell//shell:sh_test.bzl", "sh_test")
load("//bazel:defs.bzl", "bpf_transition_filegroup")

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
        srcs = [fixture_name],
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
