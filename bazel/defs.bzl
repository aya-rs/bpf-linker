"""Shared Bazel definitions for bpf-linker."""

load("@rules_rs//rs:rust_test.bzl", "rust_test")
load("@rules_shell//shell:sh_test.bzl", "sh_test")

def rust_test_with_junit(name, **kwargs):
    """Declares a Rust test that always writes Bazel's JUnit output."""
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
    files = []
    runfiles = ctx.runfiles()
    for src in ctx.attr.srcs:
        files.append(src[DefaultInfo].files)
        runfiles = runfiles.merge(src[DefaultInfo].default_runfiles)
    return [DefaultInfo(
        files = depset(transitive = files),
        runfiles = runfiles,
    )]

bpf_transition_filegroup = rule(
    implementation = _transition_filegroup_impl,
    attrs = {
        "srcs": attr.label_list(
            allow_empty = False,
            cfg = _bpf_transition,
        ),
        "target_platform": attr.label(mandatory = True),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
)
