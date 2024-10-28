use std::ffi::OsString;

use clap::Parser;
use target_lexicon::Triple;

use crate::{
    containers::{Container, ContainerEngine, PullPolicy},
    git::top_directory,
    target::{SupportedTriple, TripleExt},
};

#[derive(Parser)]
pub struct ShellArgs {
    /// Container engine (if not provided, is going to be autodetected).
    #[arg(long)]
    container_engine: Option<ContainerEngine>,

    /// Container image repository.
    #[arg(long, default_value = "ghcr.io/exein-io/cross-llvm")]
    container_repository: String,

    /// Tag of the container image.
    #[arg(long, default_value = "latest")]
    container_tag: String,

    /// Prefix in which LLVM libraries are going to be installed after build.
    #[arg(long)]
    llvm_install_dir: Option<OsString>,

    /// Pull image policy.
    #[arg(long, default_value_t = PullPolicy::default())]
    pull: PullPolicy,

    /// Target triple (optional).
    #[arg(short, long)]
    target: Option<SupportedTriple>,
}

pub fn run_shell(args: ShellArgs) -> anyhow::Result<()> {
    let ShellArgs {
        container_engine,
        container_repository,
        container_tag,
        llvm_install_dir,
        pull,
        target,
    } = args;

    let triple: Triple = match target {
        Some(target) => target.into(),
        None => target_lexicon::HOST,
    };

    let workdir = top_directory().unwrap();

    match triple.container_image(&container_repository, &container_tag) {
        Some((container_image, _)) => {
            let container = Container {
                args: vec![OsString::from("bash")],
                container_engine,
                container_image,
                interactive: true,
                llvm_install_dir,
                pull,
                triple,
                workdir,
            };
            container.run()?;
        }
        None => {
            println!("Shell is supported only for containerized targets");
        }
    }

    Ok(())
}
