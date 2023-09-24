use std::{
    env,
    ffi::OsString,
    fs,
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

/// Builds LLVM bitcode files from LLVM IR files located in a specified directory.
fn build_bitcode<P>(dir: P)
where
    P: AsRef<Path>,
{
    for entry in fs::read_dir(dir.as_ref()).expect("failed to read the directory") {
        let entry = entry.expect("failed to read the file");
        let path = entry.path();

        if path.is_file() && path.extension().unwrap_or_default() == "ll" {
            let bc_dst = path.with_extension("bc");
            llvm_as_build(path, bc_dst);
        }
    }
}

/// Compiles an LLVM IR file into an LLVM bitcode file.
fn llvm_as_build<P>(src: P, dst: P)
where
    P: AsRef<Path>,
{
    let status = Command::new("llvm-as")
        .arg("-o")
        .arg(dst.as_ref())
        .arg(src.as_ref())
        .status()
        .unwrap_or_else(|err| panic!("could not run llvm-as: {err}"));
    assert_eq!(status.code(), Some(0), "llvm-as failed");
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
    let current_dir = env::current_dir().expect("could not determine current directory");
    let directory = current_dir.join("target/sysroot");
    let () = rustc_build_sysroot::SysrootBuilder::new(&directory, target)
        .build_mode(rustc_build_sysroot::BuildMode::Build)
        .sysroot_config(rustc_build_sysroot::SysrootConfig::NoStd)
        // to be able to thoroughly test DI we need to build sysroot with debuginfo
        // this is necessary to compile rust core with DI
        .rustflag("-Cdebuginfo=2")
        .build_from_source(&rustc_src)
        .expect("failed to build sysroot");

    build_bitcode(current_dir.join("tests/ir"));

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
