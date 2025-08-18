#![deny(clippy::all)]

#[cfg(any(
    feature = "rust-llvm-19",
    feature = "rust-llvm-20",
    feature = "rust-llvm-21"
))]
extern crate aya_rustc_llvm_proxy;

use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use bpf_linker::{Cpu, Linker, LinkerInput, LinkerOptions, OptLevel, OutputType};
use clap::{
    builder::{PathBufValueParser, TypedValueParser as _},
    error::ErrorKind,
    Parser,
};
use thiserror::Error;
use tracing::{info, Level};
use tracing_subscriber::{fmt::MakeWriter, prelude::*, EnvFilter};
use tracing_tree::HierarchicalLayer;

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

fn parent_and_file_name(p: PathBuf) -> anyhow::Result<(PathBuf, PathBuf)> {
    let mut comps = p.components();
    let file_name = comps
        .next_back()
        .map(|p| match p {
            Component::Normal(p) => Ok(p),
            p => Err(anyhow::anyhow!("unexpected path component {:?}", p)),
        })
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("unexpected empty path"))?;
    let parent = comps.as_path();
    Ok((parent.to_path_buf(), Path::new(file_name).to_path_buf()))
}

#[derive(Debug, Parser)]
#[command(version)]
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
    emit: Vec<CliOutputType>,

    /// Emit BTF information
    #[clap(long)]
    btf: bool,

    /// Permit automatic insertion of __bpf_trap calls.
    /// See: https://github.com/llvm/llvm-project/commit/ab391beb11f733b526b86f9df23734a34657d876
    #[clap(long)]
    allow_bpf_trap: bool,

    /// Optimization level. 0-3, s, or z
    #[clap(short = 'O', default_value = "2")]
    optimize: Vec<CliOptLevel>,

    /// Export the symbols specified in the file `path`. The symbols must be separated by new lines
    #[clap(long, value_name = "path")]
    export_symbols: Option<PathBuf>,

    /// Output logs to the given `path`
    #[clap(
        long,
        value_name = "path",
        value_parser = PathBufValueParser::new().try_map(parent_and_file_name),
    )]
    log_file: Option<(PathBuf, PathBuf)>,

    /// Set the log level. If not specified, no logging is used. Can be one of
    /// `error`, `warn`, `info`, `debug`, `trace`.
    #[clap(long, value_name = "level")]
    log_level: Option<Level>,

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

    /// Disable exporting memcpy, memmove, memset, memcmp and bcmp. Exporting
    /// those is commonly needed when LLVM does not manage to expand memory
    /// intrinsics to a sequence of loads and stores.
    #[clap(long)]
    disable_memory_builtins: bool,

    /// Input files. Can be object files or static libraries
    #[clap(required = true)]
    inputs: Vec<PathBuf>,

    /// Comma separated list of symbols to export. See also `--export-symbols`
    #[clap(long, value_name = "symbols", use_value_delimiter = true, action = clap::ArgAction::Append)]
    export: Vec<String>,

    /// Whether to treat LLVM errors as fatal.
    #[clap(long, action = clap::ArgAction::Set, default_value_t = true)]
    fatal_errors: bool,

    // The options below are for wasm-ld compatibility
    #[clap(long = "debug", hide = true)]
    _debug: bool,
}

/// Returns a [`HierarchicalLayer`](tracing_tree::HierarchicalLayer) for the
/// given `writer`.
fn tracing_layer<W>(writer: W) -> HierarchicalLayer<W>
where
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    const TRACING_IDENT: usize = 2;
    HierarchicalLayer::new(TRACING_IDENT)
        .with_indent_lines(true)
        .with_writer(writer)
}
fn main() -> anyhow::Result<()> {
    let args = env::args().map(|arg| {
        if arg == "-flavor" {
            "--flavor".to_string()
        } else {
            arg
        }
    });
    let CommandLine {
        target,
        cpu,
        cpu_features,
        output,
        emit,
        btf,
        allow_bpf_trap,
        optimize,
        export_symbols,
        log_file,
        log_level,
        unroll_loops,
        ignore_inline_never,
        dump_module,
        llvm_args,
        disable_expand_memcpy_in_order,
        disable_memory_builtins,
        inputs,
        export,
        fatal_errors,
        _debug,
    } = match Parser::try_parse_from(args) {
        Ok(command_line) => command_line,
        Err(err) => match err.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                print!("{err}");
                return Ok(());
            }
            _ => return Err(err.into()),
        },
    };

    // Configure tracing.
    let _guard = {
        let filter = EnvFilter::from_default_env();
        let filter = match log_level {
            None => filter,
            Some(log_level) => filter.add_directive(log_level.into()),
        };
        let subscriber_registry = tracing_subscriber::registry().with(filter);
        match log_file {
            Some((parent, file_name)) => {
                let file_appender = tracing_appender::rolling::never(parent, file_name);
                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                let subscriber = subscriber_registry
                    .with(tracing_layer(io::stdout))
                    .with(tracing_layer(non_blocking));
                tracing::subscriber::set_global_default(subscriber)?;
                Some(guard)
            }
            None => {
                let subscriber = subscriber_registry.with(tracing_layer(io::stderr));
                tracing::subscriber::set_global_default(subscriber)?;
                None
            }
        }
    };

    info!(
        "command line: {:?}",
        env::args().collect::<Vec<_>>().join(" ")
    );

    let export_symbols = export_symbols.map(fs::read_to_string).transpose()?;

    // TODO: the data is owned by this call frame; we could make this zero-alloc.
    let export_symbols = export_symbols
        .as_deref()
        .into_iter()
        .flat_map(str::lines)
        .map(str::to_owned)
        .chain(export)
        .map(Into::into)
        .collect();

    let output_type = match *emit.as_slice() {
        [] => unreachable!("emit has a default value"),
        [CliOutputType(output_type), ..] => output_type,
    };
    let optimize = match *optimize.as_slice() {
        [] => unreachable!("emit has a default value"),
        [.., CliOptLevel(optimize)] => optimize,
    };

    let linker = Linker::new(LinkerOptions {
        target,
        cpu,
        cpu_features,
        optimize,
        unroll_loops,
        ignore_inline_never,
        llvm_args,
        disable_expand_memcpy_in_order,
        disable_memory_builtins,
        btf,
        allow_bpf_trap,
    })?;

    let inputs: Vec<LinkerInput> = inputs
        .iter()
        .map(|p| LinkerInput::try_from(p.as_path()))
        .collect::<Result<_, _>>()?;

    linker.link_to_file(
        inputs,
        &output,
        output_type,
        &export_symbols,
        dump_module.as_deref(),
    )?;

    if fatal_errors && linker.has_errors() {
        return Err(anyhow::anyhow!(
            "LLVM issued diagnostic with error severity"
        ));
    }

    Ok(())
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
        let args = [
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
        let CommandLine { inputs, export, .. } = Parser::parse_from(args);
        assert_eq!(export, ["foo", "bar"]);
        assert_eq!(
            inputs,
            [PathBuf::from("symbols.o"), PathBuf::from("rcgu.o")]
        );
    }

    #[test]
    fn test_export_delimiter() {
        let args = [
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
        let CommandLine { inputs, export, .. } = Parser::parse_from(args);
        assert_eq!(export, ["foo", "bar", "ayy", "lmao", "lol", "rotfl"]);
        assert_eq!(
            inputs,
            [PathBuf::from("symbols.o"), PathBuf::from("rcgu.o")]
        );
    }
}
