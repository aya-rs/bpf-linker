use std::{ffi::OsString, fs, path::PathBuf, process::Command};

use anyhow::{Context as _, Result};
use rustc_build_sysroot::{BuildMode, SysrootConfig, SysrootStatus};
use walkdir::WalkDir;

#[derive(Clone, clap::ValueEnum)]
enum Target {
    BpfebUnknownNone,
    BpfelUnknownNone,
}

impl Target {
    fn as_str(&self) -> &'static str {
        match self {
            Self::BpfebUnknownNone => "bpfeb-unknown-none",
            Self::BpfelUnknownNone => "bpfel-unknown-none",
        }
    }
}

#[derive(clap::Parser)]
struct BuildStd {
    #[arg(long)]
    rustc_src: PathBuf,

    #[arg(long)]
    sysroot_dir: PathBuf,

    #[arg(long, value_enum)]
    target: Target,
}

#[derive(clap::Parser)]
struct BuildLlvm {
    /// Source directory.
    #[arg(long)]
    src_dir: PathBuf,
    /// Build directory.
    #[arg(long)]
    build_dir: PathBuf,
    /// Directory in which the built LLVM artifacts are installed.
    #[arg(long)]
    install_prefix: PathBuf,
    /// Library install directory, must match the syntax of
    /// `CMAKE_INSTALL_LIBDIR`.
    #[arg(long, default_value = "lib")]
    install_libdir: PathBuf,
    /// C compiler.
    #[arg(long, default_value = "clang")]
    c_compiler: String,
    /// C++ compiler.
    #[arg(long, default_value = "clang++")]
    cxx_compiler: String,
    /// Target architecture to build LLVM for, must match the syntax of
    /// `CMAKE_SYSTEM_PROCESSOR` option, e.g. `aarch64`, `riscv64`.
    #[arg(long, requires = "cmake_system_name")]
    cmake_system_processor: Option<String>,
    /// Target system to build LLVM for, must match the syntax of
    /// `CMAKE_SYSTEM_NAME` option, e.g. `Linux`.
    #[arg(long)]
    cmake_system_name: Option<String>,
    /// Sysroot that contains necessary libraries for the given target, e.g.
    /// `/usr/aarch64-unknown-linux-musl`.
    #[arg(long)]
    sysroot: Option<PathBuf>,
}

#[derive(clap::Subcommand)]
enum XtaskSubcommand {
    /// Builds the Rust standard library for the given target in the current
    /// toolchain's sysroot.
    BuildStd(BuildStd),
    /// Manages and builds LLVM.
    BuildLlvm(BuildLlvm),
}

/// Additional build commands for bpf-linker.
#[derive(clap::Parser)]
struct CommandLine {
    #[command(subcommand)]
    subcommand: XtaskSubcommand,
}

fn build_std(options: BuildStd) -> Result<()> {
    let BuildStd {
        rustc_src,
        sysroot_dir,
        target,
    } = options;

    let target = target.as_str();
    let sysroot_status =
        match rustc_build_sysroot::SysrootBuilder::new(sysroot_dir.as_path(), target)
            // Do a full sysroot build.
            .build_mode(BuildMode::Build)
            // We want only `core`, not `std`.
            .sysroot_config(SysrootConfig::NoStd)
            // Include debug symbols in order to generate correct BTF types for
            // the core types as well.
            .rustflag("-Cdebuginfo=2")
            .build_from_source(&rustc_src)?
        {
            SysrootStatus::AlreadyCached => "was already built",
            SysrootStatus::SysrootBuilt => "built successfully",
        };
    println!(
        "Standard library for target {target} {sysroot_status}: {}",
        sysroot_dir.display()
    );
    Ok(())
}

