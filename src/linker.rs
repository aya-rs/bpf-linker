use ar::Archive;
use llvm_sys::bit_writer::LLVMWriteBitcodeToFile;
use llvm_sys::core::*;
use llvm_sys::error_handling::*;
use llvm_sys::prelude::*;
use llvm_sys::target_machine::*;
use log::*;
use std::{
    collections::HashSet,
    ffi::{CStr, CString},
    fs::File,
    io,
    io::Read,
    io::Seek,
    io::SeekFrom,
    path::Path,
    path::PathBuf,
    ptr, str,
    str::FromStr,
};
use thiserror::Error;

use crate::llvm;

#[derive(Debug, Error)]
pub enum LinkerError {
    #[error("invalid CPU {0}")]
    InvalidCpu(String),

    #[error("invalid LLVM target {0}")]
    InvalidTarget(String),

    #[error("`{0}`: {1}")]
    IoError(PathBuf, io::Error),

    #[error("invalid input file `{0}`")]
    InvalidInputType(PathBuf),

    #[error("failure linking module {0}")]
    LinkModuleError(PathBuf),

    #[error("failure linking module {1} from {0}")]
    LinkArchiveModuleError(PathBuf, PathBuf),

    #[error("LLVMTargetMachineEmitToFile failed: {0}")]
    EmitCodeError(String),

    #[error("LLVMWriteBitcodeToFile failed")]
    WriteBitcodeError,

    #[error("LLVMPrintModuleToFile failed: {0}")]
    WriteIRError(String),

    #[error("error reading embedded bitcode: {0}")]
    EmbeddedBitcodeError(String),

    #[error("no bitcode section found in {0}")]
    MissingBitcodeSection(PathBuf),
}

#[derive(Clone, Copy, Debug)]
pub enum Cpu {
    Generic,
    Probe,
    V1,
    V2,
    V3,
}

impl std::fmt::Display for Cpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Cpu::*;
        f.pad(match self {
            Generic => "generic",
            Probe => "probe",
            V1 => "v1",
            V2 => "v2",
            V3 => "v3",
        })
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

#[derive(Clone, Copy, Debug)]
pub enum OptLevel {
    No,         // -O0
    Less,       // -O1
    Default,    // -O2
    Aggressive, // -O3
    Size,       // -Os
    SizeMin,    // -Oz
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InputType {
    Bitcode,
    Elf,
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
                Archive => "archive",
            }
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OutputType {
    Bitcode,
    Assembly,
    LlvmAssembly,
    Object,
}

#[derive(Debug)]
pub struct LinkerOptions {
    pub target: Option<String>,
    pub cpu: Cpu,
    pub cpu_features: String,
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub output_type: OutputType,
    pub libs: Vec<PathBuf>,
    pub optimize: OptLevel,
    pub export_symbols: HashSet<String>,
    pub unroll_loops: bool,
    pub ignore_inline_never: bool,
    pub dump_module: Option<PathBuf>,
    pub llvm_args: Vec<String>,
}

pub struct Linker {
    options: LinkerOptions,
    context: LLVMContextRef,
    module: LLVMModuleRef,
    target_machine: LLVMTargetMachineRef,
}

impl Linker {
    pub fn new(options: LinkerOptions) -> Self {
        Linker {
            options,
            context: ptr::null_mut(),
            module: ptr::null_mut(),
            target_machine: ptr::null_mut(),
        }
    }

    pub fn link(mut self) -> Result<(), LinkerError> {
        self.llvm_init();
        self.link_modules()?;
        self.create_target_machine()?;
        self.optimize()?;
        self.codegen()
    }

    fn link_modules(&mut self) -> Result<(), LinkerError> {
        let mut buf = [0u8; 8];
        for path in self.options.inputs.clone() {
            let mut file = File::open(&path).map_err(|e| LinkerError::IoError(path.clone(), e))?;
            file.read(&mut buf)
                .map_err(|e| LinkerError::IoError(path.clone(), e))?;
            file.seek(SeekFrom::Start(0))
                .map_err(|e| LinkerError::IoError(path.clone(), e))?;

            let in_type =
                input_type(&buf).ok_or_else(|| LinkerError::InvalidInputType(path.clone()));

            match in_type? {
                InputType::Archive => {
                    info!("linking archive {:?}", path);

                    let mut archive = Archive::new(file);
                    while let Some(Ok(item)) = archive.next_entry() {
                        let name =
                            PathBuf::from(str::from_utf8(item.header().identifier()).unwrap());
                        info!("linking archive item {:?}", name);

                        match self.link_reader(&name, item) {
                            Ok(_) => continue,
                            Err(LinkerError::InvalidInputType(_)) => {
                                info!("ignoring archive item {:?}: unknown file type", name);
                                continue;
                            }
                            Err(LinkerError::MissingBitcodeSection(_)) => {
                                warn!("ignoring archive item {:?}: no embedded bitcode", name);
                                continue;
                            }
                            Err(_) => {
                                return Err(LinkerError::LinkArchiveModuleError(path, name))
                            }
                        };
                    }
                }
                ty => {
                    info!("linking file {:?} type {}", path, ty);
                    self.link_reader(&path, file)?;
                }
            }
        }

        if let Some(path) = &self.options.dump_module {
            let path = CString::new(path.as_os_str().to_str().unwrap()).unwrap();
            self.write_ir(&path)?;
        }

        Ok(())
    }

