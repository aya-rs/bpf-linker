// assembly-output: emit-asm
// compile-flags: --crate-type rlib
#![no_std]

#[no_mangle]
fn panic_fun() -> u32 {
    // CHECK-LABEL: panic_fun:
    // CHECK: ud2
    panic!();
}