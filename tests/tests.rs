extern crate compiletest_rs as compiletest;

use std::{env, path::PathBuf};

fn run_mode(mode: &'static str) {
    let mut config = compiletest::Config::default();

    let mut rustc_flags = format!("-C linker={}", env!("CARGO_BIN_EXE_bpf-linker"));
    let host_target = env::var("TESTS_HOST_TARGET")
        .ok()
        .unwrap_or_else(|| "0".to_string())
        == "1";
    if host_target {
        rustc_flags += " -C linker-plugin-lto -C linker-flavor=wasm-ld -C panic=abort -C link-arg=--target=bpf";
    } else {
        config.target = "bpfel-unknown-none".to_string();
    }
    config.target_rustcflags = Some(rustc_flags);
    config.llvm_filecheck = Some("FileCheck-12".into());
    config.mode = mode.parse().expect("Invalid mode");
    config.src_base = PathBuf::from(format!("tests/{}", mode));
    config.link_deps(); // Populate config.target_rustcflags with dependencies on the path
    //config.clean_rmeta(); // If your tests import the parent crate, this helps with E0464

    compiletest::run_tests(&config);
}

#[test]
fn compile_test() {
    run_mode("assembly");
}
