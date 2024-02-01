// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]

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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// The data-carrying enum should be not included in BTF.

// CHECK: ENUM 'SimpleEnum'{{.*}} size=1 vlen=3
// CHECK-NEXT: 'First' val=0
// CHECK-NEXT: 'Second' val=1
// CHECK-NEXT: 'Third' val=2
// CHECK-NOT: ENUM 'DataCarryingEnum'
