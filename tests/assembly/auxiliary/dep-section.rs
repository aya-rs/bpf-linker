// no-prefer-dynamic
// compile-flags: --crate-type rlib
#![no_std]

#[no_mangle]
#[link_section = "uprobe/dep"]
pub fn dep() -> u64 {
    42
}
