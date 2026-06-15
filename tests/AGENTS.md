# Prompt: Synchronize Bazel compiletest targets

Use this prompt to update `tests/BUILD.bazel` after adding, removing, or
changing compiletest binaries:

```text
Update tests/BUILD.bazel so Bazel builds and checks every compiletest binary
that tests/compiletest.rs runs.

Derive the required Bazel targets from the repository instead of assuming
that tests/BUILD.bazel is already complete:

1. Read tests/compiletest.rs and identify every src_base passed to run_mode.
2. Enumerate the direct Rust source files in each corresponding assembly
   directory. Exclude auxiliary directories and files containing the
   `// ignore-test` directive. Include nightly directories when
   tests/compiletest.rs includes them.
3. Read each source file's compiletest directives, including
   `compile-flags`, revision-specific `compile-flags`, `revisions`,
   `aux-build`, and mode-specific directives.
4. Compare that inventory with tests/BUILD.bazel. Add missing Bazel targets,
   update stale Bazel targets, and remove Bazel targets for compiletest
   binaries that no longer exist.

Use bpf_assembly_test for binaries under an assembly src_base and bpf_btf_test
for binaries under a BTF src_base. Translate each compiletest directive into
the corresponding helper argument:

- Map `--crate-type` to `crate_type`.
- Map `--emit` and linker arguments not supplied by the helper to `emit` or
  `rustc_flags`.
- For a file with `revisions`, declare one target per revision and set
  `check_prefixes` so FileCheck runs `CHECK` and that revision's checks.
- Declare each `aux-build` source with bpf_aux_library and add the matching
  label to `deps`. Preserve required auxiliary rustc flags, such as debug
  information for BTF inputs.
- Add run_binary, compile_data, rustc_flags, or panic_handler settings when a
  compiletest binary consumes generated C or LLVM bitcode or intentionally
  omits the default panic handler.

Do not duplicate flags that bpf_assembly_test or bpf_btf_test already adds.
Preserve an intentional difference between compiletest and Bazel only when a
comment names the exact compiletest directive and explains the difference.
Keep target names derived consistently from the source path and revision.

After editing:

1. Produce a source-to-target inventory. Confirm that every non-ignored
   compiletest binary has a Bazel test and that every exclusion is explained
   by an `// ignore-test` directive.
2. Run buildifier on tests/BUILD.bazel and any changed .bzl files.
3. Run:

   bazel test //tests:all --config=remote --cache_test_results=no \
     --test_output=errors

4. Run `git diff --check`.

Report the source files and Bazel targets added, changed, removed, or
intentionally excluded. Do not change compiletest source files merely to make
the Bazel translation easier.
```
