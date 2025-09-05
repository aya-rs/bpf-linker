use std::{ffi::CString, path::Path};

use llvm_sys::target_machine::{
    LLVMCodeGenFileType, LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetMachine,
    LLVMDisposeTargetMachine, LLVMRelocMode, LLVMTargetMachineEmitToFile,
    LLVMTargetMachineEmitToMemoryBuffer, LLVMTargetMachineRef, LLVMTargetRef,
};

use crate::llvm::{types::module::LLVMModule, MemoryBuffer, Message};

pub struct LLVMTargetMachine {
    pub(super) target_machine: LLVMTargetMachineRef,
}

impl LLVMTargetMachine {
    pub fn new(
        target: LLVMTargetRef,
        triple: &str,
        cpu: &str,
        features: &str,
    ) -> Option<LLVMTargetMachine> {
        let triple = CString::new(triple).unwrap();
        let cpu = CString::new(cpu).unwrap();
        let features = CString::new(features).unwrap();
        let tm = unsafe {
            LLVMCreateTargetMachine(
                target,
                triple.as_ptr(),
                cpu.as_ptr(),
                features.as_ptr(),
                LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
                LLVMRelocMode::LLVMRelocDefault,
                LLVMCodeModel::LLVMCodeModelDefault,
            )
        };
        if tm.is_null() {
            None
        } else {
            Some(LLVMTargetMachine { target_machine: tm })
        }
    }

    /// Returns an unsafe mutable pointer to the LLVM target machine.
    ///
    /// The caller must ensure that the [LLVMTargetMachine] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMTargetMachineRef {
        self.target_machine
    }

    pub fn codegen_to_path(
        &self,
        module: &LLVMModule,
        path: impl AsRef<Path>,
        output_type: LLVMCodeGenFileType,
    ) -> Result<(), String> {
        let path_str_ptr = path.as_ref().as_os_str().as_encoded_bytes().as_ptr().cast();

        let (ret, message) = unsafe {
            Message::with(|message| {
                LLVMTargetMachineEmitToFile(
                    self.target_machine,
                    module.module,
                    path_str_ptr,
                    output_type,
                    message,
                )
            })
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
        }
    }

    pub fn codegen_to_mem(
        &self,
        module: &LLVMModule,
        output_type: LLVMCodeGenFileType,
    ) -> Result<MemoryBuffer, String> {
        let mut out_buf = std::ptr::null_mut();
        let (ret, message) = unsafe {
            Message::with(|message| {
                LLVMTargetMachineEmitToMemoryBuffer(
                    self.target_machine,
                    module.module,
                    output_type,
                    message,
                    &mut out_buf,
                )
            })
        };
        if ret != 0 {
            return Err(message.as_c_str().unwrap().to_str().unwrap().to_string());
        }

        Ok(MemoryBuffer {
            memory_buffer: out_buf,
        })
    }
}

impl Drop for LLVMTargetMachine {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeTargetMachine(self.target_machine);
        }
    }
}
