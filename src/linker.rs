use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{CStr, CString, OsStr},
    fs,
    io::{self, Read as _},
    ops::Deref,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    str::{self, FromStr},
};

use ar::Archive;
use llvm_sys::{
    error_handling::{LLVMEnablePrettyStackTrace, LLVMInstallFatalErrorHandler},
    target_machine::LLVMCodeGenFileType,
};
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::llvm::{self, LLVMContext, LLVMModule, LLVMTargetMachine, MemoryBuffer};

/// Linker error
#[derive(Debug, Error)]
pub enum LinkerError {
    /// Invalid Cpu.
    #[error("invalid CPU {0}")]
    InvalidCpu(String),

    /// Invalid LLVM target.
    #[error("invalid LLVM target {0}")]
    InvalidTarget(String),

    /// An IO Error occurred while linking a module.
    #[error("`{0}`: {1}")]
    IoError(PathBuf, io::Error),

    /// The file is not bitcode, an object file containing bitcode or an archive file.
    #[error("invalid input file `{0}`")]
    InvalidInputType(PathBuf),

    /// Linking a module failed.
    #[error("failure linking module {0}")]
    LinkModuleError(PathBuf),

    /// Parsing an IR module failed.
    #[error("failure parsing IR module `{0}`: {1}")]
    IRParseError(PathBuf, String),

    /// Linking a module included in an archive failed.
    #[error("failure linking module {1} from {0}")]
    LinkArchiveModuleError(PathBuf, PathBuf),

    /// Optimizing the BPF code failed.
    #[error("LLVMRunPasses failed: {0}")]
    OptimizeError(String),

    /// Generating the BPF code failed.
    #[error("LLVMTargetMachineEmitToFile failed: {0}")]
    EmitCodeError(String),

    /// Writing the bitcode failed.
    #[error("LLVMWriteBitcodeToFile failed: {0}")]
    WriteBitcodeError(io::Error),

    /// Writing the LLVM IR failed.
    #[error("LLVMPrintModuleToFile failed: {0}")]
    WriteIRError(String),

    /// There was an error extracting the bitcode embedded in an object file.
    #[error("error reading embedded bitcode: {0}")]
    EmbeddedBitcodeError(String),

    /// The input object file does not have embedded bitcode.
    #[error("no bitcode section found in {0}")]
    MissingBitcodeSection(PathBuf),

    /// LLVM cannot create a module for linking.
    #[error("failed to create module")]
    CreateModuleError,
}

/// BPF Cpu type
#[derive(Clone, Copy, Debug)]
pub enum Cpu {
    Generic,
    Probe,
    V1,
    V2,
    V3,
}

impl Cpu {
    fn as_c_str(&self) -> &'static CStr {
        match self {
            Self::Generic => c"generic",
            Self::Probe => c"probe",
            Self::V1 => c"v1",
            Self::V2 => c"v2",
            Self::V3 => c"v3",
        }
    }
}

impl std::fmt::Display for Cpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(match self {
            Self::Generic => "generic",
            Self::Probe => "probe",
            Self::V1 => "v1",
            Self::V2 => "v2",
            Self::V3 => "v3",
        })
    }
}

impl FromStr for Cpu {
    type Err = LinkerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "generic" => Self::Generic,
            "probe" => Self::Probe,
            "v1" => Self::V1,
            "v2" => Self::V2,
            "v3" => Self::V3,
            _ => return Err(LinkerError::InvalidCpu(s.to_string())),
        })
    }
}

/// Optimization level
#[derive(Clone, Copy, Debug)]
pub enum OptLevel {
    /// No optimizations. Equivalent to -O0.
    No,
    /// Less than the default optimizations. Equivalent to -O1.
    Less,
    /// Default level of optimizations. Equivalent to -O2.
    Default,
    /// Aggressive optimizations. Equivalent to -O3.
    Aggressive,
    /// Optimize for size. Equivalent to -Os.
    Size,
    /// Aggressively optimize for size. Equivalent to -Oz.
    SizeMin,
}

pub enum LinkerInput<'a> {
    File { path: &'a Path },
    Buffer { name: &'a str, bytes: &'a [u8] },
}

