# BPF Linker 🔗

bpf-linker aims to simplify building modern BPF programs while still supporting
older, more restrictive kernels.

[![Build status][build-badge]][build-url]

[build-badge]: https://img.shields.io/github/actions/workflow/status/aya-rs/bpf-linker/ci.yml
[build-url]: https://github.com/aya-rs/bpf-linker/actions/workflows/ci.yml

## Overview

bpf-linker can be used to statically link multiple BPF object files together
and optionally perform optimizations needed to target older kernels. It
operates on LLVM bitcode, so the inputs can be bitcode files (.bc), LLVM IR
files (.ll), or object files with embedded bitcode (.o), optionally stored
inside ar archives (.a).

## Installation

### cargo-binstall

The recommended installation method is via
[cargo-binstall][cargo-binstall]. Install `cargo-binstall` first, then run:

```sh
cargo binstall bpf-linker
```

[cargo-binstall]: https://github.com/cargo-bins/cargo-binstall

### Manual download

Download the tarball from the [releases page][releases] that matches your Rust
target triple. The published binaries currently use `*-apple-darwin` for macOS
and `*-unknown-linux-musl` for Linux.

After downloading, unpack the archive into a directory that is included in your
`PATH`.

Example:

```sh
# Linux ARM64
curl -LO https://github.com/aya-rs/bpf-linker/releases/latest/download/bpf-linker-aarch64-unknown-linux-musl.tar.gz
# Linux x86_64
curl -LO https://github.com/aya-rs/bpf-linker/releases/latest/download/bpf-linker-x86_64-unknown-linux-musl.tar.gz
# macOS ARM64
curl -LO https://github.com/aya-rs/bpf-linker/releases/latest/download/bpf-linker-aarch64-apple-darwin.tar.gz
# macOS x86_64
curl -LO https://github.com/aya-rs/bpf-linker/releases/latest/download/bpf-linker-x86_64-apple-darwin.tar.gz

mkdir -p "$HOME/.local/bin"
tar -xpf bpf-linker-*.tar.gz -C "$HOME/.local/bin"
# Add this line to your shell startup file. If you use a different shell,
# refer to its documentation for adding directories to PATH.
export PATH="$HOME/.local/bin:$PATH"
```

[releases]: https://github.com/aya-rs/bpf-linker/releases

### Packages

bpf-linker may also be available through your operating system's package
repositories. In general, packaged builds are expected to work with Rust
toolchains provided by the same package manager. If you use Rust via rustup,
prefer installing bpf-linker with cargo-binstall or from the release tarballs
instead.

Current packaging status:

[![Packaging status](https://repology.org/badge/vertical-allrepos/bpf-linker.svg)](https://repology.org/project/bpf-linker/versions)

### Building from source

Building from source, including even a plain `cargo install bpf-linker`
invocation, is **not** recommended for regular users due to dependency on
specific LLVM version, system libraries and overall complexity of getting
the setup right.

If you're interested in packaging or contributing to bpf-linker, you're
welcome to check the build instructions in [BUILDING.md](./BUILDING.md).

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
                                     LLVM 22 builds also support allows-misaligned-mem-access. Use +feature to
                                     enable a feature, or -feature to disable it. For example --cpu-features=+allows-
                                     misaligned-mem-access,+alu32,-dwarfris [default: ]
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
