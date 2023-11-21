// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=obj -C debuginfo=2

#![no_std]

pub enum DataCarryingEnum {
    First { a: u32, b: i32 },
    Second(u32, i32),
    Third(u32),
}

#[no_mangle]
pub static A: DataCarryingEnum = DataCarryingEnum::First { a: 54, b: -23 };
#[no_mangle]
pub static B: DataCarryingEnum = DataCarryingEnum::Second(54, -23);
#[no_mangle]
pub static C: DataCarryingEnum = DataCarryingEnum::Third(36);

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// The data-carrying enum should be removed from BTF.

// CHECK-NOT: ENUM 'DataCarryingEnum'
