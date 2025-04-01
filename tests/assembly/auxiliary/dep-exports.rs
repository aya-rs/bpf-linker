// no-prefer-dynamic
// compile-flags: --crate-type rlib
#![no_std]

pub fn dep_public_symbol() -> u8 {
    // read_volatile stops LTO inlining the function in the calling crate
    unsafe { core::ptr::read_volatile(core::ptr::dangling()) }
}

#[no_mangle]
pub fn dep_no_mangle() -> u8 {
    dep_public_symbol()
}
