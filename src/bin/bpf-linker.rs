#[cfg(feature = "llvm-proxy")]
extern crate rustc_llvm_proxy;

use anyhow::anyhow;
use log::*;
use simplelog::{Config, LevelFilter, TermLogger, TerminalMode, WriteLogger};
use std::{collections::HashSet, env, fs::File, str::FromStr};
use std::{fs, path::PathBuf};
use structopt::StructOpt;

use bpf_linker::{Cpu, Linker, LinkerOptions, OptLevel, OutputType};

#[derive(Copy, Clone, Debug)]
struct CliOptLevel(OptLevel);

impl FromStr for CliOptLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use OptLevel::*;
        Ok(CliOptLevel(match s {
            "0" => No,
            "1" => Less,
            "2" => Default,
            "3" => Aggressive,
            "s" => Size,
            "z" => SizeMin,
            _ => {
                return Err(anyhow!(
                    "optimization level needs to be between 0-3, s or z (instead was `{}`)",
                    s
                ))
            }
        }))
    }
}

#[derive(Copy, Clone, Debug)]
struct CliOutputType(OutputType);

impl FromStr for CliOutputType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use OutputType::*;
        Ok(CliOutputType(match s {
            "llvm-bc" => Bitcode,
            "asm" => Assembly,
            "llvm-ir" => LlvmAssembly,
            "obj" => Object,
            _ => {
                return Err(anyhow!(
                "unknown emission type: `{}` - expected one of: `llvm-bc`, `asm`, `llvm-ir`, `obj`",
                s
            ))
            }
        }))
    }
}
#[derive(Debug, StructOpt)]
struct CommandLine {
    /// Target BPF processor. Can be one of `generic`, `probe`, `v1`, `v2`, `v3`
    #[structopt(long, default_value = "generic")]
    cpu: Cpu,

    /// Enable or disable CPU features. The available features are: alu32, dummy, dwarfris. Use
    /// +feature to enable a feature, or -feature to disable it.  For example
    /// --cpu-features=+alu32,-dwarfris
    #[structopt(long, value_name = "features", default_value = "")]
    cpu_features: String,

    #[structopt(long, number_of_values = 1)]
    rlib: Vec<PathBuf>,

    /// Write output to <output>
    #[structopt(short, long)]
    output: PathBuf,

    /// Output type. Can be one of `llvm-bc`, `asm`, `llvm-ir`, `obj`
    #[structopt(long, default_value = "obj")]
    emit: CliOutputType,

    /// Add a directory to the library search path
    #[structopt(short = "L", number_of_values = 1)]
    libs: Vec<PathBuf>,

    /// Optimization level. 0-3, s, or z
    #[structopt(short = "O", default_value = "2", multiple = true)]
    optimize: CliOptLevel,

    /// Export the symbols specified in the file `path`. The symbols are separated by new lines
    #[structopt(long, value_name = "path")]
    export_symbols: Option<PathBuf>,

    /// Output logs to the given `path`
    #[structopt(long, value_name = "path")]
    log_file: Option<PathBuf>,

    /// Set the log level. Can be one of `off`, `info`, `warn`, `debug`, `trace`.
    #[structopt(long, value_name = "level", default_value = "warn")]
    log_level: LevelFilter,

    /// Try hard to unroll loops. Useful when targeting kernels that don't support loops
    #[structopt(long)]
    unroll_loops: bool,

    /// Ignore `noinline`/`#[inline(never)]`. Useful when targeting kernels that don't support function calls
    #[structopt(long)]
    ignore_inline_never: bool,

    /// Dump the final IR module to the given `path` before generating the code
    #[structopt(long, value_name = "path")]
    dump_module: Option<PathBuf>,

    /// Extra command line arguments to pass to LLVM
    #[structopt(long, value_name = "args", use_delimiter = true, multiple = true)]
    llvm_args: Vec<String>,

    /// Bitcode files
    bitcode: Vec<PathBuf>,
}

fn main() {
    let cli = CommandLine::from_args();

    if cli.bitcode.is_empty() {
        error("no input files", clap::ErrorKind::TooFewValues);
    }

    if let Some(path) = cli.log_file.clone() {
        let log_file = match File::create(path) {
            Ok(f) => f,
            Err(e) => {
                error(
                    &format!("failed to open log file: {:?}", e),
                    clap::ErrorKind::Io,
                );
            }
        };
        WriteLogger::init(cli.log_level, Config::default(), log_file).unwrap();
    } else {
        TermLogger::init(cli.log_level, Config::default(), TerminalMode::Mixed).unwrap();
    }

    info!(
        "command line: {:?}",
        env::args().collect::<Vec<_>>().join(" ")
    );

    let CommandLine {
        cpu,
        cpu_features,
        bitcode,
        rlib,
        output,
        emit,
        libs,
        optimize,
        export_symbols,
        unroll_loops,
        ignore_inline_never,
        dump_module,
        llvm_args,
        ..
    } = cli;

    let export_symbols = export_symbols
        .map(|path| match fs::read_to_string(path) {
            Ok(symbols) => symbols
                .lines()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
            Err(e) => {
                error(&e.to_string(), clap::ErrorKind::Io);
            }
        })
        .unwrap_or_else(HashSet::new);

    let options = LinkerOptions {
        cpu,
        cpu_features,
        bitcode,
        rlib,
        output,
        output_type: emit.0,
        libs,
        optimize: optimize.0,
        export_symbols,
        unroll_loops,
        ignore_inline_never,
        dump_module,
        llvm_args,
    };

    if let Err(e) = Linker::new(options).link() {
        error(&e.to_string(), clap::ErrorKind::Io);
    }
}

fn error(desc: &str, kind: clap::ErrorKind) -> ! {
    clap::Error::with_description(desc, kind).exit();
}
