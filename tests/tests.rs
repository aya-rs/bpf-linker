use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

fn run_mode<F: Fn(&mut compiletest_rs::Config)>(
    target: &str,
    mode: &str,
    sysroot: Option<&Path>,
    cfg: Option<F>,
) {
    let mut target_rustcflags = format!("-C linker={}", env!("CARGO_BIN_EXE_bpf-linker"));
    if let Some(sysroot) = sysroot {
        let sysroot = sysroot.to_str().unwrap();
        target_rustcflags += &format!(" --sysroot {sysroot}");
    }

    let llvm_filecheck_re_str = r"^FileCheck(-\d+)?$";
    let llvm_filecheck_re = regex::Regex::new(llvm_filecheck_re_str).unwrap();
    let mut llvm_filecheck = which::which_re(llvm_filecheck_re).expect(llvm_filecheck_re_str);
    let llvm_filecheck = llvm_filecheck.next();
    assert_ne!(
        llvm_filecheck, None,
        "Could not find {llvm_filecheck_re_str}"
    );

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

    if let Some(cfg) = cfg {
        cfg(&mut config);
    }

    compiletest_rs::run_tests(&config);
}

fn btf_dump(src: &Path, dst: &Path) {
    let dst = std::fs::File::create(dst)
        .unwrap_or_else(|err| panic!("could not open btf dump file '{}': {err}", dst.display()));
    let mut bpftool = Command::new("bpftool");
    bpftool
        .arg("btf")
        .arg("dump")
        .arg("file")
        .arg(src)
        .stdout(dst);
    let status = bpftool
        .status()
        .unwrap_or_else(|err| panic!("could not run {bpftool:?}: {err}",));
    assert_eq!(status.code(), Some(0), "{bpftool:?} failed");
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
        .rustflag("-Cdebuginfo=2")
        .build_from_source(&rustc_src)
        .expect("failed to build sysroot");

    run_mode(
        target,
        "assembly",
        Some(&directory),
        None::<fn(&mut compiletest_rs::Config)>,
    );
    run_mode(
        target,
        "assembly",
        Some(&directory),
        Some(|cfg: &mut compiletest_rs::Config| {
            cfg.src_base = PathBuf::from("tests/btf");
            cfg.llvm_filecheck_preprocess = Some(btf_dump);
        }),
    );
}
