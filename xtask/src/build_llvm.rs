use std::{
    ffi::{OsStr, OsString},
    fs::{self, create_dir_all, remove_dir_all},
    path::Path,
    process::{Command, Stdio},
};

use clap::Parser;
use target_lexicon::Triple;
use thiserror::Error;

use crate::{
    containers::{ContainerEngine, ContainerError},
    target::{SupportedTriple, TripleExt},
    tempdir::TempDir,
};

#[derive(Debug, Error)]
pub enum LlvmBuildError {
    #[error(transparent)]
    Container(ContainerError),
    #[error("target {0} is not supported")]
    TargetNotSupported(String),
    #[error("cmake build failed")]
    CmakeBuild,
}

#[derive(Parser)]
pub struct BuildLlvmArgs {
    /// Container engine (if not provided, is going to be autodetected).
    #[arg(long)]
    container_engine: Option<ContainerEngine>,

    /// Prefix in which LLVM libraries are going to be installed after build.
    #[arg(long)]
    llvm_install_dir: Option<OsString>,

    /// Path to the LLVM repository directory. If not provided, it will be
    /// cloned automatically in a temporary location.
    #[arg(long)]
    llvm_repository_dir: Option<OsString>,

    /// URL to the LLVM repository. Irrelevant if `--llvm-repository-dir` is
    /// specified.
    #[arg(long, default_value = "https://github.com/aya-rs/llvm-project")]
    llvm_repository_url: String,

    /// Branch of the LLVM repository. Irrelevant if `--llvm-repository-dir` is
    /// specified.
    #[arg(long, default_value = "rustc/19.1-2024-09-17")]
    llvm_repository_branch: String,

    /// Preserve the build directory.
    #[arg(long)]
    preserve_build_dir: bool,

    /// Target triple (optional).
    #[arg(short, long)]
    target: Option<SupportedTriple>,
}

fn clone_repo(
    llvm_repository_url: &String,
    llvm_repository_branch: &str,
    destination: &OsStr,
) -> anyhow::Result<()> {
    // NOTE(vadorovsky): Sadly, git2 crate doesn't support specyfing depth when
    // cloning.
    Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(&llvm_repository_branch)
        .arg(&llvm_repository_url)
        .arg(destination)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    Ok(())
}

pub fn build_llvm(args: BuildLlvmArgs) -> anyhow::Result<()> {
    let BuildLlvmArgs {
        container_engine,
        llvm_install_dir,
        llvm_repository_dir,
        llvm_repository_url,
        llvm_repository_branch,
        preserve_build_dir,
        target,
    } = args;

    let build_tempdir = TempDir::new("aya-llvm-build", preserve_build_dir)?;

    let llvm_repository_dir = match llvm_repository_dir {
        Some(llvm_repository_dir) => llvm_repository_dir,
        None => {
            let destination = build_tempdir.to_os_string();
            clone_repo(&llvm_repository_url, &llvm_repository_branch, &destination)?;
            destination
        }
    };
    println!(
        "Building LLVM in directory {}",
        llvm_repository_dir.to_string_lossy()
    );

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
    if Path::new(&llvm_install_dir).exists() {
        remove_dir_all(&llvm_install_dir)?;
    }
    create_dir_all(&llvm_install_dir)?;

    let llvm_build_config = triple
        .llvm_build_config(&llvm_install_dir)
        .ok_or(LlvmBuildError::TargetNotSupported(triple.to_string()))?;

    let cmake_args = llvm_build_config.cmake_args();

    let build_dir = format!("aya-build-{}", llvm_build_config.target_triple);
    let build_dir_path = Path::new(&llvm_repository_dir).join(&build_dir);
    if build_dir_path.exists() {
        fs::remove_dir_all(Path::new(&llvm_repository_dir).join(&build_dir))?;
    }

    match triple.container_image() {
        Some((container_image, _)) => {
            println!("Using container image {container_image}");

            let container_engine =
                container_engine.unwrap_or(ContainerEngine::autodetect().ok_or(
                    LlvmBuildError::Container(ContainerError::ContainerEngineNotFound),
                )?);

            let mut cmd = Command::new(container_engine.to_string());
            cmd.args([
                "run",
                "--rm",
                "-it",
                "-w",
                "/usr/local/src/llvm",
                "-v",
                &format!(
                    "{}:/usr/local/src/llvm:z",
                    llvm_repository_dir.to_string_lossy()
                ),
                "-v",
                &format!(
                    "{}:{}",
                    llvm_install_dir.to_string_lossy(),
                    llvm_install_dir.to_string_lossy()
                ),
                &container_image,
                "cmake",
            ])
            .args(cmake_args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(LlvmBuildError::CmakeBuild.into());
            }

            let mut cmd = Command::new(container_engine.to_string());
            cmd.args([
                "run",
                "--rm",
                "-e",
                // "PATH=/usr/lib/llvm/18/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "-it",
                "-w",
                "/usr/local/src/llvm",
                "-v",
                &format!(
                    "{}:/usr/local/src/llvm",
                    llvm_repository_dir.to_string_lossy()
                ),
                "-v",
                &format!(
                    "{}:{}",
                    llvm_install_dir.to_string_lossy(),
                    llvm_install_dir.to_string_lossy()
                ),
                &container_image,
                "cmake",
                "--build",
                &build_dir,
                "-j",
                "--target",
                "install",
            ])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(LlvmBuildError::CmakeBuild.into());
            }
        }
        None => {
            println!("Building on host");

            let mut cmd = Command::new("cmake");
            cmd.args(cmake_args)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(LlvmBuildError::CmakeBuild.into());
            }

            let mut cmd = Command::new("cmake");
            cmd.args(["--build", &build_dir, "-j", "--target", "install"])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            println!("{cmd:?}");
            if !cmd.status()?.success() {
                return Err(LlvmBuildError::CmakeBuild.into());
            }
        }
    }

    println!(
        "Installed LLVM artifacts in {}",
        llvm_install_dir.to_string_lossy()
    );

    Ok(())
}
