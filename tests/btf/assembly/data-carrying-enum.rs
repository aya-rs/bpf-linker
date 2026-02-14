// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

pub enum SimpleEnum {
    First,
    Second,
    Third,
}

pub enum DataCarryingEnum {
    First { a: u32, b: i32 },
    Second(u32, i32),
    Third(u32),
}

#[no_mangle]
pub static A: SimpleEnum = SimpleEnum::First;
#[no_mangle]
pub static B: SimpleEnum = SimpleEnum::Second;
#[no_mangle]
pub static C: SimpleEnum = SimpleEnum::Third;

#[no_mangle]
pub static X: DataCarryingEnum = DataCarryingEnum::First { a: 54, b: -23 };
#[no_mangle]
pub static Y: DataCarryingEnum = DataCarryingEnum::Second(54, -23);
#[no_mangle]
pub static Z: DataCarryingEnum = DataCarryingEnum::Third(36);

// The data-carrying enum should be not included in BTF.

// CHECK: <ENUM> 'SimpleEnum' sz:1 n:3
// CHECK-NEXT: First = 0
// CHECK-NEXT: Second = 1
// CHECK-NEXT: Third = 2
// CHECK-NOT: <ENUM> 'DataCarryingEnum'
