#![allow(
    unused_crate_dependencies,
    reason = "Cargo exposes package-wide dependencies"
)]

use std::ffi::OsStr;

fn create_test_ir_content(name: &str) -> String {
    format!(
        r#"; ModuleID = '{name}'
source_filename = "{name}"
target datalayout = "e-m:e-p:64:64-i64:64-i128:128-n32:64-S128"
target triple = "bpfel-unknown-none"

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
fn test_link_ir_files() {
    let options = bpf_linker::LinkerOptions {
        target: None,
        cpu: bpf_linker::Cpu::Generic,
        cpu_features: Default::default(),
        optimize: bpf_linker::OptLevel::No,
        unroll_loops: false,
        ignore_inline_never: false,
        llvm_args: vec![],
        disable_expand_memcpy_in_order: false,
        disable_memory_builtins: false,
        btf: false,
        allow_bpf_trap: false,
    };

    let linker = bpf_linker::Linker::new(options);

    // Test 1: Valid IR should link successfully
    {
        let ir_content = create_test_ir_content("valid");

        assert_matches::assert_matches!(
            linker.link_to_buffer(
                [bpf_linker::LinkerInput::Buffer {
                    name: "valid.ll",
                    bytes: ir_content.as_bytes(),
                }],
                bpf_linker::OutputType::Object,
                ["test_valid"],
            ),
            Ok(output) if !output.is_empty()
        );
    }

    // Test 2: Invalid IR should fail to link
    {
        let valid_content = create_test_ir_content("invalid");
        let invalid_content =
            valid_content.replace("; ModuleID = 'invalid'", ": ModuleXX = 'corrupted'");

        assert_matches::assert_matches!(
            linker.link_to_buffer(
                [bpf_linker::LinkerInput::Buffer {
                    name: "corrupted.ll",
                    bytes: invalid_content.as_bytes(),
                }],
                bpf_linker::OutputType::Object,
                Vec::<&str>::new(),
            ),
            // TODO(MSRV 1.91.0): peel away the `AsRef::<OsStr>::as_ref`.
            // See https://doc.rust-lang.org/stable/std/path/struct.PathBuf.html#impl-PartialEq%3Cstr%3E-for-PathBuf.
            Err(bpf_linker::LinkerError::InvalidInputType(path)) if AsRef::<OsStr>::as_ref(&path) == "in_memory::corrupted.ll"
        );
    }
}
