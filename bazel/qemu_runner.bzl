"""Executable wrappers for QEMU user-mode toolchains."""

load("@bazel_lib//lib:transitions.bzl", "platform_transition_binary")
load("@hermetic_launcher//launcher:lib.bzl", "launcher")

_QEMU_TOOLCHAIN_TYPE = Label("@rules_qemu//qemu:toolchain_type")

def _qemu_runner_impl(ctx):
    qemu = ctx.toolchains[_QEMU_TOOLCHAIN_TYPE].qemu
    executable = ctx.actions.declare_file(ctx.label.name)
    embedded_args, transformed_args = launcher.args_from_entrypoint(
        executable_file = qemu,
    )
    launcher.compile_stub(
        ctx = ctx,
        embedded_args = embedded_args,
        transformed_args = transformed_args,
        output_file = executable,
        cfg = "exec",
    )

    return [DefaultInfo(
        files = depset([executable]),
        executable = executable,
        runfiles = ctx.runfiles(files = [qemu]),
    )]

_qemu_runner = rule(
    implementation = _qemu_runner_impl,
    executable = True,
    toolchains = [
        _QEMU_TOOLCHAIN_TYPE,
        "@hermetic_launcher//launcher:finalizer_toolchain_type",
        "@hermetic_launcher//launcher:template_exec_toolchain_type",
    ],
)

def qemu_runner(name, target_platform, **kwargs):
    """Creates an executable QEMU user-mode runner for a target platform."""
    _qemu_runner(
        name = name + "_raw",
        tags = ["manual"],
    )

    platform_transition_binary(
        name = name,
        binary = name + "_raw",
        target_platform = target_platform,
        **kwargs
    )
