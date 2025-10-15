use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
    path::Path,
};

use libc::c_char;
use llvm_sys::{
    bit_writer::LLVMWriteBitcodeToFile,
    core::{
        LLVMCreateMemoryBufferWithMemoryRangeCopy, LLVMDisposeMessage, LLVMDisposeModule,
        LLVMGetTarget, LLVMPrintModuleToFile, LLVMPrintModuleToString,
    },
    debuginfo::LLVMStripModuleDebugInfo,
    prelude::LLVMModuleRef,
};

use crate::llvm::{types::context::LLVMContext, MemoryBuffer, Message};

pub(crate) struct LLVMModule<'ctx> {
    pub(super) module: LLVMModuleRef,
    pub(super) _marker: PhantomData<&'ctx LLVMContext>,
}

impl LLVMModule<'_> {
    /// Returns an unsafe mutable pointer to the LLVM module.
    ///
    /// The caller must ensure that the [LLVMModule] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMModuleRef {
        self.module
    }

    pub(crate) fn get_target(&self) -> *const c_char {
        unsafe { LLVMGetTarget(self.module) }
    }

    pub(crate) fn write_bitcode_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap();

        if unsafe { LLVMWriteBitcodeToFile(self.module, path.as_ptr()) } == 1 {
            return Err("failed to write bitcode".to_string());
        }

        Ok(())
    }

    pub(crate) fn write_bitcode_to_memory(&self) -> MemoryBuffer {
        let buf = unsafe { llvm_sys::bit_writer::LLVMWriteBitcodeToMemoryBuffer(self.module) };

        MemoryBuffer { memory_buffer: buf }
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

    pub(crate) fn write_ir_to_memory(&self) -> MemoryBuffer {
        // Format the module to a string, then copy into a MemoryBuffer. We do the extra copy to keep the
        // internal API simpler, as all the other codegen methods output a MemoryBuffer.
        unsafe {
            let ptr = LLVMPrintModuleToString(self.module);
            let cstr = CStr::from_ptr(ptr);
            let bytes = cstr.to_bytes();

            let buffer_name = c"mem_buffer";

            // Copy bytes into a new LLVMMemoryBuffer so we can safely dispose the message.
            let memory_buffer = LLVMCreateMemoryBufferWithMemoryRangeCopy(
                bytes.as_ptr().cast(),
                bytes.len(),
                buffer_name.as_ptr(),
            );
            LLVMDisposeMessage(ptr);

            MemoryBuffer { memory_buffer }
        }
    }

    /// strips debug information, returns true if DI got stripped
    pub(crate) fn strip_debug_info(&mut self) -> bool {
        unsafe { LLVMStripModuleDebugInfo(self.module) != 0 }
    }
}

impl Drop for LLVMModule<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.module) };
    }
}
