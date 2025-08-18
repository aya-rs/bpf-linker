use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
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
    pub(in crate::llvm) unsafe fn as_raw(&self) -> LLVMModuleRef {
        self.module
    }

    pub unsafe fn get_target(&self) -> *const c_char {
        unsafe { LLVMGetTarget(self.module) }
    }

    pub fn write_bitcode_to_file(&self, output: &CStr) -> Result<(), String> {
        if unsafe { LLVMWriteBitcodeToFile(self.module, output.as_ptr()) } == 1 {
            return Err("failed to write bitcode".to_string());
        }

        Ok(())
    }

    pub unsafe fn write_bitcode_to_memory(&self) -> MemoryBuffer {
        let buf = llvm_sys::bit_writer::LLVMWriteBitcodeToMemoryBuffer(self.module);

        MemoryBuffer { memory_buffer: buf }
    }

    pub unsafe fn write_ir_to_file(&self, output: &CStr) -> Result<(), String> {
        let (ret, message) =
            Message::with(|message| LLVMPrintModuleToFile(self.module, output.as_ptr(), message));
        if ret == 0 {
            Ok(())
        } else {
            Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
        }
    }

    pub unsafe fn write_ir_to_memory(&self) -> MemoryBuffer {
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

    /// strips debug information, returns true if DI got stripped
    pub unsafe fn strip_debug_info(&mut self) -> bool {
        LLVMStripModuleDebugInfo(self.module) != 0
    }
}

impl<'ctx> Drop for LLVMModule<'ctx> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.module) };
    }
}
