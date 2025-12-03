# BPF Linker 🔗

bpf-linker aims to simplify building modern BPF programs while still supporting
older, more restrictive kernels.

[![Build status][build-badge]][build-url]

[build-badge]: https://img.shields.io/github/actions/workflow/status/aya-rs/bpf-linker/ci.yml
[build-url]: https://github.com/aya-rs/bpf-linker/actions/workflows/ci.yml

## Overview

bpf-linker can be used to statically link multiple BPF object files together
and optionally perform optimizations needed to target older kernels. It
operates on LLVM bitcode, so the inputs must be bitcode files (.bc) or object
files with embedded bitcode (.o), optionally stored inside ar archives (.a).

## Installation

### Binary

The easiest way to install the linker is by using [cargo-binstall][cargo-binstall]:

```sh
cargo binstall bpf-linker
```

You can also download prebuit binaries directly from the [release pages][release].

We currently provide binaries for the following platforms:

- Linux aarch64
- Linux x86_64
- macOS aarch64
- macOS x86_64

For other platforms, install the linker using the *From source* method.

[cargo-binstall]: https://crates.io/crates/cargo-binstall
[release]: https://github.com/aya-rs/bpf-linker/releases

### From source

The linker requires LLVM 21. It can use the same LLVM used by the rust compiler,
or it can use an external LLVM installation.

#### Using LLVM provided by rustc

All you need to do is run:

```sh
cargo install bpf-linker
```

However, this method works only for Linux x86_64 (`x86_64-unknown-linux-gnu`).
For any other platform, use the *external LLVM* method.

#### Using external LLVM

##### System packages

On Debian based distributions you need to install the `llvm-21-dev` and `libclang-21-dev`
packages, from the official LLVM repo at https://apt.llvm.org.

You may need to build LLVM from source if a recent version is not available
through any package manager that supports your platform.

Once you have installed LLVM 21 you can install the linker running:

```sh
cargo install bpf-linker --no-default-features --features llvm-21
```

##### Building LLVM from source

LLVM can be built from source using the `xtask build-llvm` subcommand, included
in bpf-linker sources.

First, the LLVM sources offered in [our fork][llvm-fork] need to be cloned,
using the branch matching the Rust toolchain you want to use. For current
nightly:

```sh
git clone -b rustc/21.1-2025-08-01 https://github.com/aya-rs/llvm-project ./llvm-project
```

If in doubt about which branch to use, check the LLVM version used by your Rust
compiler:

```sh
rustc [+toolchain] --version -v | grep LLVM
```

When the sources are ready, the LLVM artifacts can be built and installed in
the directory provided in the `--install-prefix` argument, using `--build-dir`
to store the state of the build.

```sh
cargo xtask llvm build \
    --src-dir ./llvm-project \
    --build-dir ./llvm-build \
    --install-prefix ./llvm-install
```

After that, bpf-linker can be built with the `LLVM_PREFIX` environment variable
pointing to that directory:

```sh
LLVM_PREFIX=./llvm-install cargo install --path .
```

If you don't have cargo you can get it from https://rustup.rs or from your distro's package manager.

[llvm-fork]: https://github.com/aya-rs/llvm-project

## Usage

### Rust

#### Nightly

To compile your eBPF crate just run:

```sh
cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release
```

If you don't want to have to pass the `target` and `build-std` options every
time, you can put them in `.cargo/config.toml` under the crate's root folder:

```toml
[build]
target = "bpfel-unknown-none"

[unstable]
build-std = ["core"]
```

##### (Experimental) BTF support

To emit [BTF debug information](https://www.kernel.org/doc/html/next/bpf/btf.html),
set the following rustflags:

```
-C debuginfo=2 -C link-arg=--btf
```

These flags will work only for the eBPF targets (`bpfeb-unknown-none`,
`bpfel-unknown-none`). Make sure you are specifying them only for eBPF crates,
not for the user-space ones!

When compiling an eBPF crate directly with `cargo +nightly build`, they can be
defined through the `RUSTFLAGS` environment variable:

```sh
RUSTFLAGS="-C debuginfo=2 -C link-arg=--btf" cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release
```

To avoid specifying them manually, you can put them in `.cargo/config.toml`:

```toml
[build]
target = "bpfel-unknown-none"
rustflags = "-C debuginfo=2 -C link-arg=--btf"

[unstable]
build-std = ["core"]
```

After that, the BPF object file present in `target/bpfel-unknown-none/release`
should contain a BTF section.

### Clang

For a simple example of how to use the linker with clang see [this
gist](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b). In
the example
[lib.c](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-lib-c)
is compiled as a static library which is then linked by
[program.c](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-program-c).
The
[Makefile](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-makefile)
shows how to compile the C code and then link it.

### CLI syntax

```
bpf-linker

USAGE:
    bpf-linker [FLAGS] [OPTIONS] --output <output> [--] [inputs]...

FLAGS:
        --disable-expand-memcpy-in-order    Disable passing --bpf-expand-memcpy-in-order to LLVM
        --disable-memory-builtins           Disble exporting memcpy, memmove, memset, memcmp and bcmp. Exporting those
                                            is commonly needed when LLVM does not manage to expand memory intrinsics to
                                            a sequence of loads and stores
    -h, --help                              Prints help information
        --ignore-inline-never               Ignore `noinline`/`#[inline(never)]`. Useful when targeting kernels that
                                            don't support function calls
        --unroll-loops                      Try hard to unroll loops. Useful when targeting kernels that don't support
                                            loops
    -V, --version                           Prints version information

OPTIONS:
        --cpu <cpu>                  Target BPF processor. Can be one of `generic`, `probe`, `v1`, `v2`, `v3` [default:
                                     generic]
        --cpu-features <features>    Enable or disable CPU features. The available features are: alu32, dummy, dwarfris.
                                     Use +feature to enable a feature, or -feature to disable it.  For example --cpu-
                                     features=+alu32,-dwarfris [default: ]
        --dump-module <path>         Dump the final IR module to the given `path` before generating the code
        --emit <emit>                Output type. Can be one of `llvm-bc`, `asm`, `llvm-ir`, `obj` [default: obj]
        --export <symbols>...        Comma separated list of symbols to export. See also `--export-symbols`
        --export-symbols <path>      Export the symbols specified in the file `path`. The symbols must be separated by
                                     new lines
    -L <libs>...                     Add a directory to the library search path
        --llvm-args <args>...        Extra command line arguments to pass to LLVM
        --log-file <path>            Output logs to the given `path`
        --log-level <level>          Set the log level. Can be one of `off`, `info`, `warn`, `debug`, `trace`
    -O <optimize>...                 Optimization level. 0-3, s, or z [default: 2]
    -o, --output <output>            Write output to <output>
        --target <target>            LLVM target triple. When not provided, the target is inferred from the inputs

ARGS:
    <inputs>...    Input files. Can be object files or static libraries
```

## License

bpf-linker is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
