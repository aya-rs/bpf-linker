// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=llvm-ir -C link-arg=--btf -C debuginfo=2

#![no_std]

use core::{marker::PhantomData, panic::PanicInfo};

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {}
}

#[repr(transparent)]
// Union fields must not have drop side effects, deriving `Copy` achieves that.
#[derive(Clone, Copy)]
pub struct Relocatable(PhantomData<()>);

#[repr(C)]
pub union Foo {
    a: u32,
    b: u64,

    _relocatable: Relocatable,
}

// CHECK: @"llvm.Foo{{.*}}" = external global i64, !llvm.preserve.access.index ![[FOO:[0-9]+]] #[[AMA:[0-9]+]]

// CHECK-LABEL: define i64 @get_b(
#[no_mangle]
#[link_section = "uprobe/get_b"]
pub unsafe extern "C" fn get_b(x: *const Foo) -> u64 {
    // CHECK: %[[OFF:[0-9]+]] = load i64, ptr @"llvm.Foo{{.*}}", align 8
    // CHECK-NEXT: %[[FIELD_PTR:[0-9]+]] = getelementptr i8, ptr %{{.*}}, i64 %[[OFF]]
    // CHECK-NEXT: %[[PASSTHROUGH:[0-9]+]] = tail call ptr @llvm.bpf.passthrough{{.*}}(i32 {{[0-9]+}}, ptr %[[FIELD_PTR]])
    // CHECK-NEXT: %{{.*}} = load i64, ptr %[[PASSTHROUGH]], align 8
    (*x).b
}

// CHECK: attributes #[[AMA]] = { "btf_ama" }
// CHECK: ![[FOO]] = !DICompositeType(tag: DW_TAG_union_type, name: "Foo"
