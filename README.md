# BPF Linker 🔗

bpf-linker aims to simplify building modern BPF programs while still supporting
older, more restrictive kernels.

[![Build](https://github.com/aya-rs/bpf-linker/workflows/Build/badge.svg)](https://github.com/aya-rs/bpf-linker/actions?query=workflow%3ABuild)

## Overview

bpf-linker can be used to statically link multiple BPF object files together
and optionally perform optimizations needed to target older kernels. It
operates on LLVM bitcode, so the inputs must be bitcode files (.bc) or object
files with embedded bitcode (.o), optionally stored inside ar archives (.a).

## Installation

The linker requires LLVM 15. It can use the same LLVM used by the rust compiler,
or it can use an external LLVM installation.

If your target is `aarch64-unknown-linux-gnu` (i.e. Linux on Apple Silicon) you 
will have to use the *external LLVM* method.

### Using LLVM provided by rustc

All you need to do is run:

```sh
cargo install bpf-linker
```

### Using external LLVM

On Debian based distributions you need to install the `llvm-15-dev` and
`libclang-15-dev` packages. If your distro doesn't have them you can get them
from the official LLVM repo at https://apt.llvm.org.

On rpm based distribution you need the `llvm-devel` and `clang-devel` packages.
If your distro doesn't have them you can get them from Fedora Rawhide.

Once you have installed LLVM 15 you can install the linker running:

```sh
cargo install --git https://github.com/aya-rs/bpf-linker  --tag v0.9.5 --no-default-features --features system-llvm -- bpf-linker
```

If you don't have cargo you can get it from https://rustup.rs or from your distro's package manager.

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

#### Stable (not recommended)

If for whatever reason you can't use rust nightly to build your project, you can
still compile your eBPF crate with:

```sh
$ cargo rustc --release -- \
        -C linker-plugin-lto \
        -C linker-flavor=wasm-ld -C linker=bpf-linker \
        -C link-arg=--target=bpf
   Compiling bpf-log-clone v0.1.0 (/home/alessandro/bpf-log-clone)
   Finished release [optimized] target(s) in 0.86s

$ file target/release/libbpf_log_clone.so
target/release/libbpf_log_clone.so: ELF 64-bit LSB relocatable, eBPF, version 1 (SYSV), not stripped
```

## Clang

For a simple example of how to use the linker with clang see [this
gist](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b). In
the example
[lib.c](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-lib-c)
is compiled as a static library which is then linked by
[program.c](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-program-c).
The
[Makefile](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-makefile)
shows how to compile the C code and then link it.

### Usage

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

## Development

### Debugging bpf-linker issues

The process for debugging BPF linker issues is as follows:

1. Find a case that reproduces using `bpf-linker`, which can be sometimes be obtained from the output of `cargo xtask build-ebpf`. With this method you may also need to create some of the directories used in the arguments.
1. Build LLVM from source as described [here](https://llvm.org/docs/GettingStarted.html). You will need to use the following flags: `cmake -S llvm -B build -G Ninja -DCMAKE_BUILD_TYPE=Debug -DLLVM_PARALLEL_LINK_JOBS=6 -DLLVM_BUILD_LLVM_DYLIB=true`. Link jobs can use around 16GB RAM per core so you should adjust that number for your environment.
1. Install LLVM using `sudo cmake --build build --target install`
1. In `bpf-linker`, `cargo clean -p llvm-sys` first to ensure that you'll link against your newly built LLVM
1. `cargo build --no-default-features --features system-llvm -p bpf-linker`

#### Attaching the debugger in vscode

Assuming you have the CodeLLDB extension installed, you can add the following to `.vscode/launch.json`:

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "lldb",
            "request": "launch",
            "name": "Debug bpf-linker'",
            "cwd": "${workspaceFolder}",
            "program": "${workspaceFolder}/target/debug/bpf-linker",
            "args": [
                "--export-symbols",
                "/tmp/rustcRb1DOx/symbols",
                "/home/dave/dev/rh-next/target/bpfel-unknown-none/release/deps/rh_next-53bd6e44bb1b7225.rh_next.c2c50e3e-cgu.0.rcgu.o",
                "-L", "/home/dave/dev/rh-next/target/bpfel-unknown-none/release/deps",
                "-L", "/home/dave/dev/rh-next/target/release/deps",
                "-L", "/home/dave/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib/rustlib/bpfel-unknown-none/lib",
                "--cpu", "generic",  "--cpu-features",
                "-L", "/home/dave/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib/rustlib/bpfel-unknown-none/lib",
                "-o", "/home/dave/dev/rh-next/rh-next-ebpf/../target/bpfel-unknown-none/release/deps/rh_next-53bd6e44bb1b7225",
                "-O3", "--debug"
            ]
        },
    ]
}
```

## License

bpf-linker is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
