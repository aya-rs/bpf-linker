use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};
use which::which;

fn run_mode(target: &str, mode: &str, sysroot: Option<&Path>) {
    let linker = env!("CARGO_BIN_EXE_bpf-linker");
    let mut target_rustcflags = format!("-C linker={linker} -C link-arg=--fatal-errors=true");
    if let Some(sysroot) = sysroot {
        let sysroot = sysroot.to_str().unwrap();
        target_rustcflags += &format!(" --sysroot {sysroot}");
    }
    let llvm_filecheck = if let Ok(filecheck) = which("FileCheck") {
        Some(filecheck)
    } else if let Ok(filecheck) = which("FileCheck-16") {
        Some(filecheck)
    } else {
        panic!("no FileCheck binary found");
    };
    let mode = mode.parse().expect("Invalid mode");
    let mut config = compiletest_rs::Config {
        target: target.to_owned(),
        target_rustcflags: Some(target_rustcflags),
        llvm_filecheck,
        mode,
        src_base: PathBuf::from(format!("tests/{}", mode)),
        ..Default::default()
    };
    config.link_deps();

    compiletest_rs::run_tests(&config);
}

#[test]
fn compile_test() {
    let target = "bpfel-unknown-none";
    let rustc =
        std::process::Command::new(env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")));
    let rustc_src = rustc_build_sysroot::rustc_sysroot_src(rustc)
        .expect("could not determine sysroot source directory");
    let mut directory = env::current_dir().expect("could not determine current directory");
    directory.push("target/sysroot");
    let () = rustc_build_sysroot::SysrootBuilder::new(&directory, target)
        .build_mode(rustc_build_sysroot::BuildMode::Build)
        .sysroot_config(rustc_build_sysroot::SysrootConfig::NoStd)
        .build_from_source(&rustc_src)
        .expect("failed to build sysroot");

    run_mode(target, "assembly", Some(&directory));
}
