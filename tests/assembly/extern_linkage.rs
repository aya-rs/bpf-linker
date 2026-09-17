// assembly-output: bpf-linker
// revisions: ir btf
// compile-flags: --crate-type cdylib -C link-arg=tests/assembly/auxiliary/extern.ll
//[ir] compile-flags: -C link-arg=--emit=llvm-ir -C link-arg=-O3
//[btf] compile-flags: -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]

// aux-build: loop-panic-handler.rs
extern crate loop_panic_handler;

// The IR fixture supplies declaration debug info that Rust does not emit.
extern "C" {
    fn call_external(value: i32) -> i32;
}

#[no_mangle]
#[link_section = "tc"]
pub fn program(value: i32) -> i32 {
    unsafe { call_external(value) }
}

// ir-DAG: @external_variable = external global i32
// ir-DAG: @weak_variable = extern_weak global i32
// ir-DAG: declare i32 @external_function(i32)
// ir-DAG: declare extern_weak i32 @weak_function(i32)
// ir-DAG: call i32 @external_function(i32 [[VALUE:%[^ ,)]+]])
// ir-DAG: call i32 @weak_function(i32 [[VALUE]])

// btf: #[[EXTERN:[0-9]+]]: <FUNC> 'external_function' --> extern
// LLVM can also emit __bpf_trap in .ksyms.
// btf: <DATASEC> '.ksyms'
// btf: off:0 sz:0 --> {{\[}}[[EXTERN]]{{\]}}
