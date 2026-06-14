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

    Bazel otherwise generates an extra action to run generate-xml.sh after the test.
    That fallback does not work properly when a Windows host uses Linux remote exec.
    This can be removed once https://github.com/dzbarsky/bazel/pull/1 and its prerequisite changes land.
    """
    binary_name = name + "-binary"
    rust_test(
        name = binary_name,
        tags = ["manual"],
        **kwargs
    )
    sh_test(
        name = name,
        srcs = ["//bazel:rust_test_wrapper.sh"],
        args = ["$(rootpath :%s)" % binary_name],
        data = [binary_name],
    )

_BPF_TRANSITION_SETTINGS = {
    "//command_line_option:platforms": "@rules_rs//rs/platforms:bpfel-unknown-none",
    # rules_rust supports "alloc" as its only no_std mode; these fixtures use core only.
    "@rules_rust//rust/settings:no_std": "alloc",
}

def _bpf_transition_impl(_settings, _attr):
    return _BPF_TRANSITION_SETTINGS

_bpf_transition = transition(
    implementation = _bpf_transition_impl,
    inputs = [],
    outputs = _BPF_TRANSITION_SETTINGS.keys(),
)

def _transition_filegroup_impl(ctx):
    # We just want to forward the DefaultInfo, but we must drop the executable (that we don't care about anyway)
    # because Bazel doesn't allow forwarding that, so we reconstruct the other fields.
    default_info = ctx.attr.src[0][DefaultInfo]
    return [DefaultInfo(
        files = default_info.files,
        runfiles = default_info.default_runfiles,
    )]

# This rule forces the `src` to be built with a different set of flags (for the BPF target).
# See https://bazel.build/extending/config#user-defined-transitions for more info on transitioning flags in the build graph.
bpf_transition_filegroup = rule(
    implementation = _transition_filegroup_impl,
    attrs = {
        "src": attr.label(
            cfg = _bpf_transition,
            mandatory = True,
        ),
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
        "name": fixture_name,
        "crate_name": _crate_name(src),
        "crate_root": src,
        "compile_data": compile_data,
        "deps": deps,
        "edition": "2021",
        # Exercise the native bpf-linker on each CI host. Its dependencies may
        # still build remotely, but this rustc invocation must execute locally.
        # We must both mark it compatible with host and tag it no-remote-exec due to Bazel quirks.
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

    # filecheck.sh writes test.xml to avoid Bazel's cross-host generate-xml.sh
    # fallback and runs btfdump before FileCheck for BTF tests.
    sh_test(
        name = name + "-test",
        srcs = ["filecheck.sh"],
        data = data,
        env = env,
    )
