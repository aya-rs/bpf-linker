"""BPF linker fixture helpers."""

load("@platforms//host:constraints.bzl", "HOST_CONSTRAINTS")
load("@rules_rs//rs:rust_binary.bzl", "rust_binary")
load("@rules_rs//rs:rust_library.bzl", "rust_library")
load("@rules_rs//rs:rust_shared_library.bzl", "rust_shared_library")
load("@rules_shell//shell:sh_test.bzl", "sh_test")
load("@with_cfg.bzl", "with_cfg")

def _bpify(rule):
    # with_cfg applies these settings to the fixture and its transitive
    # dependencies:
    # https://bazel.build/extending/config#user-defined-transitions
    return with_cfg(rule).set(
        "platforms",
        [Label("@rules_rs//rs/platforms:bpfel-unknown-none")],
    ).set(
        # rules_rust supports "alloc" as its only no_std mode; these fixtures
        # use core only. Use core after rules_rust supports core-only no_std:
        # https://github.com/hermeticbuild/rules_rust/issues/28
        Label("@rules_rust//rust/settings:no_std"),
        "alloc",
    ).build()

rust_bpf_shared_library, _ = _bpify(rust_shared_library)
rust_bpf_binary, __ = _bpify(rust_binary)

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
        panic_handler = ":btf-loop-panic-handler"):
    """Checks the BTF emitted for a BPF Rust fixture."""
    _bpf_filecheck_test(
        name = name,
        src = src,
        btf = True,
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
        check_prefixes = ""):
    fixture_name = name + "-fixture"
    bpf_name = fixture_name + "-bpfel"
    rust_kwargs = {
        "name": bpf_name,
        "crate_name": _crate_name(src),
        "crate_root": src,
        "compile_data": compile_data,
        "deps": deps,
        "edition": "2021",
        # Exercise the native bpf-linker on each CI host. Its dependencies may
        # still build remotely, but this rustc invocation must execute locally.
        # We must both mark it compatible with host and tag it no-remote-exec
        # due to Bazel quirks.
        "exec_compatible_with": HOST_CONSTRAINTS,
        "rustc_flags": rustc_flags,
        "srcs": [src],
        "tags": [
            "manual",
            "no-remote-exec",
        ],
    }
    if crate_type == "cdylib":
        rust_bpf_shared_library(**rust_kwargs)
    else:
        rust_bpf_binary(
            crate_type = crate_type,
            **rust_kwargs
        )

    data = [
        bpf_name,
        src,
        "@llvm-project//llvm:FileCheck",
    ]
    env = {
        "ARTIFACT": "$(location %s)" % bpf_name,
        "CHECKS": "$(location %s)" % src,
        "CHECK_PREFIXES": check_prefixes or "-",
        "FILECHECK": "$(location @llvm-project//llvm:FileCheck)",
        "MODE": "btf" if btf else "assembly",
    }
    if btf:
        data.append("@btfdump_crates//:btfdump__btf")
        env["BTFDUMP"] = "$(location @btfdump_crates//:btfdump__btf)"

    # filecheck_wrapper.sh runs btfdump before FileCheck for BTF tests.
    sh_test(
        name = name + "-test",
        srcs = ["filecheck_wrapper.sh"],
        data = data,
        env = env,
    )
