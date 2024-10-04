use std::{
    env,
    ffi::{OsStr, OsString},
    fs::read_dir,
    os::unix::ffi::OsStringExt,
    path::Path,
    process::{Command, Stdio},
};

use clap::Parser;
use target_lexicon::{Environment, Triple};
use thiserror::Error;

use crate::{
    containers::{ContainerEngine, ContainerError},
    target::{SupportedTriple, TripleExt},
};

#[derive(Debug, Error)]
pub enum CargoError {
    #[error(transparent)]
    Container(ContainerError),
    #[error("cargo build failed")]
    CargoBuild,
    #[error("could not find a git repository")]
    RepositoryNotFound,
}

#[derive(Parser)]
pub struct CargoArgs {
    /// Container engine (if not provided, is going to be autodetected).
    #[arg(long)]
    container_engine: Option<ContainerEngine>,

    /// Space or comma separated list of features to activate.
    #[arg(short, long)]
    features: Vec<OsString>,

    /// Activate all available features.
    #[arg(long)]
    all_features: bool,

    /// Do not activate the `default` feature.
    #[arg(long)]
    no_default_features: bool,

    /// Prefix in which LLVM libraries are going to be installed after build.
    #[arg(long)]
    llvm_install_dir: Option<OsString>,

    /// Target triple (optional).
    #[arg(short, long)]
    target: Option<SupportedTriple>,

    /// Build artifacts in release mode, with optimizations.
    #[arg(long)]
    release: bool,
}

pub fn run_cargo(args: CargoArgs, command: &OsStr) -> anyhow::Result<()> {
    let CargoArgs {
        container_engine,
        mut features,
        all_features,
        no_default_features,
        llvm_install_dir,
        target,
        release,
    } = args;

    // Disable the LLVM linking capabilities from llvm-sys, they don't support
    // cross-compilation. Instead, we are building our own linking flags, based
    // on the specified `llvm_install_dir`.
    features.push(OsString::from("llvm-sys/no-llvm-linking"));

    let triple: Triple = match target {
        Some(target) => target.into(),
        None => target_lexicon::HOST,
    };

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

    let mut rustflags = OsString::from("RUSTFLAGS=-L native=");
    rustflags.push(Path::new(&llvm_install_dir).join("lib"));
    rustflags.push(" -L native=/lib -L native=/usr/lib -l static=c++abi -l static=rt -l static=dl -l static=m -l static=z -l static=zstd");
    if triple.environment == Environment::Musl {
        rustflags.push(" -l static=c++_static");
    } else {
        rustflags.push(" -lc++");
    }
    for entry in read_dir(Path::new(&llvm_install_dir).join("lib"))? {
        let entry = entry?;
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

    match triple.container_image() {
        Some((container_image, _)) => {
            println!("Using container image {container_image}");

            let container_engine =
                container_engine.unwrap_or(ContainerEngine::autodetect().ok_or(
                    CargoError::Container(ContainerError::ContainerEngineNotFound),
                )?);

            let mut llvm_prefix = OsString::from("LLVM_SYS_191_PREFIX=");
            llvm_prefix.push(&llvm_install_dir);

            let mut rustup_toolchain_arg = OsString::from("RUSTUP_TOOLCHAIN=");
            rustup_toolchain_arg.push(env::var("RUSTUP_TOOLCHAIN").unwrap());

            let mut workdir_arg = workdir;
            workdir_arg.push(":/usr/local/src/bpf-linker");

            let mut llvm_arg = llvm_install_dir.clone();
            llvm_arg.push(":");
            llvm_arg.push(&llvm_install_dir);

            let mut cmd = Command::new(container_engine.to_string());
            cmd.args([
                OsStr::new("run"),
                OsStr::new("-e"),
                &llvm_prefix,
                OsStr::new("-e"),
                &rustflags,
                OsStr::new("-e"),
                &rustup_toolchain_arg,
                OsStr::new("-it"),
                OsStr::new("-w"),
                OsStr::new("/usr/local/src/bpf-linker"),
                OsStr::new("-v"),
                &workdir_arg,
                OsStr::new("-v"),
                &llvm_arg,
                OsStr::new(&container_image),
                OsStr::new("cargo"),
                command,
            ]);
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
            if no_default_features {
                cmd.arg("--no-default-features");
            }
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(CargoError::CargoBuild.into());
            }
        }
        None => {}
    }

    Ok(())
}