impl<'a> LinkerInput<'a> {
    pub fn new_from_file(path: &'a Path) -> Self {
        LinkerInput::File { path }
    }

    pub fn new_from_buffer(name: &'a str, bytes: &'a [u8]) -> Self {
        LinkerInput::Buffer { name, bytes }
    }
}

/// Linker input type
#[derive(Clone, Copy, Debug, PartialEq)]
enum InputType {
    /// LLVM bitcode.
    Bitcode,
    /// ELF object file.
    Elf,
    /// Mach-O object file.
    MachO,
    /// Archive file. (.a)
    Archive,
    /// IR file (.ll)
    Ir,
}

impl std::fmt::Display for InputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Bitcode => "bitcode",
                Self::Elf => "elf",
                Self::MachO => "Mach-O",
                Self::Archive => "archive",
                Self::Ir => "ir",
            }
        )
    }
}

/// Output type
#[derive(Clone, Copy, Debug)]
pub enum OutputType {
    /// LLVM bitcode.
    Bitcode,
    /// Assembly.
    Assembly,
    /// LLVM IR.
    LlvmAssembly,
    /// ELF object file.
    Object,
}

/// Options to configure the linker
#[derive(Debug)]
pub struct LinkerOptions {
    /// The LLVM target to generate code for. If None, the target will be inferred from the input
    /// modules.
    pub target: Option<CString>,
    /// Cpu type.
    pub cpu: Cpu,
    /// Cpu features.
    pub cpu_features: CString,
    /// Optimization level.
    pub optimize: OptLevel,
    /// Whether to aggressively unroll loops. Useful for older kernels that don't support loops.
    pub unroll_loops: bool,
    /// Remove `noinline` attributes from functions. Useful for kernels before 5.8 that don't
    /// support function calls.
    pub ignore_inline_never: bool,
    /// Extra command line args to pass to LLVM.
    pub llvm_args: Vec<CString>,
    /// Disable passing --bpf-expand-memcpy-in-order to LLVM.
    pub disable_expand_memcpy_in_order: bool,
    /// Disable exporting memcpy, memmove, memset, memcmp and bcmp. Exporting
    /// those is commonly needed when LLVM does not manage to expand memory
    /// intrinsics to a sequence of loads and stores.
    pub disable_memory_builtins: bool,
    /// Emit BTF information
    pub btf: bool,
    /// Permit automatic insertion of __bpf_trap calls.
    /// See: https://github.com/llvm/llvm-project/commit/ab391beb11f733b526b86f9df23734a34657d876
    pub allow_bpf_trap: bool,
}

/// BPF Linker
pub struct Linker {
    options: LinkerOptions,
    context: LLVMContext,
    diagnostic_handler: llvm::InstalledDiagnosticHandler<DiagnosticHandler>,
    dump_module: Option<PathBuf>,
}

impl Linker {
    /// Create a new linker instance with the given options.
    pub fn new(options: LinkerOptions) -> Self {
        let (context, diagnostic_handler) = llvm_init(&options);

        Self {
            options,
            context,
            diagnostic_handler,
            dump_module: None,
        }
    }

    /// Set the directory where the linker will dump the linked LLVM IR before and after
    /// optimization, for debugging and inspection purposes.
    ///
    /// When set:
    /// - The directory is created if it does not already exist.
    /// - A "pre-opt.ll" file is written with the IR before optimization.
    /// - A "post-opt.ll" file is written with the IR after optimization.
    pub fn set_dump_module_path(&mut self, path: impl AsRef<Path>) {
        self.dump_module = Some(path.as_ref().to_path_buf())
    }

