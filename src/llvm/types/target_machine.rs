use std::ffi::CStr;

use llvm_sys::target_machine::{
    LLVMCodeGenFileType, LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetMachine,
    LLVMDisposeTargetMachine, LLVMRelocMode, LLVMTargetMachineEmitToFile,
    LLVMTargetMachineEmitToMemoryBuffer, LLVMTargetMachineRef, LLVMTargetRef,
};

use crate::llvm::{types::module::LLVMModule, MemoryBuffer, Message};

pub(crate) struct LLVMTargetMachine {
    target_machine: LLVMTargetMachineRef,
}

impl LLVMTargetMachine {
    pub(crate) fn new(
        target: LLVMTargetRef,
        triple: &CStr,
        cpu: &CStr,
        features: &CStr,
    ) -> Option<Self> {
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
            Some(Self { target_machine: tm })
        }
    }

    /// Returns an unsafe mutable pointer to the LLVM target machine.
    ///
    /// The caller must ensure that the [`LLVMTargetMachine`] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMTargetMachineRef {
        self.target_machine
    }

    pub(crate) fn emit_to_file(
        &self,
        module: &LLVMModule<'_>,
        path: &CStr,
        output_type: LLVMCodeGenFileType,
    ) -> Result<(), String> {
        let (ret, message) = unsafe {
            Message::with(|message| {
                LLVMTargetMachineEmitToFile(
                    self.target_machine,
                    module.module,
                    path.as_ptr(),
                    output_type,
                    message,
                )
            })
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_string_lossy().to_string())
        }
    }

    pub(crate) fn emit_to_memory_buffer(
        &self,
        module: &LLVMModule<'_>,
        output_type: LLVMCodeGenFileType,
    ) -> Result<MemoryBuffer, String> {
        let mut out_buf = std::ptr::null_mut();
        let (ret, message) = Message::with(|message| unsafe {
            LLVMTargetMachineEmitToMemoryBuffer(
                self.target_machine,
                module.module,
                output_type,
                message,
                &mut out_buf,
            )
        });
        if ret != 0 {
            return Err(message.as_string_lossy().to_string());
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
