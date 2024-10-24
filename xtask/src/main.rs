use clap::{Parser, Subcommand};

mod build;
mod build_llvm;
mod cargo;
mod containers;
mod llvm;
mod target;
mod tempdir;
mod test;

use crate::{
    build::build,
    build_llvm::{build_llvm, BuildLlvmArgs},
    cargo::CargoArgs,
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
        Commands::Test(args) => test(args)?,
    }

    Ok(())
}
