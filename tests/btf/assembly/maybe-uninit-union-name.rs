// assembly-output: bpf-linker
// compile-flags: --crate-type cdylib -C link-arg=--emit=obj -C link-arg=--btf -C debuginfo=2

#![no_std]

#[no_mangle]
pub static GLOBAL: core::mem::MaybeUninit<u8> = core::mem::MaybeUninit::new(1);

// Ensure generic union names are sanitized for BTF compatibility.
// CHECK: <UNION> 'MaybeUninit_3C_u8_3E_'
