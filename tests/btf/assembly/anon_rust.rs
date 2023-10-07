// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=obj -C debuginfo=2

#![no_std]

use core::marker::PhantomData;

#[repr(transparent)]
pub struct AyaBtfMapMarker(PhantomData<()>);

pub struct Foo {
    // Anonymize the stuct.
    _anon: AyaBtfMapMarker,

    pub ayy: u32,
    pub lmao: u32,
}

#[no_mangle]
static FOO: Foo = Foo {
    _anon: AyaBtfMapMarker(PhantomData),

    ayy: 0,
    lmao: 0,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// CHECK: [16] STRUCT '(anon)' size=8 vlen=2
// CHECK-NEXT: 'ayy' type_id=17 bits_offset=0
// CHECK-NEXT: 'lmao' type_id=17 bits_offset=32
