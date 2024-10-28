use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, remove_file, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn find_binary(binary_re_str: &str) -> PathBuf {
    let binary_re = regex::Regex::new(binary_re_str).unwrap();
    let mut binary = which::which_re(binary_re).expect(binary_re_str);
    binary
        .next()
        .unwrap_or_else(|| panic!("could not find {binary_re_str}"))
}

fn run_mode<F: Fn(&mut compiletest_rs::Config)>(
    target: &str,
    mode: &str,
    sysroot: Option<&Path>,
    cfg: Option<F>,
) {
    let bpf_linker_exe = env!("CARGO_BIN_EXE_bpf-linker");
    let bpf_linker = match env::var_os("BPF_LINKER_QEMU") {
        // If we are running tests in a user-space emulator, we need to run
        // bpf-linker in it as well.
        Some(qemu) => {
            // Create a wrapper script which runs bpf-linker with qemu.
            //
            // Unfortunately, passing
            // `-C linker='qemu-aarch64 ./target/aarch64-uknown-linux-musl/debug/bpf-linker'`
            // doesn't work, `compiletest_rs::Config` is going to split this
            // argument because of a whitespace.
            let script_path = Path::new("/tmp/qemu_bpf_linker_wrapper.sh");
            if script_path.exists() {
                remove_file(script_path).expect("Could not remove the QEMU wrapper file");
            }
            // It's an environment variable set by us. Safe to assume it's UTF-8.
            let qemu = qemu.to_string_lossy();
            let script_content = format!(
                r#"#!/bin/bash
{qemu} "{bpf_linker_exe}" "$@"
"#
            );
            let mut file =
                File::create(script_path).expect("Failed to create the QEMU wrapper file");
            file.write_all(script_content.as_bytes())
                .expect("Failed to write to the QEMU wrapper file");
            let metadata = file
                .metadata()
                .expect("Failed to retrieve the metadata of the QEMU wrapper file");
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            file.set_permissions(permissions)
                .expect("Failed to set permissions of the QEMU wrapper file");

            script_path.to_str().unwrap()
        }
        None => env!("CARGO_BIN_EXE_bpf-linker"),
    };
    let mut target_rustcflags = format!("-C linker={}", bpf_linker);
    if let Some(sysroot) = sysroot {
        let sysroot = sysroot.to_str().unwrap();
        target_rustcflags += &format!(" --sysroot {sysroot}");
    }

    let llvm_filecheck = Some(find_binary(r"^FileCheck(-\d+)?$"));

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
fn build_bitcode<P>(src_dir: P, dst_dir: P)
where
    P: AsRef<Path>,
{
    fs::create_dir_all(dst_dir.as_ref()).expect("failed to create a build directory for bitcode");
    for entry in fs::read_dir(src_dir.as_ref()).expect("failed to read the directory") {
        let entry = entry.expect("failed to read the entry");
        let path = entry.path();

        if path.is_file() && path.extension() == Some(OsStr::new("c")) {
            let bc_dst = dst_dir
                .as_ref()
                .join(path.with_extension("bc").file_name().unwrap());
            clang_build(path, bc_dst);
        }
    }
}

/// Compiles C code into an LLVM bitcode file.
fn clang_build<P>(src: P, dst: P)
where
    P: AsRef<Path>,
{
    let clang = find_binary(r"^clang(-\d+)?$");
    let output = Command::new(clang)
        .arg("-target")
        .arg("bpf")
        .arg("-g")
        .arg("-c")
        .arg("-emit-llvm")
        .arg("-o")
        .arg(dst.as_ref())
        .arg(src.as_ref())
        .output()
        .expect("failed to execute clang");

    if !output.status.success() {
        panic!(
            "clang failed with code {:?}\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn btf_dump(src: &Path, dst: &Path) {
    let dst = std::fs::File::create(dst)
        .unwrap_or_else(|err| panic!("could not open btf dump file '{}': {err}", dst.display()));
    let mut btf = Command::new("btf");
    btf.arg("dump").arg(src).stdout(dst);
    let status = btf
        .status()
        .unwrap_or_else(|err| panic!("could not run {btf:?}: {err}",));
    assert_eq!(status.code(), Some(0), "{btf:?} failed");
}

#[test]
fn compile_test() {
    let target = "bpfel-unknown-none";
    let rustc =
        std::process::Command::new(env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")));
    let rustc_src = rustc_build_sysroot::rustc_sysroot_src(rustc)
        .expect("could not determine sysroot source directory");
    let root_dir = env::var_os("CARGO_MANIFEST_DIR")
        .expect("could not determine the root directory of the project");
    let root_dir = Path::new(&root_dir);
    let directory = root_dir.join("target/sysroot");
    match rustc_build_sysroot::SysrootBuilder::new(&directory, target)
        .build_mode(rustc_build_sysroot::BuildMode::Build)
        .sysroot_config(rustc_build_sysroot::SysrootConfig::NoStd)
        // to be able to thoroughly test DI we need to build sysroot with debuginfo
        // this is necessary to compile rust core with DI
        .rustflag("-Cdebuginfo=2")
        .build_from_source(&rustc_src)
        .expect("failed to build sysroot")
    {
        rustc_build_sysroot::SysrootStatus::AlreadyCached => {}
        rustc_build_sysroot::SysrootStatus::SysrootBuilt => {}
    }

    build_bitcode(root_dir.join("tests/c"), root_dir.join("target/bitcode"));

    run_mode(
        target,
        "assembly",
        Some(&directory),
        None::<fn(&mut compiletest_rs::Config)>,
    );

    #[cfg(not(target_os = "macos"))]
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
