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

// CHECK: name: "Foo_l_u32_l_"
// CHECK: name: "Bar_l_di_generics_Foo_l_u32_l__l_"
