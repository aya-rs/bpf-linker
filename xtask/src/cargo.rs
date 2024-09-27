use std::{ffi::OsString, process::Command};

use clap::{ArgAction, Parser};
use target_lexicon::Triple;

use crate::{
    containers::{Container, ContainerEngine, PullPolicy},
    git::top_directory,
    target::{SupportedTriple, TripleExt},
};

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

    /// Prefix in which LLVM libraries are going to be installed after build.
    #[arg(long)]
    llvm_install_dir: Option<OsString>,

    /// Pull image policy.
    #[arg(long, default_value_t = PullPolicy::default())]
    pull: PullPolicy,

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

pub fn run_cargo(
    args: CargoArgs,
    command: OsString,
    extra_args: &[OsString],
) -> anyhow::Result<()> {
    let CargoArgs {
        container_engine,
        container_repository,
        container_tag,
        mut features,
        all_features,
        llvm_install_dir,
        pull,
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

    let workdir = top_directory().unwrap();

    let mut rustflags = OsString::from("RUSTFLAGS=-C linker=clang -C link-arg=-fuse-ld=lld");
    if triple.is_cross() {
        rustflags.push(format!(" -C link-arg=--target={}", triple.clang_triple()));
    }

    let mut cargo_args = vec![
        command.to_owned(),
        OsString::from("--target"),
        OsString::from(triple.to_string()),
        OsString::from("--no-default-features"),
    ];
    if triple.is_cross() {
        cargo_args.extend([
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
        ]);
    }

    cargo_args.extend_from_slice(extra_args);

    match verbose {
        0 => {}
        1 => {
            cargo_args.push(OsString::from("-v"));
        }
        _ => {
            cargo_args.push(OsString::from("-vv"));
        }
    }
    if release {
        cargo_args.push(OsString::from("--release"));
    }
    if !features.is_empty() {
        cargo_args.push(OsString::from("--features"));
        cargo_args.extend(features);
    }
    if all_features {
        cargo_args.push(OsString::from("--all-features"));
    }

    match triple.container_image(&container_repository, &container_tag) {
        Some((container_image, _)) => {
            cargo_args.insert(0, OsString::from("cargo"));
            let container = Container {
                args: cargo_args,
                container_engine,
                container_image,
                interactive: false,
                llvm_install_dir,
                pull,
                triple,
                workdir,
            };
            container.run()?;
        }
        None => {
            let mut cmd = Command::new("cargo");
            cmd.args(cargo_args);
        }
    }

    Ok(())
}
