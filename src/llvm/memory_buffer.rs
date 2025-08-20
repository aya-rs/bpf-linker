use core::slice;

use llvm_sys::{
    core::{LLVMDisposeMemoryBuffer, LLVMGetBufferSize, LLVMGetBufferStart},
    prelude::LLVMMemoryBufferRef,
};

pub struct MemoryBufferWrapped {
    pub(super) memory_buffer: LLVMMemoryBufferRef,
}

impl MemoryBufferWrapped {
    /// Gets a byte slice of this `MemoryBuffer`.
    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            let start = LLVMGetBufferStart(self.memory_buffer);

            slice::from_raw_parts(start as *const _, self.get_size())
        }
    }

    /// Gets the byte size of this `MemoryBuffer`.
    pub fn get_size(&self) -> usize {
        unsafe { LLVMGetBufferSize(self.memory_buffer) }
    }
}

impl Drop for MemoryBufferWrapped {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeMemoryBuffer(self.memory_buffer);
        }
    }
}
