// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--unroll-loops -C link-arg=-O3
#![no_std]

// Recent kernels starting from 5.2 support some bounded loops. Older kernels need all loops to be
// unrolled. The linker provides the --unroll-loops flag to aggressively try and unroll.

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

#[no_mangle]
fn foo(arg: &mut u64) {
    for i in 0..=200 {
        *arg += i;
        // CHECK: r{{[1-9]}} += 200
    }
}
