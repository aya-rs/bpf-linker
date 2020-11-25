use ar::Archive;
use llvm_sys::bit_writer::LLVMWriteBitcodeToFile;
use llvm_sys::core::*;
use llvm_sys::error_handling::*;
use llvm_sys::prelude::*;
use llvm_sys::target_machine::*;
use log::{debug, info};
use std::{
    collections::HashSet,
    ffi::{CStr, CString},
    io,
    path::PathBuf,
    ptr, str,
    str::FromStr,
};
use std::{
    fs::{self, File},
    io::Read,
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

    #[error("failure linking module {0}")]
    LinkModuleError(PathBuf),

    #[error("failure linking module {1} from {0}")]
    LinkRlibModuleError(PathBuf, PathBuf),

    #[error("LLVMTargetMachineEmitToFile failed: {0}")]
    EmitCodeError(String),

    #[error("LLVMWriteBitcodeToFile failed")]
    WriteBitcodeError,

    #[error("LLVMPrintModuleToFile failed: {0}")]
    WriteIRError(String),
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

#[derive(Clone, Copy, Debug)]
pub enum OutputType {
    Bitcode,
    Assembly,
    LlvmAssembly,
    Object,
}

#[derive(Debug)]
pub struct LinkerOptions {
    pub cpu: Cpu,
    pub cpu_features: String,
    pub bitcode: Vec<PathBuf>,
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
        for module in &self.options.bitcode {
            info!("linking module {:?}", module);
            let data = fs::read(module).map_err(|e| LinkerError::IoError(module.clone(), e))?;
            if unsafe { !llvm::link_bitcode_buffer(self.context, self.module, data) } {
                return Err(LinkerError::LinkModuleError(module.clone()));
            }
        }

        for rlib in &self.options.rlib {
            info!("linking rlib {:?}", rlib);
            let archive_reader =
                File::open(rlib).map_err(|e| LinkerError::IoError(rlib.clone(), e))?;
            let mut archive = Archive::new(archive_reader);

            while let Some(Ok(mut item)) = archive.next_entry() {
                let name = PathBuf::from(str::from_utf8(item.header().identifier()).unwrap());
                info!("linking rlib module {:?}", name);

                if name.extension().unwrap() == "o" {
                    let mut bitcode_bytes = vec![];
                    item.read_to_end(&mut bitcode_bytes)
                        .map_err(|e| LinkerError::IoError(rlib.clone(), e))?;
                    if unsafe {
                        !llvm::link_bitcode_buffer(self.context, self.module, bitcode_bytes)
                    } {
                        return Err(LinkerError::LinkRlibModuleError(rlib.clone(), name));
                    }
                }
            }
        }

        if let Some(path) = &self.options.dump_module {
            let path = CString::new(path.as_os_str().to_str().unwrap()).unwrap();
            self.write_ir(&path)?;
        }

        Ok(())
    }

    fn create_target_machine(&mut self) -> Result<(), LinkerError> {
        unsafe {
            let c_triple = LLVMGetTarget(self.module);
            let triple = CStr::from_ptr(c_triple).to_string_lossy().to_string();
            let target = llvm::target_from_module(self.module)
                .map_err(|_msg| LinkerError::InvalidTarget(triple.clone()))?;

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
            "linking exporting symbols {:?}",
            self.options.export_symbols
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
            .map_err(|msg| LinkerError::WriteIRError(msg))
    }

    fn emit(&mut self, output: &CStr, output_type: LLVMCodeGenFileType) -> Result<(), LinkerError> {
        info!("emitting {:?} to {:?}", output_type, output);

        unsafe { llvm::codegen(self.target_machine, self.module, output, output_type) }
            .map_err(|msg| LinkerError::EmitCodeError(msg))
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
