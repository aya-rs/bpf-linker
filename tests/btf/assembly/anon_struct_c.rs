//! Check if bpf-linker is able to link bitcode which provides anonymous structs
//! exposed by named typedefs. The corresponding C code is available in
//! tests/c/anon.c.

// assembly-output: bpf-linker
// compile-flags: --crate-type bin -C link-arg=--emit=obj -C debuginfo=2 -Z unstable-options -L native=target/bitcode -l link-arg=target/bitcode/anon.bc

#![no_std]
#![no_main]

#[no_mangle]
static EXPECTED_FOO: i32 = 0;

/// A binding to the struct from C code.
///
/// In Rust, there is no concept of anonymous structs and typedef aliases
/// (`type` in Rust works differently and also produces different debug info).
/// Just defining a named struct is a correct way of creating a binding and
/// that's exactly what bindgen does.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Incognito {
    pub foo: i32,
}

extern "C" {
    pub fn incognito_foo(i: *const Incognito) -> i32;
}

#[no_mangle]
pub fn get_foo(i: *const Incognito) -> i32 {
    unsafe { incognito_foo(i) }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// CHECK: [9] TYPEDEF 'incognito' type_id=10
// CHECK: [10] STRUCT '(anon)' size=4 vlen=1
