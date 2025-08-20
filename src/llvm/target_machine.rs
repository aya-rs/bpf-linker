use std::ffi::CStr;

use llvm_sys::target_machine::{
    LLVMCodeGenFileType, LLVMDisposeTargetMachine, LLVMTargetMachineEmitToFile,
    LLVMTargetMachineEmitToMemoryBuffer, LLVMTargetMachineRef,
};

use crate::llvm::{LLVMModuleWrapped, MemoryBufferWrapped, Message};

pub struct LLVMTargetMachineWrapped {
    pub(super) target_machine: LLVMTargetMachineRef,
}

impl LLVMTargetMachineWrapped {
    pub unsafe fn codegen_to_file(
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

    pub unsafe fn codegen_to_mem(
        &self,
        module: &LLVMModuleWrapped,
        output_type: LLVMCodeGenFileType,
    ) -> Result<MemoryBufferWrapped, String> {
        let mut out_buf = std::ptr::null_mut();
        let (ret, message) = Message::with(|message| {
            LLVMTargetMachineEmitToMemoryBuffer(
                self.target_machine,
                module.module,
                output_type,
                message,
                &mut out_buf,
            )
        });
        if ret != 0 {
            return Err(message.as_c_str().unwrap().to_str().unwrap().to_string());
        }

        Ok(MemoryBufferWrapped {
            memory_buffer: out_buf,
        })
    }
}

impl Drop for LLVMTargetMachineWrapped {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeTargetMachine(self.target_machine);
        }
    }
}