    /// Link and generate the output code to file.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use std::{collections::HashSet, path::Path, borrow::Cow, ffi::CString};
    /// # use bpf_linker::{Cpu, Linker, LinkerInput, LinkerOptions, OptLevel, OutputType};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let path = Path::new("/path/to/object-or-bitcode");
    /// let bytes: &[u8] = &[]; // An in memory object/bitcode
    /// # let options = LinkerOptions {
    /// #     target: None,
    /// #     cpu: Cpu::Generic,
    /// #     cpu_features: CString::default(),
    /// #     optimize: OptLevel::Default,
    /// #     unroll_loops: false,
    /// #     ignore_inline_never: false,
    /// #     llvm_args: vec![],
    /// #     disable_expand_memcpy_in_order: false,
    /// #     disable_memory_builtins: false,
    /// #     allow_bpf_trap: false,
    /// #     btf: false,
    /// # };
    /// # let linker = Linker::new(options);
    ///
    /// let export_symbols = ["my_sym_1", "my_sym_2"];
    ///
    /// linker.link_to_file(
    ///     [
    ///         LinkerInput::new_from_file(path),
    ///         LinkerInput::new_from_buffer("my buffer", bytes), // In memory buffer needs a name
    ///     ],
    ///     "/path/to/output",
    ///     OutputType::Object,
    ///     export_symbols,
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn link_to_file<'i, 'a, I, P, E>(
        &self,
        inputs: I,
        output: P,
        output_type: OutputType,
        export_symbols: E,
    ) -> Result<(), LinkerError>
    where
        I: IntoIterator<Item = LinkerInput<'i>>,
        E: IntoIterator<Item = &'a str>,
        P: AsRef<Path>,
    {
        let (linked_module, target_machine) = self.link(inputs, export_symbols)?;
        codegen_to_file(
            &linked_module,
            &target_machine,
            output.as_ref(),
            output_type,
        )?;
        Ok(())
    }

    /// Link and generate the output code to an in-memory buffer.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use std::{collections::HashSet, path::Path, borrow::Cow, ffi::CString};
    /// # use bpf_linker::{Cpu, Linker, LinkerInput, LinkerOptions, OptLevel, OutputType};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let path = Path::new("/path/to/object-or-bitcode");
    /// let bytes: &[u8] = &[]; // An in memory object/bitcode
    /// # let options = LinkerOptions {
    /// #     target: None,
    /// #     cpu: Cpu::Generic,
    /// #     cpu_features: CString::default(),
    /// #     optimize: OptLevel::Default,
    /// #     unroll_loops: false,
    /// #     ignore_inline_never: false,
    /// #     llvm_args: vec![],
    /// #     disable_expand_memcpy_in_order: false,
    /// #     disable_memory_builtins: false,
    /// #     allow_bpf_trap: false,
    /// #     btf: false,
    /// # };
    /// # let linker = Linker::new(options);
    ///
    /// let export_symbols = ["my_sym_1", "my_sym_2"];
    ///
    /// let out_buf = linker.link_to_buffer(
    ///     [
    ///         LinkerInput::new_from_file(path),
    ///         LinkerInput::new_from_buffer("my buffer", bytes), // In memory buffer needs a name
    ///     ],
    ///     OutputType::Bitcode,
    ///     export_symbols,
    /// )?;
    ///
    /// // Use the buffer as slice of u8
    /// let bytes = out_buf.as_slice();
    /// println!("Linked {} bytes into memory)", bytes.len());
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn link_to_buffer<'i, 'a, I, E>(
        &self,
        inputs: I,
        output_type: OutputType,
        export_symbols: E,
    ) -> Result<LinkerOutput, LinkerError>
    where
        I: IntoIterator<Item = LinkerInput<'i>>,
        E: IntoIterator<Item = &'a str>,
    {
        let (linked_module, target_machine) = self.link(inputs, export_symbols)?;
        codegen_to_buffer(&linked_module, &target_machine, output_type)
    }

    /// Link and generate the output code.
    fn link<'ctx, 'i, 'a, I, E>(
        &'ctx self,
        inputs: I,
        export_symbols: E,
    ) -> Result<(LLVMModule<'ctx>, LLVMTargetMachine), LinkerError>
    where
        I: IntoIterator<Item = LinkerInput<'i>>,
        E: IntoIterator<Item = &'a str>,
    {
        let Self {
            options,
            context,
            dump_module,
            ..
        } = self;

        let mut module = link_modules(context, inputs)?;

        let target_machine = create_target_machine(options, &module)?;

        if let Some(path) = dump_module {
            fs::create_dir_all(path).map_err(|err| LinkerError::IoError(path.to_owned(), err))?;
        }
        if let Some(path) = dump_module {
            // dump IR before optimization
            let path = path.join("pre-opt.ll");
            let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            module
                .write_ir_to_path(&path)
                .map_err(LinkerError::WriteIRError)?;
        };
        optimize(
            options,
            context,
            &target_machine,
            &mut module,
            export_symbols,
        )?;
        if let Some(path) = dump_module {
            // dump IR before optimization
            let path = path.join("post-opt.ll");
            let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            module
                .write_ir_to_path(&path)
                .map_err(LinkerError::WriteIRError)?;
        };

        Ok((module, target_machine))
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostic_handler.with_view(|h| h.has_errors)
    }
}

