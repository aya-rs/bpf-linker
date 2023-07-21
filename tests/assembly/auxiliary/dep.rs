// no-prefer-dynamic
// compile-flags: --crate-type rlib
#![no_std]

#[no_mangle]
fn some_dep() -> u8 {
    42
}
