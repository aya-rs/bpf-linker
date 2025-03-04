// assembly-output: bpf-linker
// no-prefer-dynamic
// compile-flags: --crate-type bin -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]
#![no_main]

use core::ffi::c_void;

#[no_mangle]
static mut FOO: *mut c_void = core::ptr::null_mut();

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// We check the BTF dump out of btfdump
// CHECK-NOT: <ENUM> 'c_void'
