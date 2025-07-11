use std::{env, ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf, process::Command};

use anyhow::{Context as _, Result, bail};
use rustc_build_sysroot::{BuildMode, SysrootConfig, SysrootStatus};

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

#[derive(clap::Subcommand)]
enum XtaskSubcommand {
    /// Builds the Rust standard library for the given target in the current
    /// toolchain's sysroot.
    BuildStd(BuildStd),
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

fn main() -> Result<()> {
    let CommandLine { subcommand } = clap::Parser::parse();
    match subcommand {
        XtaskSubcommand::BuildStd(options) => build_std(options),
    }
}
