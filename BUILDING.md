# Building bpf-linker from source

## LLVM

bpf-linker is a bitcode linker that uses libLLVM to link bitcode inputs. That
means the LLVM version used by bpf-linker must match the LLVM version used by
the Rust toolchain you intend to use.

There are two recommended ways of obtaining an appropriate LLVM.

### Building LLVM from source

LLVM can be built from source using the `xtask build-llvm` subcommand, included
in the bpf-linker sources.

First, clone the LLVM sources from [our fork][llvm-fork], using the branch
that matches the Rust toolchain you want to use. For example:

```sh
git clone -b rustc/22.1-2026-01-27 https://github.com/aya-rs/llvm-project ./llvm-project
```

If in doubt about which branch to use, check the LLVM version used by your Rust
compiler:

```sh
rustc [+toolchain] --version -v | grep LLVM
```

Once the sources are available, LLVM can be built and installed into the
directory specified by `--install-prefix`, using `--build-dir` to store the
build state.

```sh
cargo xtask llvm build \
    --src-dir ./llvm-project \
    --build-dir ./llvm-build \
    --install-prefix ./llvm-install
```

After that, bpf-linker can be built with the `LLVM_PREFIX` environment
variable pointing to that directory:

```sh
export LLVM_PREFIX=./llvm-install
```

Alternatively, the `bin` directory from the install prefix can be added to
`PATH`:

```sh
export PATH="$(pwd)/llvm-install/bin:$PATH"
```

[llvm-fork]: https://github.com/aya-rs/llvm-project

### System packages

On Debian-based distributions, you can install the `llvm-<version>-dev` and
`libclang-<version>-dev` packages from the official LLVM repository at
https://apt.llvm.org.

Different operating systems and Linux distributions might provide their own
LLVM packages. If you're interested in packaging bpf-linker, you may also need
to ensure that the correct LLVM version is packaged for that environment.

## Building bpf-linker with Cargo

bpf-linker uses Cargo features to select the LLVM version, via `llvm-*`
features such as `llvm-22`. By default, LLVM and its dependencies are linked
dynamically. Static linking can be enabled with the `llvm-link-static` feature.

Examples:

```
# Dynamic linking
cargo build --no-default-features --features llvm-22
cargo install bpf-linker --no-default-features --features llvm-22
cargo install --path . --no-default-features --features llvm-22

# Static linking
cargo build --no-default-features --features llvm-22,llvm-link-static
cargo install bpf-linker --no-default-features --features llvm-22,llvm-link-static
cargo install --path . --no-default-features --features llvm-22,llvm-link-static
```

## Running tests

bpf-linker comes with compiletests, similar to the ones in Rust and LLVM, that
compile the code to LLVM IR (or BTF) and assert the output matches the
expected IR.

### With Rust nightly

Use `cargo test` with same arguments as used for build, e.g.:

```
cargo +nightly test --no-default-features --features llvm-22
```

### With Rust stable

BPF targets are [Tier 3 in Rust][rustc-tiers] and therefore rustup does not
provide BPF targets in stable editions of Rust. There are two ways to overcome
that.

[rustc-tiers]: https://doc.rust-lang.org/rustc/target-tier-policy.html

#### Prebuilding the BPF sysroot

Build the BPF sysroot with:

```
RUSTC_SRC="$(rustc --print sysroot)/lib/rustlib/src/rust/library"
BPFEL_SYSROOT_DIR="$(pwd)/bpf-sysroot"
RUSTC_BOOTSTRAP=1 cargo xtask build-std \
  --rustc-src "$RUSTC_SRC" \
  --sysroot-dir "$BPFEL_SYSROOT_DIR" \
  --target bpfel-unknown-none
```

Then point the tests to the sysroot using the `BPFEL_SYSROOT_DIR` variable:

```
BPFEL_SYSROOT_DIR="$(pwd)/bpf-sysroot" \
    cargo test --no-default-features --features llvm-22
```

#### Building the sysroot on demand

It's done by the tests automatically when `BPFEL_SYSROOT_DIR` is not defined,
but in case of Rust stable it requires `RUSTC_BOOTSTRAP=1`:

```
RUSTC_BOOTSTRAP=1 cargo test --no-default-features --features llvm-22
```
