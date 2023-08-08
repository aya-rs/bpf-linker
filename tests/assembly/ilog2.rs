// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--unroll-loops -C link-arg=-O3
#![no_std]
// bpf target did not historically support the ilog2 operation because LLVM did
// not have a way to do leading/trailing bits

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

#[no_mangle]
fn foo(arg: &mut u64) {
    *arg = arg.ilog2() as u64;
}
