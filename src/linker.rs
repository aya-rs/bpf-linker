use ar::Archive;
use llvm_sys::bit_writer::LLVMWriteBitcodeToFile;
use llvm_sys::core::*;
use llvm_sys::error_handling::*;
use llvm_sys::prelude::*;
use llvm_sys::target_machine::*;
use log::*;
use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{CStr, CString},
    fs::File,
    io,
    io::Read,
    io::Seek,
    os::unix::ffi::OsStrExt as _,
    path::Path,
    path::PathBuf,
    ptr, str,
    str::FromStr,
};
use thiserror::Error;

use crate::llvm;

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
    #[error("LLVMWriteBitcodeToFile failed")]
    WriteBitcodeError,

    /// Writing the LLVM IR failed.
    #[error("LLVMPrintModuleToFile failed: {0}")]
    WriteIRError(String),

    /// There was an error extracting the bitcode embedded in an object file.
    #[error("error reading embedded bitcode: {0}")]
    EmbeddedBitcodeError(String),

    /// The input object file does not have embedded bitcode.
    #[error("no bitcode section found in {0}")]
    MissingBitcodeSection(PathBuf),
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
    fn to_str(self) -> &'static str {
        use Cpu::*;
        match self {
            Generic => "generic",
            Probe => "probe",
            V1 => "v1",
            V2 => "v2",
            V3 => "v3",
        }
    }
}

impl std::fmt::Display for Cpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.to_str())
    }
}

