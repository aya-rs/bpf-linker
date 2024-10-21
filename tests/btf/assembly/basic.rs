// assembly-output: bpf-linker
// no-prefer-dynamic
// compile-flags: --crate-type bin -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]
#![no_main]

#[no_mangle]
#[link_section = "uprobe/connect"]
pub fn connect() {}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// We check the BTF dump out of btfdump
// CHECK: <FUNC> 'connect' --> global
