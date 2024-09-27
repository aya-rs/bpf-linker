use std::{
    env,
    ffi::{OsStr, OsString},
    fmt::Display,
    fs::read_dir,
    os::unix::ffi::OsStringExt,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::Context;
use clap::{ArgAction, Parser, ValueEnum};
use target_lexicon::{Environment, Triple};
use thiserror::Error;

use crate::{
    containers::ContainerEngine,
    target::{SupportedTriple, TripleExt},
};

#[derive(Debug, Error)]
pub enum CargoError {
    #[error("cargo build failed")]
    CargoBuild,
    #[error("could not find a git repository")]
    RepositoryNotFound,
}

#[derive(Clone, ValueEnum)]
pub enum LinkType {
    Dynamic,
    Static,
}

impl Display for LinkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dynamic => write!(f, "dylib"),
            Self::Static => write!(f, "static"),
        }
    }
}

impl LinkType {
    fn default(triple: &Triple) -> Self {
        // Link system libraries dynamically only on GNU/Linux. The reason
        // being - Debian doesn't ship static zlib and zstd.
        // Static linking works fine on other systems (BSDs, macOS,
        // musl/Linux).
        if triple.environment == Environment::Gnu {
            Self::Dynamic
        } else {
            Self::Static
        }
    }
}

#[derive(Parser)]
pub struct CargoArgs {
    /// Container engine (if not provided, is going to be autodetected).
    #[arg(long)]
    container_engine: Option<ContainerEngine>,

    /// Container image repository.
    #[arg(long, default_value = "ghcr.io/exein-io/cross-llvm")]
    container_repository: String,

    /// Tag of the container image.
    #[arg(long, default_value = "latest")]
    container_tag: String,

    /// Space or comma separated list of features to activate.
    #[arg(short, long)]
    features: Vec<OsString>,

    /// Activate all available features.
    #[arg(long)]
    all_features: bool,

    #[arg(long)]
    link_type: Option<LinkType>,

    /// Prefix in which LLVM libraries are going to be installed after build.
    #[arg(long)]
    llvm_install_dir: Option<OsString>,

    /// Build artifacts in release mode, with optimizations.
    #[arg(long)]
    release: bool,

    /// Target triple (optional).
    #[arg(short, long)]
    target: Option<SupportedTriple>,

    /// Use verbose output (-vv very verbose/build.rs output).
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,
}

