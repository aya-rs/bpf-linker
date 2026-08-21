"""Expose the Cargo package version to Bazel Rust targets."""

# rules_rs has no public repository-time TOML helper.
# buildifier: disable=bzl-visibility
load("@rules_rs//rs/private:toml2json.bzl", "run_toml2json")

def _cargo_toml_version_impl(rctx):
    cargo_toml = rctx.path(rctx.attr.cargo_toml)
    rctx.watch(cargo_toml)

    version = run_toml2json(rctx, cargo_toml)["package"]["version"]
    rctx.file("BUILD.bazel", executable = False)
    rctx.file("version.bzl", "VERSION = %s\n" % json.encode(version), executable = False)

    return rctx.repo_metadata(reproducible = True)

cargo_toml_version = repository_rule(
    implementation = _cargo_toml_version_impl,
    attrs = {
        "cargo_toml": attr.label(allow_single_file = True, mandatory = True),
    },
)
