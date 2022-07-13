#![deny(clippy::all)]

#[cfg(feature = "llvm-proxy")]
extern crate aya_rustc_llvm_proxy;

use log::*;
use simplelog::{Config, LevelFilter, SimpleLogger, TermLogger, TerminalMode, WriteLogger};
use std::{collections::HashSet, env, fs::File, str::FromStr};
use std::{fs, path::PathBuf};
use structopt::StructOpt;
use thiserror::Error;

use bpf_linker::{Cpu, Linker, LinkerOptions, OptLevel, OutputType};

#[derive(Debug, Error)]
enum CliError {
    #[error("optimization level needs to be between 0-3, s or z (instead was `{0}`)")]
    InvalidOptimization(String),
    #[error("unknown emission type: `{0}` - expected one of: `llvm-bc`, `asm`, `llvm-ir`, `obj`")]
    InvalidOutputType(String),
}

#[derive(Copy, Clone, Debug)]
struct CliOptLevel(OptLevel);

impl FromStr for CliOptLevel {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use OptLevel::*;
        Ok(CliOptLevel(match s {
            "0" => No,
            "1" => Less,
            "2" => Default,
            "3" => Aggressive,
            "s" => Size,
            "z" => SizeMin,
            _ => return Err(CliError::InvalidOptimization(s.to_string())),
        }))
    }
}

#[derive(Copy, Clone, Debug)]
struct CliOutputType(OutputType);

impl FromStr for CliOutputType {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use OutputType::*;
        Ok(CliOutputType(match s {
            "llvm-bc" => Bitcode,
            "asm" => Assembly,
            "llvm-ir" => LlvmAssembly,
            "obj" => Object,
            _ => return Err(CliError::InvalidOutputType(s.to_string())),
        }))
    }
}
#[derive(Debug, StructOpt)]
struct CommandLine {
    /// LLVM target triple. When not provided, the target is inferred from the inputs
    #[structopt(long)]
    target: Option<String>,

    /// Target BPF processor. Can be one of `generic`, `probe`, `v1`, `v2`, `v3`
    #[structopt(long, default_value = "generic")]
    cpu: Cpu,

    /// Enable or disable CPU features. The available features are: alu32, dummy, dwarfris. Use
    /// +feature to enable a feature, or -feature to disable it.  For example
    /// --cpu-features=+alu32,-dwarfris
    #[structopt(long, value_name = "features", default_value = "")]
    cpu_features: String,

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
    optimize: Vec<CliOptLevel>,

    /// Export the symbols specified in the file `path`. The symbols must be separated by new lines
    #[structopt(long, value_name = "path")]
    export_symbols: Option<PathBuf>,

    /// Output logs to the given `path`
    #[structopt(long, value_name = "path")]
    log_file: Option<PathBuf>,

    /// Set the log level. Can be one of `off`, `info`, `warn`, `debug`, `trace`.
    #[structopt(long, value_name = "level")]
    log_level: Option<LevelFilter>,

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

    /// Disable passing --bpf-expand-memcpy-in-order to LLVM.
    #[structopt(long)]
    disable_expand_memcpy_in_order: bool,

    /// Disble exporting memcpy, memmove, memset, memcmp and bcmp. Exporting
    /// those is commonly needed when LLVM does not manage to expand memory
    /// intrinsics to a sequence of loads and stores.
    #[structopt(long)]
    disable_memory_builtins: bool,

    #[structopt(long)]
    keep_btf: bool,

    /// Input files. Can be object files or static libraries
    inputs: Vec<PathBuf>,

    // The options below are for wasm-ld compatibility
    /// Comma separated list of symbols to export. See also `--export-symbols`
    #[structopt(long, value_name = "symbols", use_delimiter = true, multiple = true)]
    export: Vec<String>,

    #[structopt(short = "l", use_delimiter = true, multiple = true, hidden = true)]
    #[allow(dead_code)]
    lib: Option<String>,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    debug: bool,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    rsp_quoting: Option<String>,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    flavor: Option<String>,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    no_entry: bool,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    gc_sections: bool,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    strip_debug: bool,
    #[structopt(long, hidden = true)]
    #[allow(dead_code)]
    strip_all: bool,
}

fn main() {
    let args = env::args().map(|arg| {
        if arg == "-flavor" {
            "--flavor".to_string()
        } else {
            arg
        }
    });
    let cli = CommandLine::from_iter(args);

    if cli.inputs.is_empty() {
        error("no input files", clap::ErrorKind::TooFewValues);
    }

    let env_log_level = match env::var("RUST_LOG") {
        Ok(s) if !s.is_empty() => match s.parse::<LevelFilter>() {
            Ok(l) => Some(l),
            Err(e) => error(
                &format!("invalid RUST_LOG value: {}", e),
                clap::ErrorKind::InvalidValue,
            ),
        },
        _ => None,
    };
    let log_level = cli.log_level.or(env_log_level).unwrap_or(LevelFilter::Warn);
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
        WriteLogger::init(log_level, Config::default(), log_file).unwrap();
    } else if TermLogger::init(log_level, Config::default(), TerminalMode::Mixed).is_err() {
        SimpleLogger::init(log_level, Config::default()).unwrap();
    }

    info!(
        "command line: {:?}",
        env::args().collect::<Vec<_>>().join(" ")
    );

    let CommandLine {
        target,
        cpu,
        cpu_features,
        inputs,
        output,
        emit,
        libs,
        optimize,
        export_symbols,
        unroll_loops,
        ignore_inline_never,
        dump_module,
        llvm_args,
        disable_expand_memcpy_in_order,
        disable_memory_builtins,
        keep_btf,
        mut export,
        ..
    } = cli;

    let mut export_symbols = export_symbols
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
    export_symbols.extend(export.drain(..));

    let options = LinkerOptions {
        target,
        cpu,
        cpu_features,
        inputs,
        output,
        output_type: emit.0,
        libs,
        optimize: optimize.last().unwrap().0,
        export_symbols,
        unroll_loops,
        ignore_inline_never,
        dump_module,
        llvm_args,
        disable_expand_memcpy_in_order,
        disable_memory_builtins,
        keep_btf,
    };

    if let Err(e) = Linker::new(options).link() {
        error(&e.to_string(), clap::ErrorKind::Io);
    }
}

fn error(desc: &str, kind: clap::ErrorKind) -> ! {
    clap::Error::with_description(desc, kind).exit();
}