fn build_llvm(options: BuildLlvm) -> Result<()> {
    let BuildLlvm {
        src_dir,
        build_dir,
        install_prefix,
        install_libdir,
        c_compiler,
        cxx_compiler,
        cmake_system_processor,
        cmake_system_name,
        sysroot,
    } = options;

    let mut install_arg = OsString::from("-DCMAKE_INSTALL_PREFIX=");
    install_arg.push(install_prefix.as_os_str());
    let mut install_libdir_arg = OsString::from("-DCMAKE_INSTALL_LIBDIR=");
    install_libdir_arg.push(install_libdir.as_os_str());
    let mut cmake_configure = Command::new("cmake");
    let _: &mut _ = cmake_configure
        .arg("-S")
        .arg(src_dir.join("llvm"))
        .arg("-B")
        .arg(&build_dir)
        .args([
            "-G",
            "Ninja",
            "-DCMAKE_BUILD_TYPE=RelWithDebInfo",
            "-DLLVM_BUILD_LLVM_DYLIB=ON",
            "-DLLVM_ENABLE_ASSERTIONS=ON",
            "-DLLVM_ENABLE_PROJECTS=",
            "-DLLVM_ENABLE_RUNTIMES=",
            "-DLLVM_INSTALL_UTILS=ON",
            "-DLLVM_LINK_LLVM_DYLIB=ON",
            "-DLLVM_TARGETS_TO_BUILD=BPF",
            "-DLLVM_USE_LINKER=lld",
            // We build a minimal LLVM (only the BPF target). If its version matches the
            // system LLVM and our libLLVM ends up on LD_LIBRARY_PATH, binaries like clang
            // may accidentally load *our* libLLVM. That can fail at runtime because this
            // build omits components/targets the system binaries expect (missing symbols).
            //
            // Give our shared library a unique version suffix so it cannot be confused
            // with the system libLLVM. Our build script (build.rs) determines the exact
            // filename of the produced library, as long as our llvm-config appears first
            // in PATH. We can then safely add our install directory to LD_LIBRARY_PATH
            // without worrying about conflicts.
            "-DLLVM_VERSION_SUFFIX=-bpf-linker-1",
        ])
        .args([
            format!("-DCMAKE_C_COMPILER={c_compiler}"),
            format!("-DCMAKE_CXX_COMPILER={cxx_compiler}"),
        ])
        .args([install_arg, install_libdir_arg]);
    if let Some(cmake_system_processor) = cmake_system_processor {
        let _: &mut _ =
            cmake_configure.arg(format!("-DCMAKE_SYSTEM_PROCESSOR={cmake_system_processor}"));
    }
    if let Some(cmake_system_name) = cmake_system_name {
        let _: &mut _ = cmake_configure.arg(format!("-DCMAKE_SYSTEM_NAME={cmake_system_name}"));
    }
    if let Some(sysroot) = sysroot {
        // CMake expects the sysroot path to be absolute.
        let sysroot = fs::canonicalize(&sysroot).with_context(|| {
            format!(
                "failed to canonicalize the sysroot path {}",
                sysroot.display()
            )
        })?;
        let mut sysroot_arg = OsString::from("-DCMAKE_SYSROOT=");
        sysroot_arg.push(sysroot.as_os_str());
        let _: &mut _ = cmake_configure.arg(sysroot_arg);
        let _: &mut _ = cmake_configure.args([
            // Tell CMake to:
            //
            // - Search for libraries and include files only in the sysroot.
            // - Never try to execute any binaries (e.g. ar, llvm-min-tblgen) from
            //   the sysroot and instead try to find them on the host system.
            "-DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER",
            "-DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY",
            "-DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY",
            // CMake's "try compile" check for -fPIC does not work for cross
            // builds that require a sysroot, because the sysroot arguments are
            // not passed to the compile flags for the test.
            "-DC_SUPPORTS_FPIC=ON",
            "-DCXX_SUPPORTS_FPIC=ON",
        ]);
    }
    println!("Configuring LLVM with command {cmake_configure:?}");
    let status = cmake_configure.status().with_context(|| {
        format!("failed to configure LLVM build with command {cmake_configure:?}")
    })?;
    if !status.success() {
        anyhow::bail!("failed to configure LLVM build with command {cmake_configure:?}: {status}");
    }

    let mut cmake_build = Command::new("cmake");
    let cmake_build = cmake_build
        .arg("--build")
        .arg(build_dir)
        .args(["--target", "install"])
        // Create symlinks rather than copies to conserve disk space,
        // especially on GitHub-hosted runners.
        //
        // Since the LLVM build creates a bunch of symlinks (and this setting
        // does not turn those into symlinks-to-symlinks), use absolute
        // symlinks so we can distinguish the two cases.
        .env("CMAKE_INSTALL_MODE", "ABS_SYMLINK");
    println!("Building LLVM with command {cmake_build:?}");
    let status = cmake_build
        .status()
        .with_context(|| format!("failed to build LLVM with command {cmake_configure:?}"))?;
    if !status.success() {
        anyhow::bail!("failed to configure LLVM build with command {cmake_configure:?}: {status}");
    }

    // Move targets over the symlinks that point to them.
    //
    // This whole dance would be simpler if CMake supported
    // `CMAKE_INSTALL_MODE=MOVE`.
    for entry in WalkDir::new(&install_prefix).follow_links(false) {
        let entry = entry.with_context(|| {
            format!(
                "failed to read filesystem entry while traversing install prefix {}",
                install_prefix.display()
            )
        })?;
        if !entry.file_type().is_symlink() {
            continue;
        }

        let link_path = entry.path();
        let target = fs::read_link(link_path)
            .with_context(|| format!("failed to read the link {}", link_path.display()))?;
        if target.is_absolute() {
            fs::rename(&target, link_path).with_context(|| {
                format!(
                    "failed to move the target file {} to the location of the symlink {}",
                    target.display(),
                    link_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let CommandLine { subcommand } = clap::Parser::parse();
    match subcommand {
        XtaskSubcommand::BuildStd(options) => build_std(options),
        XtaskSubcommand::BuildLlvm(options) => build_llvm(options),
    }
}
