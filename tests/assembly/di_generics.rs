// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=llvm-ir -C debuginfo=2

// Verify that the linker correctly massages map names.
#![no_std]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

struct Foo<T> {
    x: T,
}

#[no_mangle]
#[link_section = "maps"]
static mut FOO: Foo<u32> = Foo { x: 0 };

struct Bar<T> {
    x: T,
}

#[no_mangle]
#[link_section = "maps"]
static mut BAR: Bar<Foo<u32>> = Bar { x: Foo { x: 0 } };

// CHECK: name: "Foo_3C_u32_3E_"
// CHECK: name: "Bar_3C_di_5F_generics_3A__3A_Foo_3C_u32_3E__3E_"
