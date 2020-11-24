// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib
#![no_std]

// Check that #[no_mangle] symbols are exported, including those coming from dependency crates.

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

// aux-build: dep.rs
extern crate dep;

#[no_mangle]
fn connect() {
}

// CHECK: .globl connect
// CHECK: .globl some_dep