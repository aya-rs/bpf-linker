// no-prefer-dynamic
// compile-flags: --crate-type rlib
#![no_std]

#[unsafe(no_mangle)]
#[unsafe(link_section = "uprobe/dep")]
pub fn dep() -> u64 {
    42
}
