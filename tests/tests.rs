extern crate compiletest_rs as compiletest;

use std::{env, path::PathBuf};
use which::which;

fn run_mode(mode: &'static str) {
    let mut config = compiletest::Config::default();

    let mut rustc_flags = format!("-C linker={}", env!("CARGO_BIN_EXE_bpf-linker"));
    // Default to the host target backdoor so that tests can run locally without building rustc from source.
    let host_target = env::var_os("TESTS_HOST_TARGET").map_or(true, |v| v == "1");
    if host_target {
        rustc_flags = [
            rustc_flags.as_str(),
            "-C link-arg=--target=bpf",
            "-C linker-flavor=bpf-linker",
            "-C linker-plugin-lto",
            "-C panic=abort",
            "-C target-cpu=generic",
            "-Z unstable-options",
        ]
        .join(" ");
    } else {
        config.target = "bpfel-unknown-none".to_string();
    }
    config.target_rustcflags = Some(rustc_flags);
    if let Ok(filecheck) = which("FileCheck") {
        config.llvm_filecheck = Some(filecheck)
    } else if let Ok(filecheck) = which("FileCheck-16") {
        config.llvm_filecheck = Some(filecheck)
    } else {
        panic!("no FileCheck binary found");
    };
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
