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
# Dynamic linking, Rust nightly
cargo +nightly --no-default-features --features llvm-22

# Static linking
cargo +nightly --no-default-features --features llvm-22,llvm-link-static
```
