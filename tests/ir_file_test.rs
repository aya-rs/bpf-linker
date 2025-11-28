#![expect(unused_crate_dependencies, reason = "used in lib/bin")]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn linker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bpf-linker"))
}

fn create_test_ir_file(dir: &Path, name: &str) -> PathBuf {
    let ir_path = dir.join(format!("{}.ll", name));
    let ir_content = format!(
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
    );
    fs::write(&ir_path, ir_content).expect("Failed to write test IR file");
    ir_path
}

#[test]
fn test_link_ir_file() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let ir_file = create_test_ir_file(temp_dir.path(), "alessandro");
    let output_file = temp_dir.path().join("output.o");

    let output = Command::new(linker_path())
        .arg("--export")
        .arg(format!("test_{}", "alessandro"))
        .arg(&ir_file)
        .arg("-o")
        .arg(&output_file)
        .output()
        .expect("Failed to execute bpf-linker");

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        panic!("bpf-linker failed with status: {}", output.status);
    }

    assert!(
        output_file.exists(),
        "Output file should exist: {:?}",
        output_file
    );
    assert!(
        output_file.metadata().unwrap().len() > 0,
        "Output file should not be empty"
    );
}

#[test]
fn test_invalid_ir_file() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");

    let valid_ir_file = create_test_ir_file(temp_dir.path(), "alessandro");

    let valid_content = fs::read_to_string(valid_ir_file).expect("Failed to read valid IR file");

    // Corrupting IR content
    let invalid_content =
        valid_content.replace("; ModuleID = 'alessandro'", ": ModuleXX = 'corrupted'");

    let invalid_ir_file = temp_dir.path().join("corrupted.ll");

    fs::write(&invalid_ir_file, invalid_content).expect("Failed to write invalid IR file");

    let output_file = temp_dir.path().join("output.o");

    let output = Command::new(linker_path())
        .arg(&invalid_ir_file)
        .arg("-o")
        .arg(&output_file)
        .output()
        .expect("Failed to execute bpf-linker");

    // Should fail with corrupted IR
    assert!(
        !output.status.success(),
        "bpf-linker should fail with corrupted IR. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
