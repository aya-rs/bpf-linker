use std::{
    env,
    ffi::{OsStr, OsString},
    fmt::Display,
    path::Path,
    process::{Command, Stdio},
};

use clap::ValueEnum;
use target_lexicon::Triple;
use thiserror::Error;
use which::which;

use crate::target::TripleExt;

#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("no supported container engine (docker, podman) was found")]
    ContainerEngineNotFound,
    #[error("failed to execute the container")]
    Run,
}

#[derive(Clone, ValueEnum)]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl Display for ContainerEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Docker => write!(f, "docker"),
            Self::Podman => write!(f, "podman"),
        }
    }
}

impl ContainerEngine {
    pub fn autodetect() -> Result<Self, ContainerError> {
        if which("docker").is_ok() {
            Ok(Self::Docker)
        } else if which("podman").is_ok() {
            Ok(Self::Podman)
        } else {
            Err(ContainerError::ContainerEngineNotFound)
        }
    }
}

#[derive(Clone, ValueEnum)]
pub enum PullPolicy {
    Always,
    Missing,
    Never,
    Newer,
}

impl Default for PullPolicy {
    fn default() -> Self {
        Self::Missing
    }
}

impl Display for PullPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Always => write!(f, "always"),
            Self::Missing => write!(f, "missing"),
            Self::Never => write!(f, "never"),
            Self::Newer => write!(f, "newer"),
        }
    }
}

pub struct Container {
    pub args: Vec<OsString>,
    pub container_engine: Option<ContainerEngine>,
    pub container_image: String,
    pub interactive: bool,
    pub llvm_install_dir: Option<OsString>,
    pub pull: PullPolicy,
    pub triple: Triple,
    pub workdir: OsString,
}

impl Container {
    pub fn run(&self) -> anyhow::Result<()> {
        let Self {
            args,
            container_engine,
            container_image,
            interactive,
            llvm_install_dir,
            pull,
            triple,
            workdir,
        } = self;

        println!("Using container image {container_image}");

        let container_engine = container_engine
            .clone()
            .unwrap_or(ContainerEngine::autodetect()?);

        let llvm_install_dir = match llvm_install_dir {
            Some(llvm_install_dir) => llvm_install_dir,
            None => &Path::new("/tmp")
                .join(format!("aya-llvm-{triple}"))
                .into_os_string(),
        };

        let mut llvm_prefix = OsString::from("BPF_LINKER_LLVM_PREFIX=");
        llvm_prefix.push(llvm_install_dir);

        let rustup_toolchain = env::var("RUSTUP_TOOLCHAIN").unwrap();
        let rustup_toolchain = rustup_toolchain.split('-').next().unwrap();
        let mut rustup_toolchain_triple = target_lexicon::HOST;
        rustup_toolchain_triple.environment = triple.environment;
        let rustup_toolchain = format!("{rustup_toolchain}-{}", rustup_toolchain_triple);
        let mut rustup_toolchain_arg = OsString::from("RUSTUP_TOOLCHAIN=");
        rustup_toolchain_arg.push(rustup_toolchain);

        let cargo_dir = Path::new(&env::var_os("HOME").unwrap()).join(".cargo");
        let mut cargo_dir_arg = cargo_dir.into_os_string();
        cargo_dir_arg.push(":/root/host-cargo:z");

        let mut workdir_arg = workdir.clone();
        workdir_arg.push(":/home/cross/src:z");

        let mut llvm_install_arg = llvm_install_dir.clone();
        llvm_install_arg.push(":");
        llvm_install_arg.push(llvm_install_dir);

        let mut cmd = Command::new(container_engine.to_string());
        cmd.args([
            OsStr::new("run"),
            OsStr::new("--rm"),
            OsStr::new("-e"),
            &llvm_prefix,
            OsStr::new("-e"),
            &triple.rustflags(),
            OsStr::new("-e"),
            &rustup_toolchain_arg,
        ]);
        if triple.is_cross() {
            let mut qemu = OsString::from("BPF_LINKER_QEMU=");
            qemu.push(triple.qemu());
            cmd.args([OsStr::new("-e"), &qemu]);
        }
        if *interactive {
            cmd.arg("-i");
        }
        cmd.args([
            OsStr::new("-t"),
            OsStr::new("--pull"),
            OsStr::new(&pull.to_string()),
            OsStr::new("-w"),
            OsStr::new("/home/cross/src"),
            OsStr::new("-v"),
            &cargo_dir_arg,
            OsStr::new("-v"),
            &workdir_arg,
            OsStr::new("-v"),
            &llvm_install_arg,
            OsStr::new(&container_image),
        ]);
        cmd.args(args);
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        println!("{cmd:?}");
        if !cmd.status()?.success() {
            return Err(ContainerError::Run.into());
        }

        Ok(())
    }
}
