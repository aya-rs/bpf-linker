// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

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

// CHECK: <STRUCT> '<anon>' sz:8 n:2
// CHECK-NEXT: 'ayy' off:0 --> [{{[0-9]+}}]
// CHECK-NEXT: 'lmao' off:32 --> [{{[0-9]+}}]
