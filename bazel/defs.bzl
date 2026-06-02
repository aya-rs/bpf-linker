"""Shared Bazel definitions for bpf-linker."""

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
