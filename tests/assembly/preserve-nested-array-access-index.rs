// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=llvm-ir -C link-arg=--btf -C debuginfo=2

#![no_std]

use core::{marker::PhantomData, panic::PanicInfo};

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {}
}

#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct Relocatable(PhantomData<()>);

#[repr(C)]
pub struct Bar {
    x: u32,
    y: u32,
}

#[repr(C)]
pub struct Foo {
    arr: [Bar; 4],

    _marker: Relocatable,
}

// CHECK: @"llvm.Foo:0:20$0:0:2:1" = external global i64, !llvm.preserve.access.index ![[FOO:[0-9]+]] #[[AMA:[0-9]+]]

// CHECK-LABEL: define i32 @get_arr_2_y(
#[no_mangle]
#[link_section = "uprobe/get_arr_2_y"]
pub unsafe extern "C" fn get_arr_2_y(x: *const Foo) -> u32 {
    // CHECK: %[[OFF:[0-9]+]] = load i64, ptr @"llvm.Foo:0:20$0:0:2:1", align 8
    // CHECK-NEXT: %[[FIELD_PTR:[0-9]+]] = getelementptr i8, ptr %{{.*}}, i64 %[[OFF]]
    // CHECK-NEXT: %[[PASSTHROUGH:[0-9]+]] = tail call ptr @llvm.bpf.passthrough{{.*}}(i32 {{[0-9]+}}, ptr %[[FIELD_PTR]])
    // CHECK-NEXT: %{{.*}} = load i32, ptr %[[PASSTHROUGH]], align 4
    (*x).arr[2].y
}

// CHECK: attributes #[[AMA]] = { "btf_ama" }
// CHECK: ![[FOO]] = !DICompositeType(tag: DW_TAG_structure_type, name: "Foo"
