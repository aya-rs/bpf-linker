//! Check if bpf-linker is able to link bitcode which provides anonymous structs
//! exposed by named typedefs. The IR (and corresponding C code) is available in
//! tests/ir/anon.ll.

// assembly-output: bpf-linker
// compile-flags: --crate-type bin -C link-arg=--emit=obj -C debuginfo=2 -Z unstable-options -L native=tests/ir -l link-arg=tests/ir/anon.bc

#![no_std]
#![no_main]

use core::ffi::{c_int, c_void};

#[no_mangle]
static FORBIDDEN_PID: c_int = 0;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct task {
    pub pid: c_int,
}

extern "C" {
    pub fn task_pid(t: *const task) -> c_int;
}

unsafe fn arg<T>(ctx: *mut c_void, n: usize) -> *const T {
    *(ctx as *const usize).add(n) as *const T
}

#[no_mangle]
#[link_section = "lsm/task_alloc"]
pub fn task_alloc(ctx: *mut c_void) -> i32 {
    let task = unsafe { arg(ctx, 0) };

    if unsafe { task_pid(task) } == FORBIDDEN_PID {
        return -1;
    }

    0
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// CHECK: [9] TYPEDEF 'task' type_id=10
// CHECK: [10] STRUCT '(anon)' size=4 vlen=1