pub fn run_cargo(args: CargoArgs, command: &OsStr) -> anyhow::Result<()> {
    let CargoArgs {
        container_engine,
        container_repository,
        container_tag,
        mut features,
        all_features,
        link_type,
        llvm_install_dir,
        release,
        target,
        verbose,
    } = args;

    // Disable the LLVM linking capabilities from llvm-sys, they don't support
    // cross-compilation. Instead, we are building our own linking flags, based
    // on the specified `llvm_install_dir`.
    features.push(OsString::from("llvm-sys/no-llvm-linking"));

    let triple: Triple = match target {
        Some(target) => target.into(),
        None => target_lexicon::HOST,
    };

    let link_type = link_type.unwrap_or(LinkType::default(&triple));
    let sysroot = triple.sysroot();

    let llvm_install_dir = match llvm_install_dir {
        Some(llvm_install_dir) => llvm_install_dir,
        None => Path::new("/tmp")
            .join(format!("aya-llvm-{triple}"))
            .into_os_string(),
    };

    let workdir = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    let workdir = match workdir {
        Ok(output) if output.status.success() => {
            OsString::from_vec(
                // Remove the trailing `\n` character.
                output.stdout[..output.stdout.len() - 1].to_vec(),
            )
        }
        Ok(_) => {
            return Err(CargoError::RepositoryNotFound.into());
        }
        Err(_) => {
            return Err(CargoError::RepositoryNotFound.into());
        }
    };

    let mut rustflags = OsString::from("RUSTFLAGS=-C linker=clang");
    if triple.is_cross() {
        rustflags.push(" -C link-arg=--target=");
        rustflags.push(triple.to_string());
    }
    if let Some(sysroot) = sysroot {
        rustflags.push(" -C link-arg=--sysroot=");
        rustflags.push(sysroot.clone());
    }
    for libdir in triple.libdirs() {
        rustflags.push(" -L native=");
        rustflags.push(libdir);
    }
    rustflags.push(" -L native=");
    rustflags.push(Path::new(&llvm_install_dir).join("lib"));
    rustflags.push(format!(" -l {}=rt", link_type));
    rustflags.push(format!(" -l {}=dl", link_type));
    rustflags.push(format!(" -l {}=m", link_type));
    rustflags.push(format!(" -l {}=z", link_type));
    rustflags.push(format!(" -l {}=zstd", link_type));
    if triple.environment == Environment::Gnu {
        rustflags.push(format!(" -l {}=stdc++", link_type));
    } else {
        rustflags.push(format!(" -l {}=c++_static", link_type));
        rustflags.push(format!(" -l {}=c++abi", link_type));
    }

    for entry in read_dir(Path::new(&llvm_install_dir).join("lib"))
        .context("LLVM build directory not found")?
    {
        let entry = entry.context("failed to retrieve the file in the LLVM build directory")?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("a") {
            rustflags.push(" -l static=");
            rustflags.push(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .strip_prefix("lib")
                    .unwrap()
                    .strip_suffix(".a")
                    .unwrap(),
            );
        }
    }

    let cargo_args = [
        OsString::from("--config"),
        OsString::from("target.aarch64-unknown-linux-gnu.runner = 'qemu-aarch64 -L /usr/aarch64-linux-gnu/lib -L /usr/lib/aarch64-linux-gnu'"),
        OsString::from("--config"),
        OsString::from("target.aarch64-unknown-linux-musl.runner = 'qemu-aarch64'"),
        OsString::from("--config"),
        OsString::from("target.riscv64gc-unknown-linux-gnu.runner = 'qemu-riscv64 -L /usr/riscv64-linux-gnu/lib -L /usr/lib/riscv64-linux-gnu'"),
        OsString::from("--config"),
        OsString::from("target.riscv64gc-unknown-linux-musl.runner = 'qemu-riscv64'"),
        OsString::from("--config"),
        OsString::from("target.x86_64-unknown-linux-gnu.runner = 'qemu-x86_64 -L /usr/x86_64-linux-gnu/lib -L /usr/lib/x86_64-linux-gnu'"),
        OsString::from("--config"),
        OsString::from("target.x86_64-unknown-linux-musl.runner = 'qemu-x86_64'"),
        command.to_owned(),
        OsString::from("--target"),
        OsString::from(triple.to_string()),
        OsString::from("--no-default-features"),
    ];

    match triple.container_image(&container_repository, &container_tag) {
        Some((container_image, _)) => {
            println!("Using container image {container_image}");

            let container_engine = container_engine.unwrap_or(ContainerEngine::autodetect()?);

            let mut llvm_prefix = OsString::from("LLVM_SYS_191_PREFIX=");
            llvm_prefix.push(&llvm_install_dir);

            let rustup_toolchain = env::var("RUSTUP_TOOLCHAIN").unwrap();
            let rustup_toolchain = rustup_toolchain.split('-').next().unwrap();
            let mut rustup_toolchain_triple = target_lexicon::HOST;
            rustup_toolchain_triple.environment = triple.environment;
            let rustup_toolchain = format!("{rustup_toolchain}-{}", rustup_toolchain_triple);
            let mut rustup_toolchain_arg = OsString::from("RUSTUP_TOOLCHAIN=");
            rustup_toolchain_arg.push(rustup_toolchain);

            let mut workdir_arg = workdir;
            workdir_arg.push(":/usr/local/src/bpf-linker");

            let mut llvm_arg = llvm_install_dir.clone();
            llvm_arg.push(":");
            llvm_arg.push(&llvm_install_dir);

            let mut cmd = Command::new(container_engine.to_string());
            cmd.args([
                OsStr::new("run"),
                OsStr::new("--rm"),
                OsStr::new("-e"),
                &llvm_prefix,
                OsStr::new("-e"),
                &rustflags,
                OsStr::new("-e"),
                &rustup_toolchain_arg,
            ]);
            if triple.is_cross() {
                let mut qemu = OsString::from("BPF_LINKER_QEMU=qemu-");
                qemu.push(triple.architecture.to_string());
                cmd.args([OsStr::new("-e"), &qemu]);
            }
            cmd.args([
                OsStr::new("-t"),
                OsStr::new("-w"),
                OsStr::new("/usr/local/src/bpf-linker"),
                OsStr::new("-v"),
                &workdir_arg,
                OsStr::new("-v"),
                &llvm_arg,
                OsStr::new(&container_image),
                OsStr::new("cargo"),
            ]);
            cmd.args(cargo_args);
            match verbose {
                0 => {}
                1 => {
                    cmd.arg("-v");
                }
                _ => {
                    cmd.arg("-vv");
                }
            }
            if release {
                cmd.arg("--release");
            }
            if !features.is_empty() {
                cmd.arg("--features");
                cmd.args(features);
            }
            if all_features {
                cmd.arg("--all-features");
            }
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(CargoError::CargoBuild.into());
            }
        }
        None => {
            let mut cmd = Command::new("cargo");
            cmd.args(cargo_args);
        }
    }

    Ok(())
}
