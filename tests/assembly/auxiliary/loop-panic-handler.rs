// no-prefer-dynamic
// compile-flags: --crate-type rlib
#![no_std]

#[panic_handler]
fn panic_impl(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

