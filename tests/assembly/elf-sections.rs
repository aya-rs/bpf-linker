// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib

// BPF crates export programs (functions) and maps (kernel data structures) through ELF sections.
// Verify that the linker correctly outputs those sections.
#![no_std]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

// aux-build: dep-section.rs
extern crate dep_section;

#[no_mangle]
#[link_section = "uprobe/connect"]
pub fn connect() {
}

#[no_mangle]
#[link_section = "maps/counter"]
static mut COUNTER: u32 = 0;

// CHECK: .section "uprobe/connect","ax"
// CHECK: .section "uprobe/dep","ax"
// CHECK: .section "maps/counter","aw"