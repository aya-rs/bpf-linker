# BPF Linker 🔗

bpf-linker aims to simplify building modern BPF programs while still supporting
older, more restrictive kernels.

# Overview

bpf-linker can be used to statically link multiple BPF object files together
and optionally perform optimizations needed to target older kernels. It
operates on LLVM bitcode, so the inputs must be bitcode files (.bc) or object
files with embedded bitcode (.o), optionally stored inside ar archives (.a).

# Installation

The linker requires LLVM 11.

On Debian based distributions you need to install the `llvm-11-dev` and
`libclang-11-dev` packages. If your distro doesn't have them you can get them
from the official LLVM repo at https://apt.llvm.org.

On rpm based distribution you need the `llvm-devel` and `clang-devel` packages.
If your distro doesn't have them you can get them from Fedora Rawhide.

Once you have installed LLVM 11 you can install the linker running:
```
cargo install --git https://github.com/alessandrod/bpf-linker --rev origin/main
```

If you don't have cargo you can get it from https://rustup.rs or from your distro's package manager.

# Examples

## Rust

The ultimate goal is to have `rustc --target=bpf[el|eb]-unknown-none` invoke bpf-linker directly.

In the meantime you can already compile a crate for BPF by explicitly making rustc use bpf-linker:

```
$ cargo rustc --release -- \
        -C linker-plugin-lto \
        -C linker-flavor=wasm-ld -C linker=bpf-linker \
        -C link-arg=--target=bpf
   Compiling bpf-log-clone v0.1.0 (/home/alessandro/bpf-log-clone)
   Finished release [optimized] target(s) in 0.86s

$ file target/release/libbpf_log_clone.so
target/release/libbpf_log_clone.so: ELF 64-bit LSB relocatable, eBPF, version 1 (SYSV), not stripped
```

With `-C linker-plugin-lto` we instruct rustc to pass bitcode to the linker, with
`-C linker-flavor=wasm-ld -C linker=bpf-linker` we make the compiler use
bpf-linker, which conveniently implements a command line compatible with
wasm-ld so rustc knows how to invoke it.

# Clang

For a simple example of how to use the linker with clang see [this
gist](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b). In
the example
[lib.c](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-lib-c)
is compiled as a static library which is then linked by
[program.c](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-program-c).
The
[Makefile](https://gist.github.com/alessandrod/ed6f11ba41bcd8a19d8655e57a00350b#file-makefile)
shows how to compile the C code and then link it.

# Usage

```
bpf-linker

USAGE:
    bpf-linker [FLAGS] [OPTIONS] --output <output> [--] [inputs]...

FLAGS:
    -h, --help                   Prints help information
        --ignore-inline-never    Ignore `noinline`/`#[inline(never)]`. Useful when targeting kernels that don't support
                                 function calls
        --unroll-loops           Try hard to unroll loops. Useful when targeting kernels that don't support loops
    -V, --version                Prints version information

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

# License

bpf-linker is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.