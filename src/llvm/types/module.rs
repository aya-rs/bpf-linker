use std::{ffi::CString, marker::PhantomData, path::Path};

use libc::c_char;
use llvm_sys::{
    bit_writer::LLVMWriteBitcodeToFile,
    core::{LLVMDisposeModule, LLVMGetTarget, LLVMPrintModuleToFile},
    debuginfo::LLVMStripModuleDebugInfo,
    prelude::LLVMModuleRef,
};

use crate::llvm::{types::context::LLVMContext, Message};

pub(crate) struct LLVMModule<'ctx> {
    pub(super) module: LLVMModuleRef,
    pub(super) _marker: PhantomData<&'ctx LLVMContext>,
}

impl LLVMModule<'_> {
    /// Returns an unsafe mutable pointer to the LLVM module.
    ///
    /// The caller must ensure that the [`LLVMModule`] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMModuleRef {
        self.module
    }

    pub(crate) fn get_target(&self) -> *const c_char {
        unsafe { LLVMGetTarget(self.module) }
    }

    pub(crate) fn write_bitcode_to_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<(), std::io::Error> {
        let path = CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap();

        if unsafe { LLVMWriteBitcodeToFile(self.module, path.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(())
    }

    pub(crate) fn write_ir_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap();

        let (ret, message) = unsafe {
            Message::with(|message| LLVMPrintModuleToFile(self.module, path.as_ptr(), message))
        };

        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_string_lossy().to_string())
        }
    }

    /// strips debug information, returns true if DI got stripped
    pub(crate) fn strip_debug_info(&mut self) -> bool {
        unsafe { LLVMStripModuleDebugInfo(self.module) != 0 }
    }
}

impl Drop for LLVMModule<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.module) };
    }
}
