use std::{
    env, ffi::OsString, fs, os::unix::ffi::OsStringExt as _, path::PathBuf, process::Command,
};

use anyhow::{Context as _, Result, bail};
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
    #[arg(long, value_enum, default_value_t = Target::BpfelUnknownNone)]
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

fn sysroot_dir() -> Result<PathBuf> {
    let mut rustc = Command::new(env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")));
    let output = rustc
        .args(["--print", "sysroot"])
        .output()
        .with_context(|| format!("failed to derermine rustc sysroot: {rustc:?}"))?;
    if !output.status.success() {
        bail!("failed to determine rustc sysroot: {output:?}");
    }
    Ok(PathBuf::from(OsString::from_vec(
        output.stdout.trim_ascii().to_vec(),
    )))
}

fn build_std(options: BuildStd) -> Result<()> {
    let BuildStd { target } = options;

    let sysroot_dir = sysroot_dir()?;
    let source_dir = sysroot_dir
        .join("lib")
        .join("rustlib")
        .join("src")
        .join("rust")
        .join("library");
    let target = target.as_str();
    let rustlibdir = sysroot_dir.join("lib").join("rustlib").join(target);
    let sysroot_status =
        match rustc_build_sysroot::SysrootBuilder::new(sysroot_dir.as_path(), target)
            // Do a full sysroot build.
            .build_mode(BuildMode::Build)
            // We want only `core`, not `std`.
            .sysroot_config(SysrootConfig::NoStd)
            // Include debug symbols in order to generate correct BTF types for
            // the core types as well.
            .rustflag("-Cdebuginfo=2")
            .build_from_source(&source_dir)?
        {
            SysrootStatus::AlreadyCached => "was already built",
            SysrootStatus::SysrootBuilt => "built successfully",
        };
    println!(
        "Standard library for target {target} {sysroot_status}: {}",
        rustlibdir.display()
    );
    Ok(())
}

fn build_llvm(options: BuildLlvm) -> Result<()> {
    let BuildLlvm {
        src_dir,
        build_dir,
        install_prefix,
    } = options;

    // let build_dir = src_dir.join("build");

    let mut install_arg = OsString::from("-DCMAKE_INSTALL_PREFIX=");
    install_arg.push(install_prefix.as_os_str());
    let mut cmake_configure = Command::new("cmake");
    let cmake_configure = cmake_configure
        .arg("-S")
        .arg(src_dir.join("llvm"))
        .arg("-B")
        .arg(&build_dir)
        .args([
            "-G",
            "Ninja",
            "-DCMAKE_BUILD_TYPE=RelWithDebInfo",
            "-DCMAKE_C_COMPILER=clang",
            "-DCMAKE_CXX_COMPILER=clang++",
            "-DLLVM_BUILD_LLVM_DYLIB=ON",
            "-DLLVM_ENABLE_ASSERTIONS=ON",
            "-DLLVM_ENABLE_PROJECTS=",
            "-DLLVM_ENABLE_RUNTIMES=",
            "-DLLVM_INSTALL_UTILS=ON",
            "-DLLVM_LINK_LLVM_DYLIB=ON",
            "-DLLVM_TARGETS_TO_BUILD=BPF",
            "-DLLVM_USE_LINKER=lld",
        ])
        .arg(install_arg);
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
    for entry in WalkDir::new(&install_prefix)
        .follow_links(false)
        .into_iter()
    {
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
            // Move the file the link points to into the location of the symlink
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
