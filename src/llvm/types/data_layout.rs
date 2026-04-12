use std::marker::PhantomData;

use llvm_sys::{
    prelude::LLVMTypeRef,
    target::{LLVMABISizeOfType, LLVMOffsetOfElement, LLVMTargetDataRef},
};

pub(crate) struct DataLayout<'ctx> {
    data_layout: LLVMTargetDataRef,
    _marker: PhantomData<&'ctx ()>,
}

impl DataLayout<'_> {
    pub(in crate::llvm) unsafe fn from_ref(data_layout: LLVMTargetDataRef) -> Self {
        Self {
            data_layout,
            _marker: PhantomData,
        }
    }

    pub(crate) fn abi_size_of_type(&self, ty: LLVMTypeRef) -> u64 {
        unsafe { LLVMABISizeOfType(self.data_layout, ty) }
    }

    pub(crate) fn offset_of_element(&self, struct_ty: LLVMTypeRef, element: u32) -> u64 {
        unsafe { LLVMOffsetOfElement(self.data_layout, struct_ty, element) }
    }
}
