// assembly-output: bpf-linker
// compile-flags: --crate-type bin -C link-arg=--emit=obj -C debuginfo=2 -C link-arg=--log-level=debug -C link-arg=--log-file=/tmp/linker-lol.log -C link-arg=--btf
#![no_std]
#![no_main]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

// aux-build: dep-exports.rs
extern crate dep_exports as dep;

pub use dep::dep_public_symbol as local_re_exported;

#[no_mangle]
fn local_no_mangle() -> u8 {
    local_public(1, 2)
}

#[inline(never)]
pub fn local_public(_arg1: u32, _arg2: u32) -> u8 {
    // bind v so we create a debug variable which needs its scope to be fixed
    let v = dep::dep_public_symbol();
    // call inline functions so we get inlinedAt scopes to be fixed
    inline_function_1(v) + inline_function_2(v)
}

#[inline(always)]
fn inline_function_1(v: u8) -> u8 {
    unsafe { core::ptr::read_volatile(v as *const u8) }
}

#[inline(always)]
fn inline_function_2(v: u8) -> u8 {
    inline_function_1(v)
}

// #[no_mangle] functions keep linkage=global
// CHECK: FUNC 'local_no_mangle' type_id={{[0-9]+}} linkage=global

// check that parameter names are preserved
// CHECK: FUNC_PROTO
// CHECK-NEXT: _arg1
// CHECK-NEXT: _arg2

// public functions get static linkage
// CHECK: FUNC '{{.*}}local_public{{.*}}' type_id={{[0-9]+}} linkage=static
// CHECK: FUNC '{{.*}}dep_public_symbol{{.*}}' type_id={{[0-9]+}} linkage=static

// #[no_mangle] is honored for dep functions
// CHECK: FUNC 'dep_no_mangle' type_id={{[0-9]+}} linkage=global
