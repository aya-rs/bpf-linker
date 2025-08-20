use std::ffi::CStr;

use llvm_sys::target_machine::{
    LLVMCodeGenFileType, LLVMDisposeTargetMachine, LLVMTargetMachineEmitToFile,
    LLVMTargetMachineRef,
};

use crate::llvm::{LLVMModuleWrapped, Message};

pub struct LLVMTargetMachineWrapped {
    pub(super) target_machine: LLVMTargetMachineRef,
}

impl LLVMTargetMachineWrapped {
    pub unsafe fn codegen(
        &self,
        module: &LLVMModuleWrapped,
        output: &CStr,
        output_type: LLVMCodeGenFileType,
    ) -> Result<(), String> {
        let (ret, message) = Message::with(|message| {
            LLVMTargetMachineEmitToFile(
                self.target_machine,
                module.module,
                output.as_ptr() as *mut _,
                output_type,
                message,
            )
        });
        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
        }
    }
}

impl Drop for LLVMTargetMachineWrapped {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeTargetMachine(self.target_machine);
        }
    }
}
