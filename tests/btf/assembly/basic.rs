// assembly-output: bpf-linker
// no-prefer-dynamic
// compile-flags: --crate-type bin -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]
#![no_main]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

#[no_mangle]
#[link_section = "uprobe/connect"]
pub fn connect() {}

// We check the BTF dump out of btfdump
// CHECK: <FUNC> 'connect' --> global
