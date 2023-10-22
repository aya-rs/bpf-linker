// assembly-output: bpf-linker
// revisions: cdylib bin
// [cdylib]compile-flags: --crate-type cdylib
// [bin]compile-flags: --crate-type bin
//
// When compiling cdylibs or bins, only #[no_mangle] symbols are exported.
//
// Dylibs aren't supported, since they re-export all public symbols of all statically linked
// dependencies. In practice that means that all of the core crate is re-exported, which crashes the
// LLVM BPF target because of varargs, > 5 arguments, and all the other usual reasons. See
// https://github.com/rust-lang/rust/blob/0039d73/compiler/rustc_codegen_ssa/src/back/linker.rs#L1685
//
// staticlib and rlib don't link, they just store bitcode in .a/.rlib so they are not relevant.
//
//
#![no_std]
#![no_main]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

// aux-build: dep-exports.rs
extern crate dep_exports as dep;

pub use dep::dep_public_symbol as local_re_exported;

#[no_mangle]
fn local_no_mangle() -> u8 {
    dep::dep_public_symbol()
}

pub fn local_public() -> u8 {
    dep::dep_public_symbol()
}

// #[no_mangle] symbols are exported
// CHECK,cdylib: .globl local_no_mangle
// CHECK,cdylib: .globl dep_no_mangle
// CHECK,bin: .globl local_no_mangle
// CHECK,bin: .globl dep_no_mangle

// public symbols are not exported
// public symbols of dependencies are not exported
// re-exported symbols are not exported
// CHECK,cdylib-NOT: .globl local_public
// CHECK,cdylib-NOT: .globl dep_public_symbol
// CHECK,cdylib-NOT: .globl local_re_exported
// CHECK,bin-NOT: .globl local_public
// CHECK,bin-NOT: .globl dep_public_symbol
// CHECK,bin-NOT: .globl local_re_exported
