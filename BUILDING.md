# Building bpf-linker from source

## LLVM

bpf-linker is a bitcode linker that uses libLLVM to link bitcode inputs. That
means the LLVM version used by bpf-linker must match the LLVM version used by
the Rust toolchain you intend to use.

There are four recommended ways of obtaining an appropriate LLVM.

### Our prebuilt LLVM on ghcr.io

We regularly build LLVM in CI and publish the binary artifacts on ghcr.io.
They can be retrieved using [oras][oras].

First, pick an appropriate image from our [LLVM container page][containers-llvm].
The tags mention the LLVM version, the platform, and our custom revision, e.g.

* `23-x86_64-unknown-linux-gnu-4` - LLVM 23, x86_64 Linux, glibc, revision 4
* `21-aarch64-unknown-linux-musl-4` - LLVM 21, aarch64 Linux, musl, revision 4
* `22-aarch64-apple-darwin-4` - LLVM 22, aarch64 macOS, revision 4

Always pick the latest revision available, if there are multiple.

After picking an appropriate image, it can be downloaded with oras, e.g.

```sh
oras pull ghcr.io/aya-rs/llvm:23-x86_64-unknown-linux-gnu-4
```

And the resulting tarball unpacked to a directory:

```sh
mkdir llvm
tar --zstd -xpf llvm-archive-x86_64-unknown-linux-gnu.tar.zst \
    -C llvm/
```

[oras]: https://oras.land/
[containers-llvm]: https://github.com/aya-rs/bpf-linker/pkgs/container/llvm/versions

### Building LLVM with Bazel

The repository's Bazel graph builds the pinned LLVM revision from source and
can package its outputs in the same layout as the ghcr.io artifact. It's
recommended to disable ThinLTO to avoid linker errors if your system linker
uses a different LLVM version than the one you're building.

```sh
bazel build //:llvm-archive --config=release --features=-thin_lto
mkdir llvm
tar --zstd -xpf bazel-bin/llvm-archive.tar.zst -C llvm/
```

The archive contains FileCheck, the shared LLVM library, and its static
component libraries. It is intended for building and debugging bpf-linker,
rather than as a complete LLVM developer installation.

### Building LLVM with CMake

If you prefer to build LLVM the way it is built upstream, check out the
appropriate branch from
[rust-lang/llvm-project](https://github.com/rust-lang/llvm-project/) for the
LLVM version used by your Rust toolchain, then configure and install it with
CMake:

```sh
cmake -S llvm-project/llvm -B llvm-build -G Ninja \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DCMAKE_INSTALL_PREFIX="$PWD/llvm" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DCMAKE_C_COMPILER=clang \
    -DCMAKE_CXX_COMPILER=clang++ \
    -DLLVM_BUILD_LLVM_DYLIB=ON \
    -DLLVM_ENABLE_ASSERTIONS=ON \
    -DLLVM_ENABLE_PROJECTS= \
    -DLLVM_ENABLE_RUNTIMES= \
    -DLLVM_INSTALL_UTILS=ON \
    -DLLVM_LINK_LLVM_DYLIB=ON \
    -DLLVM_TARGETS_TO_BUILD=BPF \
    -DLLVM_USE_LINKER=lld
cmake --build llvm-build --target install
export LLVM_PREFIX="$PWD/llvm"
```

### System packages

On Debian-based distributions, you can install the `llvm-<version>-dev` and
`libclang-<version>-dev` packages from the official LLVM repository at
https://apt.llvm.org.

Different operating systems and Linux distributions might provide their own
LLVM packages. If you're interested in packaging bpf-linker, you may also need
to ensure that the correct LLVM version is packaged for that environment.

## Building bpf-linker with Cargo

bpf-linker uses Cargo features to select the LLVM version, via `llvm-*`
features such as `llvm-23`. By default, LLVM and its dependencies are linked
dynamically. Static linking can be enabled with the `llvm-link-static` feature.

If you used either of the first two methods of obtaining LLVM, set the
`LLVM_PREFIX` variable to point to the LLVM prefix inside the extracted
archive. When linking statically, also configure the names and locations of
the Bazel-built libraries:

```sh
export LLVM_PREFIX="$PWD/llvm"
export CXXSTDLIB=libcxx,libcxxabi
export CXXSTDLIB_PATH="$PWD/llvm/lib"
export ZLIB_PATH="$PWD/llvm/lib"
export LIBZSTD_PATH="$PWD/llvm/lib"
```

Installations that provide `llvm-config` are also discovered through `PATH`.

Examples:

```
# Dynamic linking
cargo build --no-default-features --features llvm-23
cargo install bpf-linker --no-default-features --features llvm-23
cargo install --path . --no-default-features --features llvm-23

# Static linking
cargo build --no-default-features --features llvm-23,llvm-link-static
cargo install bpf-linker --no-default-features --features llvm-23,llvm-link-static
cargo install --path . --no-default-features --features llvm-23,llvm-link-static
```

## Running tests

bpf-linker comes with compiletests, similar to the ones in Rust and LLVM, that
compile the code to LLVM IR (or BTF) and assert the output matches the
expected IR.

### With Rust nightly

Use `cargo test` with same arguments as used for build, e.g.:

```
cargo +nightly test --no-default-features --features llvm-23
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
RUSTC_BOOTSTRAP=1 cargo test --no-default-features --features llvm-23
```
