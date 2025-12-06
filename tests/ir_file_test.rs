#![expect(unused_crate_dependencies, reason = "used in lib/bin")]

use std::ffi::CString;

use bpf_linker::{Linker, LinkerInput, LinkerOptions, OutputType};

fn create_test_ir_content(name: &str) -> String {
    format!(
        r#"; ModuleID = '{name}'
source_filename = "{name}"
target datalayout = "e-m:e-p:64:64-i64:64-i128:128-n32:64-S128"
target triple = "bpf"

define i32 @test_{name}(i32 %x) #0 {{
entry:
  %result = add i32 %x, 1
  ret i32 %result
}}

attributes #0 = {{ noinline nounwind optnone }}

!llvm.module.flags = !{{!0}}
!0 = !{{i32 1, !"wchar_size", i32 4}}
"#
    )
}

#[test]
fn test_link_ir_file() {
    let ir_content = create_test_ir_content("alessandro");

    let options = LinkerOptions {
        target: None,
        cpu: bpf_linker::Cpu::Generic,
        cpu_features: CString::default(),
        optimize: bpf_linker::OptLevel::No,
        unroll_loops: false,
        ignore_inline_never: false,
        llvm_args: vec![],
        disable_expand_memcpy_in_order: false,
        disable_memory_builtins: false,
        btf: false,
        allow_bpf_trap: false,
    };

    let linker = Linker::new(options);

    let result = linker.link_to_buffer(
        [LinkerInput::Buffer {
            name: "alessandro.ll",
            bytes: ir_content.as_bytes(),
        }],
        OutputType::Object,
        ["test_alessandro"],
    );

    assert!(
        result.is_ok(),
        "Linking IR should succeed: {:?}",
        result.err()
    );

    let output = result.unwrap();
    assert!(!output.as_slice().is_empty(), "Output should not be empty");
}

#[test]
fn test_invalid_ir_file() {
    let valid_content = create_test_ir_content("alessandro");

    let invalid_content =
        valid_content.replace("; ModuleID = 'alessandro'", ": ModuleXX = 'corrupted'");

    let options = LinkerOptions {
        target: None,
        cpu: bpf_linker::Cpu::Generic,
        cpu_features: CString::default(),
        optimize: bpf_linker::OptLevel::No,
        unroll_loops: false,
        ignore_inline_never: false,
        llvm_args: vec![],
        disable_expand_memcpy_in_order: false,
        disable_memory_builtins: false,
        btf: false,
        allow_bpf_trap: false,
    };

    let linker = Linker::new(options);

    let result = linker.link_to_buffer(
        [LinkerInput::Buffer {
            name: "corrupted.ll",
            bytes: invalid_content.as_bytes(),
        }],
        OutputType::Object,
        Vec::<&str>::new(),
    );

    assert!(result.is_err(), "Linking corrupted IR should fail");
}
