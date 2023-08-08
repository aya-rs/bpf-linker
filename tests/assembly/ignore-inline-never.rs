// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-args=--ignore-inline-never
// only-bpfel
#![no_std]

// Kernels prior to 5.8 (August 2020) don't support function calls. In order to support those
// kernels, which are the vast majority of versions deployed today, the linker provides the
// --ignore-inline-never option to ignore #[inline(never)] attributes. The flag is useful when
// something in a dependency crate is marked #[inline(never)], like core::panicking::*.

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

#[inline(never)]
fn actually_inlined(a: u64) -> u64 {
    a + 42
}

#[no_mangle]
#[link_section = "uprobe/fun"]
pub extern "C" fn fun(a: u64) -> u64 {
    // CHECK-LABEL: fun:
    actually_inlined(a)
    // CHECK: r{{[0-9]}} = r{{[0-9]}}
    // CHECK-NEXT: r{{[0-9]}} += 42
}
