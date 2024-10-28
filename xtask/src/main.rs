use clap::{Parser, Subcommand};
use install::install;
use shell::{run_shell, ShellArgs};

mod build;
mod build_llvm;
mod cargo;
mod check;
mod containers;
mod git;
mod install;
mod llvm;
mod shell;
mod target;
mod tempdir;
mod test;

use crate::{
    build::build,
    build_llvm::{build_llvm, BuildLlvmArgs},
    cargo::CargoArgs,
    check::check,
    test::test,
};

/// The `xtask` CLI.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Subcommands
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build bpf-linker.
    Build(CargoArgs),
    /// Build LLVM.
    BuildLlvm(BuildLlvmArgs),
    /// Check the bpf-linker package.
    Check(CargoArgs),
    /// Install bpf-linker.
    Install(CargoArgs),
    /// Run a shell inside a cross environment.
    Shell(ShellArgs),
    /// Test bpf-linker.
    Test(CargoArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build(args) => build(args)?,
        Commands::BuildLlvm(args) => {
            build_llvm(args)?;
        }
        Commands::Check(args) => check(args)?,
        Commands::Install(args) => install(args)?,
        Commands::Shell(args) => run_shell(args)?,
        Commands::Test(args) => test(args)?,
    }

    Ok(())
}