impl FromStr for Cpu {
    type Err = LinkerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use Cpu::*;
        Ok(match s {
            "generic" => Generic,
            "probe" => Probe,
            "v1" => V1,
            "v2" => V2,
            "v3" => V3,
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
}

impl std::fmt::Display for InputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use InputType::*;
        write!(
            f,
            "{}",
            match self {
                Bitcode => "bitcode",
                Elf => "elf",
                MachO => "Mach-O",
                Archive => "archive",
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
    pub target: Option<String>,
    /// Cpu type.
    pub cpu: Cpu,
    /// Cpu features.
    pub cpu_features: String,
    /// Input files. Can be bitcode, object files with embedded bitcode or archive files.
    pub inputs: Vec<PathBuf>,
    /// Where to save the output.
    pub output: PathBuf,
    /// The format to output.
    pub output_type: OutputType,
    pub libs: Vec<PathBuf>,
    /// Optimization level.
    pub optimize: OptLevel,
    /// Set of symbol names to export.
    pub export_symbols: HashSet<Cow<'static, str>>,
    /// Whether to aggressively unroll loops. Useful for older kernels that don't support loops.
    pub unroll_loops: bool,
    /// Remove `noinline` attributes from functions. Useful for kernels before 5.8 that don't
    /// support function calls.
    pub ignore_inline_never: bool,
    /// Write the linked module IR before and after optimization.
    pub dump_module: Option<PathBuf>,
    /// Extra command line args to pass to LLVM.
    pub llvm_args: Vec<String>,
    /// Disable passing --bpf-expand-memcpy-in-order to LLVM.
    pub disable_expand_memcpy_in_order: bool,
    /// Disable exporting memcpy, memmove, memset, memcmp and bcmp. Exporting
    /// those is commonly needed when LLVM does not manage to expand memory
    /// intrinsics to a sequence of loads and stores.
    pub disable_memory_builtins: bool,
}

/// BPF Linker
pub struct Linker {
    options: LinkerOptions,
    context: LLVMContextRef,
    module: LLVMModuleRef,
    target_machine: LLVMTargetMachineRef,
    has_errors: bool,
}

impl Linker {
    /// Create a new linker instance with the given options.
    pub fn new(options: LinkerOptions) -> Self {
        Linker {
            options,
            context: ptr::null_mut(),
            module: ptr::null_mut(),
            target_machine: ptr::null_mut(),
            has_errors: false,
        }
    }

    /// Link and generate the output code.
    pub fn link(&mut self) -> Result<(), LinkerError> {
        self.llvm_init();
        self.link_modules()?;
        self.create_target_machine()?;
        if let Some(path) = &self.options.dump_module {
            std::fs::create_dir_all(path).map_err(|err| LinkerError::IoError(path.clone(), err))?;
        }
        if let Some(path) = &self.options.dump_module {
            // dump IR before optimization
            let path = path.join("pre-opt.ll");
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            self.write_ir(&path)?;
        };
        self.optimize()?;
        if let Some(path) = &self.options.dump_module {
            // dump IR before optimization
            let path = path.join("post-opt.ll");
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            self.write_ir(&path)?;
        };
        self.codegen()?;
        Ok(())
    }

    pub fn has_errors(&self) -> bool {
        self.has_errors
    }

    fn link_modules(&mut self) -> Result<(), LinkerError> {
        // buffer used to perform file type detection
        let mut buf = [0u8; 8];
        for path in self.options.inputs.clone() {
            let mut file = File::open(&path).map_err(|e| LinkerError::IoError(path.clone(), e))?;

            // determine whether the input is bitcode, ELF with embedded bitcode, an archive file
            // or an invalid file
            file.read_exact(&mut buf)
                .map_err(|e| LinkerError::IoError(path.clone(), e))?;
            file.rewind()
                .map_err(|e| LinkerError::IoError(path.clone(), e))?;
            let in_type = detect_input_type(&buf)
                .ok_or_else(|| LinkerError::InvalidInputType(path.clone()))?;

            match in_type {
                InputType::Archive => {
                    info!("linking archive {:?}", path);

                    // Extract the archive and call link_reader() for each item.
                    let mut archive = Archive::new(file);
                    while let Some(Ok(item)) = archive.next_entry() {
                        let name =
                            PathBuf::from(str::from_utf8(item.header().identifier()).unwrap());
                        info!("linking archive item {:?}", name);

                        match self.link_reader(&name, item, None) {
                            Ok(_) => continue,
                            Err(LinkerError::InvalidInputType(_)) => {
                                info!("ignoring archive item {:?}: invalid type", name);
                                continue;
                            }
                            Err(LinkerError::MissingBitcodeSection(_)) => {
                                warn!("ignoring archive item {:?}: no embedded bitcode", name);
                                continue;
                            }
                            Err(_) => return Err(LinkerError::LinkArchiveModuleError(path, name)),
                        };
                    }
                }
                ty => {
                    info!("linking file {:?} type {}", path, ty);
                    match self.link_reader(&path, file, Some(ty)) {
                        Ok(_) => {}
                        Err(LinkerError::InvalidInputType(_)) => {
                            info!("ignoring file {:?}: invalid type", path);
                            continue;
                        }
                        Err(LinkerError::MissingBitcodeSection(_)) => {
                            warn!("ignoring file {:?}: no embedded bitcode", path);
                        }
                        err => return err,
                    }
                }
            }
        }

        Ok(())
    }

    // link in a `Read`-er, which can be a file or an archive item
    fn link_reader(
        &mut self,
        path: &Path,
        mut reader: impl Read,
        in_type: Option<InputType>,
    ) -> Result<(), LinkerError> {
        let mut data = Vec::new();
        let _: usize = reader
            .read_to_end(&mut data)
            .map_err(|e| LinkerError::IoError(path.to_owned(), e))?;
        // in_type is unknown when we're linking an item from an archive file
        let in_type = in_type
            .or_else(|| detect_input_type(&data))
            .ok_or_else(|| LinkerError::InvalidInputType(path.to_owned()))?;

        use InputType::*;
        let bitcode = match in_type {
            Bitcode => data,
            Elf => match unsafe { llvm::find_embedded_bitcode(self.context, &data) } {
                Ok(Some(bitcode)) => bitcode,
                Ok(None) => return Err(LinkerError::MissingBitcodeSection(path.to_owned())),
                Err(e) => return Err(LinkerError::EmbeddedBitcodeError(e)),
            },
            // we need to handle this here since archive files could contain
            // mach-o files, eg somecrate.rlib containing lib.rmeta which is
            // mach-o on macos
            InputType::MachO => return Err(LinkerError::InvalidInputType(path.to_owned())),
            // this can't really happen
            Archive => panic!("nested archives not supported duh"),
        };

        if unsafe { !llvm::link_bitcode_buffer(self.context, self.module, &bitcode) } {
            return Err(LinkerError::LinkModuleError(path.to_owned()));
        }

        Ok(())
    }

    fn create_target_machine(&mut self) -> Result<(), LinkerError> {
        let Self {
            options:
                LinkerOptions {
                    target,
                    cpu,
                    cpu_features,
                    ..
                },
            module,
            target_machine,
            ..
        } = self;
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
            Some(triple) => {
                let c_triple = CString::new(triple.as_str()).unwrap();
                (triple.as_str(), unsafe {
                    llvm::target_from_triple(&c_triple)
                })
            }
            None => {
                let c_triple = unsafe { LLVMGetTarget(*module) };
                let triple = unsafe { CStr::from_ptr(c_triple) }.to_str().unwrap();
                if triple.starts_with("bpf") {
                    // case 2
                    (triple, unsafe { llvm::target_from_module(*module) })
                } else {
                    // case 3.
                    info!("detected non-bpf input target {} and no explicit output --target specified, selecting `bpf'", triple);
                    let triple = "bpf";
                    let c_triple = CString::new(triple).unwrap();
                    (triple, unsafe { llvm::target_from_triple(&c_triple) })
                }
            }
        };
        let target = target.map_err(|_msg| LinkerError::InvalidTarget(triple.to_owned()))?;

        debug!(
            "creating target machine: triple: {} cpu: {} features: {}",
            triple, cpu, cpu_features,
        );

        *target_machine =
            unsafe { llvm::create_target_machine(target, triple, cpu.to_str(), cpu_features) }
                .ok_or_else(|| LinkerError::InvalidTarget(triple.to_owned()))?;

        Ok(())
    }

    fn optimize(&mut self) -> Result<(), LinkerError> {
        if !self.options.disable_memory_builtins {
            self.options.export_symbols.extend(
                ["memcpy", "memmove", "memset", "memcmp", "bcmp"]
                    .into_iter()
                    .map(Into::into),
            );
        };
        debug!(
            "linking exporting symbols {:?}, opt level {:?}",
            self.options.export_symbols, self.options.optimize
        );
        // run optimizations. Will optionally remove noinline attributes, intern all non exported
        // programs and maps and remove dead code.

        unsafe {
            llvm::DIFix::new(self.context, self.module).run();

            llvm::optimize(
                self.target_machine,
                self.module,
                self.options.optimize,
                self.options.ignore_inline_never,
                &self.options.export_symbols,
            )
        }
        .map_err(LinkerError::OptimizeError)
    }

    fn codegen(&mut self) -> Result<(), LinkerError> {
        let output = CString::new(self.options.output.as_os_str().to_str().unwrap()).unwrap();
        match self.options.output_type {
            OutputType::Bitcode => self.write_bitcode(&output),
            OutputType::LlvmAssembly => self.write_ir(&output),
            OutputType::Assembly => self.emit(&output, LLVMCodeGenFileType::LLVMAssemblyFile),
            OutputType::Object => self.emit(&output, LLVMCodeGenFileType::LLVMObjectFile),
        }
    }

    fn write_bitcode(&mut self, output: &CStr) -> Result<(), LinkerError> {
        info!("writing bitcode to {:?}", output);

        if unsafe { LLVMWriteBitcodeToFile(self.module, output.as_ptr()) } == 1 {
            return Err(LinkerError::WriteBitcodeError);
        }

        Ok(())
    }

    fn write_ir(&mut self, output: &CStr) -> Result<(), LinkerError> {
        info!("writing IR to {:?}", output);

        unsafe { llvm::write_ir(self.module, output) }.map_err(LinkerError::WriteIRError)
    }

    fn emit(&mut self, output: &CStr, output_type: LLVMCodeGenFileType) -> Result<(), LinkerError> {
        info!("emitting {:?} to {:?}", output_type, output);

        unsafe { llvm::codegen(self.target_machine, self.module, output, output_type) }
            .map_err(LinkerError::EmitCodeError)
    }

    fn llvm_init(&mut self) {
        let mut args = Vec::<Cow<str>>::new();
        args.push("bpf-linker".into());
        // Disable cold call site detection. Many accessors in aya-ebpf return Result<T, E>
        // where the layout is larger than 64 bits, but the LLVM BPF target only supports
        // up to 64 bits return values. Since the accessors are tiny in terms of code, we
        // avoid the issue by annotating them with #[inline(always)]. If they are classified
        // as cold though - and they often are starting from LLVM17 - #[inline(always)]
        // is ignored and the BPF target fails codegen.
        args.push("--cold-callsite-rel-freq=0".into());
        if self.options.unroll_loops {
            // setting cmdline arguments is the only way to customize the unroll pass with the
            // C API.
            args.extend([
                "--unroll-runtime".into(),
                "--unroll-runtime-multi-exit".into(),
                format!("--unroll-max-upperbound={}", std::u32::MAX).into(),
                format!("--unroll-threshold={}", std::u32::MAX).into(),
            ]);
        }
        if !self.options.disable_expand_memcpy_in_order {
            args.push("--bpf-expand-memcpy-in-order".into());
        }
        args.extend(self.options.llvm_args.iter().map(Into::into));
        info!("LLVM command line: {:?}", args);
        unsafe {
            llvm::init(&args, "BPF linker");

            self.context = LLVMContextCreate();
            LLVMContextSetDiagnosticHandler(
                self.context,
                Some(llvm::diagnostic_handler::<Self>),
                self as *mut _ as _,
            );
            LLVMInstallFatalErrorHandler(Some(llvm::fatal_error));
            LLVMEnablePrettyStackTrace();
            self.module = llvm::create_module(
                self.options.output.file_stem().unwrap().to_str().unwrap(),
                self.context,
            )
            .unwrap();
        }
    }
}

impl llvm::LLVMDiagnosticHandler for Linker {
    fn handle_diagnostic(&mut self, severity: llvm_sys::LLVMDiagnosticSeverity, message: &str) {
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

impl Drop for Linker {
    fn drop(&mut self) {
        unsafe {
            if !self.target_machine.is_null() {
                LLVMDisposeTargetMachine(self.target_machine);
            }
            if !self.module.is_null() {
                LLVMDisposeModule(self.module);
            }
            if !self.context.is_null() {
                LLVMContextDispose(self.context);
            }
        }
    }
}

fn detect_input_type(data: &[u8]) -> Option<InputType> {
    if data.len() < 8 {
        return None;
    }

    use InputType::*;
    match &data[..4] {
        b"\x42\x43\xC0\xDE" | b"\xDE\xC0\x17\x0b" => Some(Bitcode),
        b"\x7FELF" => Some(Elf),
        b"\xcf\xfa\xed\xfe" => Some(MachO),
        _ => {
            if &data[..8] == b"!<arch>\x0A" {
                Some(Archive)
            } else {
                None
            }
        }
    }
}
