#![expect(unused_crate_dependencies, reason = "used in lib/bin")]

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::{OsStrExt as _, OsStringExt as _},
    path::{Path, PathBuf},
    process::Command,
};

fn rustc_cmd() -> Command {
    Command::new(env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")))
}

fn find_binary(binary_re_str: &str) -> PathBuf {
    let binary_re = regex::Regex::new(binary_re_str).unwrap();
    let mut binary = which::which_re(binary_re).expect(binary_re_str);
    binary
        .next()
        .unwrap_or_else(|| panic!("could not find {binary_re_str}"))
}

fn run_mode<F>(target: &str, mode: &str, sysroot: &Path, cfg: Option<F>)
where
    F: Fn(&mut compiletest_rs::Config),
{
    let target_rustcflags = format!(
        "-C linker={} --sysroot {}",
        env!("CARGO_BIN_EXE_bpf-linker"),
        sysroot.to_str().unwrap()
    );

    let llvm_filecheck = Some(find_binary(r"^FileCheck(-\d+)?$"));

    let mode = mode.parse().expect("Invalid mode");
    let mut config = compiletest_rs::Config {
        target: target.to_owned(),
        target_rustcflags: Some(target_rustcflags),
        llvm_filecheck,
        mode,
        src_base: PathBuf::from(format!("tests/{mode}")),
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

fn is_nightly() -> bool {
    let output = rustc_cmd()
        .arg("--version")
        .output()
        .expect("failed to determine rustc version");
    if !output.status.success() {
        panic!("failed to determine rustc version: {output:?}");
    }
    const NIGHTLY: &[u8] = b"nightly";
    output.stdout.windows(NIGHTLY.len()).any(|b| NIGHTLY.eq(b))
}

/// Returns the active toolchain sysroot if it supports the requested BPF
/// target.
///
/// This currently works only with custom Rust toolchains built with BPF target
/// support.
fn toolchain_bpf_sysroot(target: &str) -> Option<PathBuf> {
    let output = rustc_cmd()
        .args(["--print", "sysroot"])
        .output()
        .expect("failed to determine rustc sysroot");
    if !output.status.success() {
        panic!("failed to determine rustc sysroot: {output:?}");
    }

    let mut sysroot = output.stdout;
    while matches!(sysroot.last(), Some(b'\n' | b'\r')) {
        let _newline = sysroot.pop();
    }
    let sysroot = PathBuf::from(OsString::from_vec(sysroot));
    let target_libdir = sysroot.join("lib/rustlib").join(target).join("lib");
    if !target_libdir.is_dir() {
        eprintln!(
            "toolchain sysroot {} does not contain target {target} sysroot {}; falling back to building a sysroot",
            sysroot.display(),
            target_libdir.display(),
        );
        return None;
    }

    let has_core = fs::read_dir(&target_libdir)
        .unwrap_or_else(|err| {
            panic!(
                "failed to read target libdir {}: {err}",
                target_libdir.display()
            )
        })
        .any(|entry| {
            let entry = entry.unwrap_or_else(|err| {
                panic!(
                    "failed to read entry in target libdir {}: {err}",
                    target_libdir.display()
                )
            });
            let name = entry.file_name();
            let name = name.as_os_str().as_bytes();
            name.starts_with(b"libcore-") && name.ends_with(b".rlib")
        });
    if !has_core {
        eprintln!(
            "toolchain sysroot {} has target {} but does not contain libcore; falling back to building a sysroot",
            sysroot.display(),
            target
        );
    }

    has_core.then_some(sysroot)
}

fn btf_dump(src: &Path, dst: &Path) {
    let dst = fs::File::create(dst)
        .unwrap_or_else(|err| panic!("could not open btf dump file '{}': {err}", dst.display()));
    let mut btf = Command::new("btf");
    let status = btf
        .arg("dump")
        .arg(src)
        .stdout(dst)
        .status()
        .unwrap_or_else(|err| panic!("could not run {btf:?}: {err}"));
    assert_eq!(status.code(), Some(0), "{btf:?} failed");
}

#[test]
fn compile_test() {
    let target = "bpfel-unknown-none";
    let root_dir = env::var_os("CARGO_MANIFEST_DIR")
        .expect("could not determine the root directory of the project");
    let root_dir = Path::new(&root_dir);
    let bpf_sysroot = if let Some(bpf_sysroot) = env::var_os("BPFEL_SYSROOT_DIR") {
        PathBuf::from(bpf_sysroot)
    } else if let Some(bpf_sysroot) = toolchain_bpf_sysroot(target) {
        bpf_sysroot
    } else {
        let rustc_src = rustc_build_sysroot::rustc_sysroot_src(rustc_cmd())
            .expect("could not determine sysroot source directory");
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
        directory
    };

    build_bitcode(root_dir.join("tests/c"), root_dir.join("target/bitcode"));

    run_mode(
        target,
        "assembly",
        &bpf_sysroot,
        None::<fn(&mut compiletest_rs::Config)>,
    );
    run_mode(
        target,
        "assembly",
        &bpf_sysroot,
        Some(|cfg: &mut compiletest_rs::Config| {
            cfg.src_base = PathBuf::from("tests/btf");
            cfg.llvm_filecheck_preprocess = Some(btf_dump);
        }),
    );
    // The `tests/nightly` directory contains tests which require unstable compiler
    // features through the `-Z` argument in `compile-flags`.
    if is_nightly() {
        run_mode(
            target,
            "assembly",
            &bpf_sysroot,
            Some(|cfg: &mut compiletest_rs::Config| {
                cfg.src_base = PathBuf::from("tests/nightly/btf");
                cfg.llvm_filecheck_preprocess = Some(btf_dump);
            }),
        );
    }
}

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
