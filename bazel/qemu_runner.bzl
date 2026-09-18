"""Executable wrappers for QEMU user-mode toolchains."""

load("@bazel_lib//lib:transitions.bzl", "platform_transition_binary")

_QEMU_TOOLCHAIN_TYPE = Label("@rules_qemu//qemu:toolchain_type")

def _qemu_runner_impl(ctx):
    qemu = ctx.toolchains[_QEMU_TOOLCHAIN_TYPE].qemu
    executable = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(
        output = executable,
        target_file = qemu,
    )

    return [DefaultInfo(
        files = depset([executable]),
        executable = executable,
        runfiles = ctx.runfiles(files = [qemu]),
    )]

_qemu_runner = rule(
    implementation = _qemu_runner_impl,
    executable = True,
    toolchains = [_QEMU_TOOLCHAIN_TYPE],
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