    fn link_reader(&mut self, path: &Path, mut reader: impl Read) -> Result<(), LinkerError> {
        let mut data = Vec::new();
        reader
            .read_to_end(&mut data)
            .map_err(|e| LinkerError::IoError(path.to_owned(), e))?;
        let in_type =
            input_type(&data).ok_or_else(|| LinkerError::InvalidInputType(path.to_owned()))?;

        use InputType::*;
        let bitcode = match in_type {
            Bitcode => data,
            Elf => match unsafe { llvm::find_embedded_bitcode(self.context, &data) } {
                Ok(Some(bitcode)) => bitcode,
                Ok(None) => return Err(LinkerError::MissingBitcodeSection(path.to_owned())),
                Err(e) => return Err(LinkerError::EmbeddedBitcodeError(e)),
            },
            // this can't really happen
            Archive => panic!("nested archives not supported duh"),
        };

        if unsafe { !llvm::link_bitcode_buffer(self.context, self.module, &bitcode) } {
            return Err(LinkerError::LinkModuleError(path.to_owned()));
        }

        Ok(())
    }

    fn create_target_machine(&mut self) -> Result<(), LinkerError> {
        unsafe {
            let (triple, target) = match self.options.target.clone() {
                Some(triple) => (
                    triple.clone(),
                    llvm::target_from_triple(&CString::new(triple).unwrap()),
                ),
                None => {
                    let c_triple = LLVMGetTarget(self.module);
                    let triple = CStr::from_ptr(c_triple).to_string_lossy().to_string();
                    (triple, llvm::target_from_module(self.module))
                }
            };
            let target = target.map_err(|_msg| LinkerError::InvalidTarget(triple.clone()))?;

            debug!(
                "creating target machine: triple: {} cpu: {} features: {}",
                triple,
                self.options.cpu.to_string(),
                self.options.cpu_features
            );

            self.target_machine = llvm::create_target_machine(
                target,
                &triple,
                &self.options.cpu.to_string(),
                &self.options.cpu_features,
            )
            .ok_or(LinkerError::InvalidTarget(triple))?;
        }

        Ok(())
    }

    fn optimize(&mut self) -> Result<(), LinkerError> {
        debug!(
            "linking exporting symbols {:?}, opt level {:?}",
            self.options.export_symbols, self.options.optimize
        );
        unsafe {
            llvm::optimize(
                self.target_machine,
                self.module,
                self.options.optimize,
                self.options.ignore_inline_never,
                &self.options.export_symbols,
            )
        };

        Ok(())
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

        unsafe { llvm::write_ir(self.module, &output) }
            .map_err(LinkerError::WriteIRError)
    }

    fn emit(&mut self, output: &CStr, output_type: LLVMCodeGenFileType) -> Result<(), LinkerError> {
        info!("emitting {:?} to {:?}", output_type, output);

        unsafe { llvm::codegen(self.target_machine, self.module, output, output_type) }
            .map_err(LinkerError::EmitCodeError)
    }

    fn llvm_init(&mut self) {
        unsafe {
            let mut args = vec!["bpf-linker".to_string()];
            if self.options.unroll_loops {
                args.push("--unroll-runtime".to_string());
                args.push("--unroll-runtime-multi-exit".to_string());
                args.push(format!("--unroll-max-upperbound={}", std::u32::MAX));
                args.push(format!("--unroll-threshold={}", std::u32::MAX));
            }
            args.extend_from_slice(&self.options.llvm_args);
            info!("LLVM command line: {:?}", args);
            llvm::init(&args, "BPF linker");

            self.context = LLVMContextCreate();
            LLVMContextSetDiagnosticHandler(
                self.context,
                Some(llvm::diagnostic_handler),
                ptr::null_mut(),
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

fn input_type(data: &[u8]) -> Option<InputType> {
    if data.len() < 8 {
        return None;
    }

    use InputType::*;
    match &data[..4] {
        b"\x42\x43\xC0\xDE" => Some(Bitcode),
        b"\x7FELF" => Some(Elf),
        _ => {
            if &data[..8] == b"!<arch>\x0A" {
                Some(Archive)
            } else {
                None
            }
        }
    }
}
