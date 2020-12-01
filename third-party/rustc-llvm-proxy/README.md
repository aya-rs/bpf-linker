# Rustc LLVM Proxy

[![Build Status](https://travis-ci.org/denzp/rustc-llvm-proxy.svg?branch=master)](https://travis-ci.org/denzp/rustc-llvm-proxy)
[![Build status](https://ci.appveyor.com/api/projects/status/4oxi872d3nir8ndk/branch/master?svg=true)](https://ci.appveyor.com/project/denzp/rustc-llvm-proxy)
[![Current Version](https://img.shields.io/crates/v/rustc-llvm-proxy.svg)](https://crates.io/crates/rustc-llvm-proxy)
[![Docs](https://docs.rs/rustc-llvm-proxy/badge.svg)](https://docs.rs/rustc-llvm-proxy)

Dynamically proxy LLVM calls into Rust own shared library! 🎉

## Use cases
Normally there is no much need for the crate, except a couple of exotic cases:

* Your crate is some kind build process helper that leverages LLVM (e.g. [ptx-linker](https://github.com/denzp/rust-ptx-linker)),
* Your crate needs to stay up to date with Rust LLVM version (again [ptx-linker](https://github.com/denzp/rust-ptx-linker)),
* You would prefer not to have dependencies on host LLVM libs (as always [ptx-linker](https://github.com/denzp/rust-ptx-linker)).

## Usage
First, you need to make sure no other crate links your binary against system LLVM library.
In case you are using `llvm-sys`, this can be achieved with a special feature:

``` toml
[dependencies.llvm-sys]
version = "60"
features = ["no-llvm-linking", "disable-alltargets-init"]
```

Then all you need to do is to include the crate into your project:

``` toml
[dependencies]
rustc-llvm-proxy = "0.2"
```

``` rust
extern crate rustc_llvm_proxy;
```
