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

pub struct LLVMModule<'ctx> {
    pub(super) module: LLVMModuleRef,
    pub(super) _marker: PhantomData<&'ctx LLVMContext>,
}

impl<'ctx> LLVMModule<'ctx> {
    /// Returns an unsafe mutable pointer to the LLVM module.
    ///
    /// The caller must ensure that the [LLVMModule] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMModuleRef {
        self.module
    }

    pub fn get_target(&self) -> *const c_char {
        unsafe { LLVMGetTarget(self.module) }
    }

    pub fn write_bitcode_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path_str_ptr = path.as_ref().as_os_str().as_encoded_bytes().as_ptr().cast();

        if unsafe { LLVMWriteBitcodeToFile(self.module, path_str_ptr) } == 1 {
            return Err("failed to write bitcode".to_string());
        }

        Ok(())
    }

    pub fn write_bitcode_to_memory(&self) -> MemoryBuffer {
        let buf = unsafe { llvm_sys::bit_writer::LLVMWriteBitcodeToMemoryBuffer(self.module) };

        MemoryBuffer { memory_buffer: buf }
    }

    pub fn write_ir_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path_str_ptr = path.as_ref().as_os_str().as_encoded_bytes().as_ptr().cast();

        let (ret, message) = unsafe {
            Message::with(|message| LLVMPrintModuleToFile(self.module, path_str_ptr, message))
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
        }
    }

    pub fn write_ir_to_memory(&self) -> MemoryBuffer {
        // NOTE: This function implementation wraps the result into a MemoryBuffer,
        // with a copy.
        //
        // The reason is to uniform the output for LinkerOutput.
        // The cleanest solution is LinkerOutput being a wrapper over an enum, with
        // the last being an LLVMMemoryBuffer or a LLVMMessage.
        unsafe {
            let ptr = LLVMPrintModuleToString(self.module);
            let cstr = CStr::from_ptr(ptr);
            let bytes = cstr.to_bytes();

            let buffer_name = CString::new("mem_buffer").unwrap();

            // Copy bytes into a new LLVMMemoryBuffer so we can safely dispose the message.
            let memory_buffer = LLVMCreateMemoryBufferWithMemoryRangeCopy(
                bytes.as_ptr() as *const ::libc::c_char,
                bytes.len(),
                buffer_name.as_ptr(),
            );
            LLVMDisposeMessage(ptr);

            MemoryBuffer { memory_buffer }
        }
    }

    /// strips debug information, returns true if DI got stripped
    pub fn strip_debug_info(&mut self) -> bool {
        unsafe { LLVMStripModuleDebugInfo(self.module) != 0 }
    }
}

impl<'ctx> Drop for LLVMModule<'ctx> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.module) };
    }
}