fn link_modules<'ctx, 'i, I>(
    context: &'ctx LLVMContext,
    inputs: I,
) -> Result<LLVMModule<'ctx>, LinkerError>
where
    I: IntoIterator<Item = LinkerInput<'i>>,
{
    let mut module = context
        .create_module(c"linked_module")
        .ok_or(LinkerError::CreateModuleError)?;

    let mut buf = Vec::new();
    for input in inputs {
        let data: Vec<u8>;
        let (path, input) = match input {
            LinkerInput::File { path } => {
                data = fs::read(path).map_err(|e| LinkerError::IoError(path.to_owned(), e))?;
                (path.to_owned(), data.as_ref())
            }
            LinkerInput::Buffer { name, bytes } => {
                (PathBuf::from(format!("in_memory::{}", name)), bytes)
            }
        };

        // determine whether the input is bitcode, ELF with embedded bitcode, an archive file
        // or an invalid file
        let in_type =
            detect_input_type(input).ok_or_else(|| LinkerError::InvalidInputType(path.clone()))?;

        match in_type {
            InputType::Archive => {
                info!("linking archive {}", path.display());

                // Extract the archive and call link_reader() for each item.
                let mut archive = Archive::new(input);
                while let Some(item) = archive.next_entry() {
                    let mut item = item.map_err(|e| LinkerError::IoError(path.clone(), e))?;
                    let name = PathBuf::from(OsStr::from_bytes(item.header().identifier()));
                    info!("linking archive item {}", name.display());

                    buf.clear();
                    let _: usize = item
                        .read_to_end(&mut buf)
                        .map_err(|e| LinkerError::IoError(name.to_owned(), e))?;
                    let in_type = match detect_input_type(&buf) {
                        Some(in_type) => in_type,
                        None => {
                            info!("ignoring archive item {}: invalid type", name.display());
                            continue;
                        }
                    };

                    match link_data(context, &mut module, &name, &buf, in_type) {
                        Ok(()) => continue,
                        Err(LinkerError::InvalidInputType(name)) => {
                            info!("ignoring archive item {}: invalid type", name.display());
                            continue;
                        }
                        Err(LinkerError::MissingBitcodeSection(name)) => {
                            warn!(
                                "ignoring archive item {}: no embedded bitcode",
                                name.display()
                            );
                            continue;
                        }
                        // TODO: this discards the underlying error.
                        Err(_) => {
                            return Err(LinkerError::LinkArchiveModuleError(
                                path.to_owned(),
                                name.to_owned(),
                            ));
                        }
                    };
                }
            }
            ty => {
                info!("linking file {} type {}", path.display(), ty);
                match link_data(context, &mut module, &path, input, ty) {
                    Ok(()) => {}
                    Err(LinkerError::InvalidInputType(path)) => {
                        info!("ignoring file {}: invalid type", path.display());
                        continue;
                    }
                    Err(LinkerError::MissingBitcodeSection(path)) => {
                        warn!("ignoring file {}: no embedded bitcode", path.display());
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    Ok(module)
}

fn link_data<'ctx>(
    context: &'ctx LLVMContext,
    module: &mut LLVMModule<'ctx>,
    path: &Path,
    data: &[u8],
    in_type: InputType,
) -> Result<(), LinkerError> {
   if in_type == InputType::Ir {
        let mut ir_data = data.to_vec();
        ir_data.push(0);
        let c_str = CStr::from_bytes_with_nul(&ir_data)
            .expect("we just added the null terminator");
        
        return llvm::link_ir_buffer(context, module, c_str)
            .map_err(|e| LinkerError::IRParseError(path.to_owned(), e))
            .and_then(|linked| {
                if linked {
                    Ok(())
                } else {
                    Err(LinkerError::LinkModuleError(path.to_owned()))
                }
            });
    }
    let bitcode = match in_type {
        InputType::Bitcode => Cow::Borrowed(data),
        InputType::Elf => match llvm::find_embedded_bitcode(context, data) {
            Ok(Some(bitcode)) => Cow::Owned(bitcode),
            Ok(None) => return Err(LinkerError::MissingBitcodeSection(path.to_owned())),
            Err(e) => return Err(LinkerError::EmbeddedBitcodeError(e)),
        },
        // we need to handle this here since archive files could contain
        // mach-o files, eg somecrate.rlib containing lib.rmeta which is
        // mach-o on macos
        InputType::MachO => return Err(LinkerError::InvalidInputType(path.to_owned())),
        // this can't really happen
        InputType::Archive => panic!("nested archives not supported duh"),
        InputType::Ir => unreachable!(), // handled above
    };

    if !llvm::link_bitcode_buffer(context, module, &bitcode) {
        return Err(LinkerError::LinkModuleError(path.to_owned()));
    }

    Ok(())
}

fn create_target_machine(
    options: &LinkerOptions,
    module: &LLVMModule<'_>,
) -> Result<LLVMTargetMachine, LinkerError> {
    let LinkerOptions {
        target,
        cpu,
        cpu_features,
        ..
    } = options;
    // Here's how the output target is selected:
    //
    // 1) rustc with builtin BPF support: cargo build --target=bpf[el|eb]-unknown-none
    //      the input modules are already configured for the correct output target
    //
    // 2) rustc with no BPF support: cargo rustc -- -C linker-flavor=bpf-linker -C linker=bpf-linker -C link-arg=--target=bpf[el|eb]
    //      the input modules are configured for the *host* target, and the output target
    //      is configured with the `--target` linker argument
    //
    // 3) rustc with no BPF support: cargo rustc -- -C linker-flavor=bpf-linker -C linker=bpf-linker
    //      the input modules are configured for the *host* target, the output target isn't
    //      set via `--target`, so default to `bpf` (bpfel or bpfeb depending on the host
    //      endianness)
    let (triple, target) = match target {
        // case 1
        Some(c_triple) => (c_triple.as_c_str(), llvm::target_from_triple(c_triple)),
        None => {
            let c_triple = module.get_target();
            let c_triple = unsafe { CStr::from_ptr(c_triple) };
            if c_triple.to_bytes().starts_with(b"bpf") {
                // case 2
                (c_triple, llvm::target_from_module(module))
            } else {
                // case 3.
                info!(
                    "detected non-bpf input target {} and no explicit output --target specified, selecting `bpf'",
                    OsStr::from_bytes(c_triple.to_bytes()).display()
                );
                let c_triple = c"bpf";
                (c_triple, llvm::target_from_triple(c_triple))
            }
        }
    };
    let target =
        target.map_err(|_msg| LinkerError::InvalidTarget(triple.to_string_lossy().to_string()))?;

    debug!(
        "creating target machine: triple: {} cpu: {} features: {}",
        triple.to_string_lossy(),
        cpu,
        cpu_features.to_string_lossy(),
    );

    let target_machine = LLVMTargetMachine::new(target, triple, cpu.as_c_str(), cpu_features)
        .ok_or_else(|| LinkerError::InvalidTarget(triple.to_string_lossy().to_string()))?;

    Ok(target_machine)
}

fn optimize<'ctx, 'a, E>(
    options: &LinkerOptions,
    context: &'ctx LLVMContext,
    target_machine: &LLVMTargetMachine,
    module: &mut LLVMModule<'ctx>,
    export_symbols: E,
) -> Result<(), LinkerError>
where
    E: IntoIterator<Item = &'a str>,
{
    let LinkerOptions {
        disable_memory_builtins,
        optimize,
        btf,
        ignore_inline_never,
        ..
    } = options;

    let mut export_symbols: HashSet<Cow<'_, [u8]>> = export_symbols
        .into_iter()
        .map(|s| Cow::Borrowed(s.as_bytes()))
        .collect();

    if !disable_memory_builtins {
        export_symbols.extend(
            ["memcpy", "memmove", "memset", "memcmp", "bcmp"]
                .into_iter()
                .map(|s| s.as_bytes().into()),
        );
    };
    debug!(
        "linking exporting symbols {:?}, opt level {:?}",
        export_symbols, optimize
    );
    // run optimizations. Will optionally remove noinline attributes, intern all non exported
    // programs and maps and remove dead code.

    if *btf {
        // if we want to emit BTF, we need to sanitize the debug information
        llvm::DISanitizer::new(context, module).run(&export_symbols);
    } else {
        // if we don't need BTF emission, we can strip DI
        let ok = module.strip_debug_info();
        debug!("Stripping DI, changed={}", ok);
    }

    llvm::optimize(
        target_machine,
        module,
        options.optimize,
        *ignore_inline_never,
        &export_symbols,
    )
    .map_err(LinkerError::OptimizeError)?;

    Ok(())
}

fn codegen_to_file(
    module: &LLVMModule<'_>,
    target_machine: &LLVMTargetMachine,
    output: &Path,
    output_type: OutputType,
) -> Result<(), LinkerError> {
    info!("writing {:?} to {:?}", output_type, output);
    let output = CString::new(output.as_os_str().as_encoded_bytes()).unwrap();
    match output_type {
        OutputType::Bitcode => module
            .write_bitcode_to_path(&output)
            .map_err(LinkerError::WriteBitcodeError),
        OutputType::LlvmAssembly => module
            .write_ir_to_path(&output)
            .map_err(LinkerError::WriteIRError),
        OutputType::Assembly => target_machine
            .emit_to_file(module, &output, LLVMCodeGenFileType::LLVMAssemblyFile)
            .map_err(LinkerError::EmitCodeError),
        OutputType::Object => target_machine
            .emit_to_file(module, &output, LLVMCodeGenFileType::LLVMObjectFile)
            .map_err(LinkerError::EmitCodeError),
    }
}

fn codegen_to_buffer(
    module: &LLVMModule<'_>,
    target_machine: &LLVMTargetMachine,
    output_type: OutputType,
) -> Result<LinkerOutput, LinkerError> {
    let memory_buffer = match output_type {
        OutputType::Bitcode => module.write_bitcode_to_memory(),
        OutputType::LlvmAssembly => module.write_ir_to_memory(),
        OutputType::Assembly => target_machine
            .emit_to_memory_buffer(module, LLVMCodeGenFileType::LLVMAssemblyFile)
            .map_err(LinkerError::EmitCodeError)?,
        OutputType::Object => target_machine
            .emit_to_memory_buffer(module, LLVMCodeGenFileType::LLVMObjectFile)
            .map_err(LinkerError::EmitCodeError)?,
    };

    Ok(LinkerOutput {
        inner: memory_buffer,
    })
}

fn llvm_init(
    options: &LinkerOptions,
) -> (
    LLVMContext,
    llvm::InstalledDiagnosticHandler<DiagnosticHandler>,
) {
    let mut args = Vec::<Cow<'_, CStr>>::new();
    args.push(c"bpf-linker".into());
    // Disable cold call site detection. Many accessors in aya-ebpf return Result<T, E>
    // where the layout is larger than 64 bits, but the LLVM BPF target only supports
    // up to 64 bits return values. Since the accessors are tiny in terms of code, we
    // avoid the issue by annotating them with #[inline(always)]. If they are classified
    // as cold though - and they often are starting from LLVM17 - #[inline(always)]
    // is ignored and the BPF target fails codegen.
    args.push(c"--cold-callsite-rel-freq=0".into());
    if options.unroll_loops {
        // setting cmdline arguments is the only way to customize the unroll pass with the
        // C API.
        args.extend([
            c"--unroll-runtime".into(),
            c"--unroll-runtime-multi-exit".into(),
            CString::new(format!("--unroll-max-upperbound={}", u32::MAX))
                .unwrap()
                .into(),
            CString::new(format!("--unroll-threshold={}", u32::MAX))
                .unwrap()
                .into(),
        ]);
    }
    if !options.disable_expand_memcpy_in_order {
        args.push(c"--bpf-expand-memcpy-in-order".into());
    }
    if !options.allow_bpf_trap {
        // TODO: Remove this once ksyms support is guaranteed.
        // LLVM introduces __bpf_trap calls at points where __builtin_trap would normally be
        // emitted. This is currently not supported by aya because __bpf_trap requires a .ksyms
        // section, but this is not trivial to support. In the meantime, using this flag
        // returns LLVM to the old behaviour, which did not introduce these calls and therefore
        // does not require the .ksyms section.
        args.push(c"--bpf-disable-trap-unreachable".into());
    }
    args.extend(options.llvm_args.iter().map(Into::into));
    info!("LLVM command line: {:?}", args);
    llvm::init(args.as_slice(), c"BPF linker");

    let mut context = LLVMContext::new();

    let diagnostic_handler = context.set_diagnostic_handler(DiagnosticHandler::default());

    unsafe {
        LLVMInstallFatalErrorHandler(Some(llvm::fatal_error));
        LLVMEnablePrettyStackTrace();
    }

    (context, diagnostic_handler)
}

#[derive(Default)]
pub(crate) struct DiagnosticHandler {
    pub(crate) has_errors: bool,
    // The handler is passed to LLVM as a raw pointer so it must not be moved.
    _marker: std::marker::PhantomPinned,
}

impl llvm::LLVMDiagnosticHandler for DiagnosticHandler {
    fn handle_diagnostic(
        &mut self,
        severity: llvm_sys::LLVMDiagnosticSeverity,
        message: Cow<'_, str>,
    ) {
        // TODO(https://reviews.llvm.org/D155894): Remove this when LLVM no longer emits these
        // errors.
        //
        // See https://github.com/rust-lang/compiler-builtins/blob/a61823f/src/mem/mod.rs#L22-L68.
        const MATCHERS: &[&str] = &[
            "A call to built-in function 'memcpy' is not supported.\n",
            "A call to built-in function 'memmove' is not supported.\n",
            "A call to built-in function 'memset' is not supported.\n",
            "A call to built-in function 'memcmp' is not supported.\n",
            "A call to built-in function 'bcmp' is not supported.\n",
            "A call to built-in function 'strlen' is not supported.\n",
        ];

        match severity {
            llvm_sys::LLVMDiagnosticSeverity::LLVMDSError => {
                if MATCHERS.iter().any(|matcher| message.ends_with(matcher)) {
                    return;
                }
                self.has_errors = true;

                error!("llvm: {}", message)
            }
            llvm_sys::LLVMDiagnosticSeverity::LLVMDSWarning => warn!("llvm: {}", message),
            llvm_sys::LLVMDiagnosticSeverity::LLVMDSRemark => debug!("remark: {}", message),
            llvm_sys::LLVMDiagnosticSeverity::LLVMDSNote => debug!("note: {}", message),
        }
    }
}

fn detect_input_type(data: &[u8]) -> Option<InputType> {
    if data.len() < 8 {
        return None;
    }
    match &data[..4] {
        b"\x42\x43\xC0\xDE" | b"\xDE\xC0\x17\x0b" => Some(InputType::Bitcode),
        b"\x7FELF" => Some(InputType::Elf),
        b"\xcf\xfa\xed\xfe" => Some(InputType::MachO),
        _ => {
            if &data[..8] == b"!<arch>\x0A" {
                Some(InputType::Archive)
            } else if is_llvm_ir(data) {
                Some(InputType::Ir)
            } else {
                None
            }
        }
    }
}

fn is_llvm_ir(data: &[u8]) -> bool {
    const PREFIXES: &[&[u8]] = &[
        b"; ModuleID",
        b"source_filename",
        b"target datalayout",
        b"target triple",
        b"define ",
        b"declare ",
        b"!llvm",
    ];

    let trimmed = data.trim_ascii_start();

    PREFIXES.iter().any(|p| trimmed.starts_with(p))
}

pub struct LinkerOutput {
    inner: MemoryBuffer,
}

impl LinkerOutput {
    pub fn as_slice(&self) -> &[u8] {
        self.inner.as_slice()
    }
}

impl AsRef<[u8]> for LinkerOutput {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Deref for LinkerOutput {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
