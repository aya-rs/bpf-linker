use std::{ffi::CStr, marker::PhantomData};

use llvm_sys::{
    bit_writer::LLVMWriteBitcodeToFile,
    core::{LLVMDisposeModule, LLVMGetTarget, LLVMPrintModuleToFile},
    prelude::LLVMModuleRef,
};

use crate::llvm::Message;

pub struct LLVMModuleWrapped<'ctx> {
    pub(super) module: LLVMModuleRef,
    pub(super) _marker: PhantomData<&'ctx super::LLVMContextWrapped>,
}

impl<'ctx> LLVMModuleWrapped<'ctx> {
    pub unsafe fn get_target(&self) -> *const i8 {
        unsafe { LLVMGetTarget(self.module) }
    }

    pub fn write_bitcode(&self, output: &CStr) -> Result<(), String> {
        if unsafe { LLVMWriteBitcodeToFile(self.module, output.as_ptr()) } == 1 {
            return Err("failed to write bitcode".to_string());
        }

        Ok(())
    }

    pub unsafe fn write_ir(&self, output: &CStr) -> Result<(), String> {
        let (ret, message) =
            Message::with(|message| LLVMPrintModuleToFile(self.module, output.as_ptr(), message));
        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
        }
    }
}

impl<'ctx> Drop for LLVMModuleWrapped<'ctx> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.module) };
    }
}
