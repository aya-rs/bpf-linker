use std::{
    ffi::{CStr, CString},
    path::Path,
};

use llvm_sys::target_machine::{
    LLVMCodeGenFileType, LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetMachine,
    LLVMDisposeTargetMachine, LLVMRelocMode, LLVMTargetMachineEmitToFile, LLVMTargetMachineRef,
    LLVMTargetRef,
};

use crate::llvm::{types::module::LLVMModule, Message};

pub(crate) struct LLVMTargetMachine {
    pub(super) target_machine: LLVMTargetMachineRef,
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
    /// The caller must ensure that the [LLVMTargetMachine] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMTargetMachineRef {
        self.target_machine
    }

    pub(crate) fn emit_to_file(
        &self,
        module: &LLVMModule<'_>,
        path: impl AsRef<Path>,
        output_type: LLVMCodeGenFileType,
    ) -> Result<(), String> {
        let path = CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap();

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
}

impl Drop for LLVMTargetMachine {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeTargetMachine(self.target_machine);
        }
    }
}
