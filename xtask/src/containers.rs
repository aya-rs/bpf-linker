use std::process::{Command, Stdio};

use chrono::Utc;
use clap::{Parser, ValueEnum};
use target_lexicon::Triple;
use thiserror::Error;
use which::which;

use crate::target::{SupportedTriple, TripleExt};

#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("no supported container engine (docker, podman) was found")]
    ContainerEngineNotFound,
    #[error("containerized builds are not supported for target {0}")]
    UnsupportedTarget(String),
    #[error("failed to build a container image")]
    ContainerImageBuild,
    #[error("failed to push a container image")]
    ContainerImagePush,
    #[error("failed to tag a container image as latest")]
    ContainerImageTag,
}

#[derive(Clone, ValueEnum)]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl ToString for ContainerEngine {
    fn to_string(&self) -> String {
        match self {
            Self::Docker => "docker".to_owned(),
            Self::Podman => "podman".to_owned(),
        }
    }
}

impl ContainerEngine {
    pub fn autodetect() -> Option<Self> {
        if which("docker").is_ok() {
            return Some(Self::Docker);
        }
        if which("podman").is_ok() {
            return Some(Self::Podman);
        }
        None
    }
}

#[derive(Parser)]
pub struct BuildContainerImageArgs {
    /// Container engine (if not provided, is going to be autodetected)
    #[arg(long)]
    container_engine: Option<ContainerEngine>,

    /// Do not use existing cached images for the container build. Build from
    /// the start with a new set of cached layers.
    #[arg(long)]
    no_cache: bool,

    /// Push the image after build.
    #[arg(long)]
    push: bool,

    /// Target triple (optional)
    #[arg(short, long)]
    target: Option<SupportedTriple>,
}

fn push_image(container_engine: &ContainerEngine, tag: &str) -> anyhow::Result<()> {
    let mut cmd = Command::new(container_engine.to_string());
    cmd.args(["push", tag])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    println!("{cmd:?}");
    if !cmd.status()?.success() {
        return Err(ContainerError::ContainerImagePush.into());
    }
    Ok(())
}

pub fn build_container_image(args: BuildContainerImageArgs) -> anyhow::Result<()> {
    let BuildContainerImageArgs {
        container_engine,
        no_cache,
        push,
        target,
    } = args;

    let triple: Triple = match target {
        Some(target) => target.into(),
        None => target_lexicon::HOST,
    };

    match triple.container_image() {
        Some((tag, dockerfile)) => {
            let container_engine = container_engine.unwrap_or(
                ContainerEngine::autodetect().ok_or(ContainerError::ContainerEngineNotFound)?,
            );

            let date = Utc::now().format("%Y%m%d");
            let tag_with_date = format!("{tag}:{date}");
            let tag_latest = format!("{tag}:latest");

            let mut cmd = Command::new(container_engine.to_string());
            cmd.args([
                "buildx",
                "build",
                "-t",
                &tag_with_date,
                "-f",
                &dockerfile,
                ".",
            ])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
            if no_cache {
                cmd.arg("--no-cache");
            }
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(ContainerError::ContainerImageBuild.into());
            }

            let mut cmd = Command::new(container_engine.to_string());
            cmd.args(["tag", &tag_with_date, &tag_latest])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(ContainerError::ContainerImageTag.into());
            }

            if push {
                push_image(&container_engine, &tag_with_date)?;
                push_image(&container_engine, &tag_latest)?;
            }

            Ok(())
        }
        None => Err(ContainerError::UnsupportedTarget(triple.to_string()).into()),
    }
}
