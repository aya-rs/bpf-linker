use core::slice;

use llvm_sys::{
    core::{LLVMDisposeMemoryBuffer, LLVMGetBufferSize, LLVMGetBufferStart},
    prelude::LLVMMemoryBufferRef,
};

pub(crate) struct MemoryBuffer {
    memory_buffer: LLVMMemoryBufferRef,
}

impl MemoryBuffer {
    pub(crate) const fn new(memory_buffer: LLVMMemoryBufferRef) -> Self {
        Self { memory_buffer }
    }

    pub(crate) const fn as_mut_ptr(&self) -> LLVMMemoryBufferRef {
        let Self { memory_buffer } = self;
        *memory_buffer
    }

    /// Gets a byte slice of this `MemoryBuffer`.
    pub(crate) fn as_slice(&self) -> &[u8] {
        unsafe {
            let start = LLVMGetBufferStart(self.memory_buffer);

            slice::from_raw_parts(start.cast(), self.get_size())
        }
    }

    /// Gets the byte size of this `MemoryBuffer`.
    pub(crate) fn get_size(&self) -> usize {
        unsafe { LLVMGetBufferSize(self.memory_buffer) }
    }
}

impl Drop for MemoryBuffer {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeMemoryBuffer(self.memory_buffer);
        }
    }
}
