// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=llvm-ir

#![no_std]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

// Extern declarations
extern "C" {
    fn bpf_kfunc_call_test_acquire(arg: *mut u64) -> *mut u64;
    fn bpf_kfunc_call_test_release(arg: *mut u64);
    static bpf_prog_active: u32;
    static CONFIG_HZ: u64;
}

#[no_mangle]
#[link_section = "tc"]
pub fn test_extern_symbols() -> u64 {
    unsafe {
        let mut val: u64 = 42;
        let ptr = bpf_kfunc_call_test_acquire(&mut val as *mut u64);
        bpf_kfunc_call_test_release(ptr);
        
        let active = core::ptr::read_volatile(&bpf_prog_active);
        let hz = core::ptr::read_volatile(&CONFIG_HZ);
        active as u64 + hz
    }
}


// Verify extern variables: external, not internal
// CHECK: @bpf_prog_active = external{{.*}}global i32{{.*}}section ".ksyms"
// CHECK: @CONFIG_HZ = external{{.*}}global i64{{.*}}section ".ksyms"
// CHECK-NOT: @bpf_prog_active = internal
// CHECK-NOT: @CONFIG_HZ = internal
// Verify extern functions preserve linkage/calling convention/function signature
// CHECK: declare ptr @bpf_kfunc_call_test_acquire(ptr){{.*}}section ".ksyms"
// CHECK: declare void @bpf_kfunc_call_test_release(ptr){{.*}}section ".ksyms"
// CHECK-NOT: declare internal{{.*}}@bpf_kfunc_call_test_acquire unnamed_addr #0
// CHECK-NOT: declare internal{{.*}}@bpf_kfunc_call_test_release unnamed_addr #0
// CHECK-NOT: declare{{.*}}fastcc{{.*}}@bpf_kfunc_call_test_acquire unnamed_addr #0 
// CHECK-NOT: declare{{.*}}fastcc{{.*}}@bpf_kfunc_call_test_release unnamed_addr #0
