#![deny(clippy::all)]

#[cfg(feature = "rust-llvm")]
extern crate aya_rustc_llvm_proxy;

use clap::Parser;
use log::*;
use simplelog::{Config, LevelFilter, SimpleLogger, TermLogger, TerminalMode, WriteLogger};
use std::{
    collections::HashSet,
    env,
    fs::{self, File},
    path::PathBuf,
    str::FromStr,
};
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
#[derive(Debug, Parser)]
struct CommandLine {
    /// LLVM target triple. When not provided, the target is inferred from the inputs
    #[clap(long)]
    target: Option<String>,

    /// Target BPF processor. Can be one of `generic`, `probe`, `v1`, `v2`, `v3`
    #[clap(long, default_value = "generic")]
    cpu: Cpu,

    /// Enable or disable CPU features. The available features are: alu32, dummy, dwarfris. Use
    /// +feature to enable a feature, or -feature to disable it.  For example
    /// --cpu-features=+alu32,-dwarfris
    #[clap(long, value_name = "features", default_value = "")]
    cpu_features: String,

    /// Write output to <output>
    #[clap(short, long)]
    output: PathBuf,

    /// Output type. Can be one of `llvm-bc`, `asm`, `llvm-ir`, `obj`
    #[clap(long, default_value = "obj")]
    emit: CliOutputType,

    /// Add a directory to the library search path
    #[clap(short = 'L', number_of_values = 1)]
    libs: Vec<PathBuf>,

    /// Optimization level. 0-3, s, or z
    #[clap(short = 'O', default_value = "2")]
    optimize: Vec<CliOptLevel>,

    /// Export the symbols specified in the file `path`. The symbols must be separated by new lines
    #[clap(long, value_name = "path")]
    export_symbols: Option<PathBuf>,

    /// Output logs to the given `path`
    #[clap(long, value_name = "path")]
    log_file: Option<PathBuf>,

    /// Set the log level. Can be one of `off`, `info`, `warn`, `debug`, `trace`.
    #[clap(long, value_name = "level")]
    log_level: Option<LevelFilter>,

    /// Try hard to unroll loops. Useful when targeting kernels that don't support loops
    #[clap(long)]
    unroll_loops: bool,

    /// Ignore `noinline`/`#[inline(never)]`. Useful when targeting kernels that don't support function calls
    #[clap(long)]
    ignore_inline_never: bool,

    /// Dump the final IR module to the given `path` before generating the code
    #[clap(long, value_name = "path")]
    dump_module: Option<PathBuf>,

    /// Extra command line arguments to pass to LLVM
    #[clap(long, value_name = "args", use_value_delimiter = true, action = clap::ArgAction::Append)]
    llvm_args: Vec<String>,

    /// Disable passing --bpf-expand-memcpy-in-order to LLVM.
    #[clap(long)]
    disable_expand_memcpy_in_order: bool,

    /// Disble exporting memcpy, memmove, memset, memcmp and bcmp. Exporting
    /// those is commonly needed when LLVM does not manage to expand memory
    /// intrinsics to a sequence of loads and stores.
    #[clap(long)]
    disable_memory_builtins: bool,

    /// Input files. Can be object files or static libraries
    inputs: Vec<PathBuf>,

    // The options below are for wasm-ld compatibility
    /// Comma separated list of symbols to export. See also `--export-symbols`
    #[clap(long, value_name = "symbols", use_value_delimiter = true, action = clap::ArgAction::Append)]
    export: Vec<String>,

    #[clap(
        short = 'l',
        long = "lib",
        use_value_delimiter = true,
        action = clap::ArgAction::Append,
        hide = true
    )]
    _lib: Option<String>,
    #[clap(long = "debug", hide = true)]
    _debug: bool,
    #[clap(long = "rsp-quoting", hide = true)]
    _rsp_quoting: Option<String>,
    #[clap(long = "flavor", hide = true)]
    _flavor: Option<String>,
    #[clap(long = "no-entry", hide = true)]
    _no_entry: bool,
    #[clap(long = "gc-sections", hide = true)]
    _gc_sections: bool,
    #[clap(long = "strip-debug", hide = true)]
    _strip_debug: bool,
    #[clap(long = "strip-all", hide = true)]
    _strip_all: bool,
}

fn main() {
    let args = env::args().map(|arg| {
        if arg == "-flavor" {
            "--flavor".to_string()
        } else {
            arg
        }
    });
    let cli = CommandLine::parse_from(args);

    if cli.inputs.is_empty() {
        error("no input files", clap::error::ErrorKind::TooFewValues);
    }

    let env_log_level = match env::var("RUST_LOG") {
        Ok(s) if !s.is_empty() => match s.parse::<LevelFilter>() {
            Ok(l) => Some(l),
            Err(e) => error(
                &format!("invalid RUST_LOG value: {e}"),
                clap::error::ErrorKind::InvalidValue,
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
                    &format!("failed to open log file: {e:?}"),
                    clap::error::ErrorKind::Io,
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
                error(&e.to_string(), clap::error::ErrorKind::Io);
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
    };

    if let Err(e) = Linker::new(options).link() {
        error(&e.to_string(), clap::error::ErrorKind::Io);
    }
}

fn error(desc: &str, kind: clap::error::ErrorKind) -> ! {
    clap::Error::raw(kind, desc.to_string()).exit();
}

#[cfg(test)]
mod test {
    use super::*;

    // Test made to reproduce the following bug:
    // https://github.com/aya-rs/bpf-linker/issues/27
    // where --export argument followed by positional arguments resulted in
    // parsing the positional args as `export`, not as `inputs`.
    // There can be multiple exports, but they always have to be preceded by
    // `--export` flag.
    #[test]
    fn test_export_input_args() {
        let args = vec![
            "bpf-linker",
            "--export",
            "foo",
            "--export",
            "bar",
            "symbols.o", // this should be parsed as `input`, not `export`
            "rcgu.o",    // this should be parsed as `input`, not `export`
            "-L",
            "target/debug/deps",
            "-L",
            "target/debug",
            "-L",
            "/home/foo/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib",
            "-o",
            "/tmp/bin.s",
            "--target=bpf",
            "--emit=asm",
        ];
        let cli = CommandLine::parse_from(args);
        assert_eq!(cli.export, vec!["foo", "bar"]);
        assert_eq!(
            cli.inputs,
            vec![PathBuf::from("symbols.o"), PathBuf::from("rcgu.o")]
        );
    }

    #[test]
    fn test_export_delimiter() {
        let args = vec![
            "bpf-linker",
            "--export",
            "foo,bar",
            "--export=ayy,lmao",
            "symbols.o", // this should be parsed as `input`, not `export`
            "--export=lol",
            "--export",
            "rotfl",
            "rcgu.o", // this should be parsed as `input`, not `export`
            "-L",
            "target/debug/deps",
            "-L",
            "target/debug",
            "-L",
            "/home/foo/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib",
            "-o",
            "/tmp/bin.s",
            "--target=bpf",
            "--emit=asm",
        ];
        let cli = CommandLine::parse_from(args);
        assert_eq!(
            cli.export,
            vec!["foo", "bar", "ayy", "lmao", "lol", "rotfl"]
        );
        assert_eq!(
            cli.inputs,
            vec![PathBuf::from("symbols.o"), PathBuf::from("rcgu.o")]
        );
    }
}
