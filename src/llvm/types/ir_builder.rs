use std::marker::PhantomData;

use llvm_sys::{
    core::{
        LLVMBuildCall2, LLVMCreateBuilderInContext, LLVMDisposeBuilder, LLVMGlobalGetValueType,
        LLVMPositionBuilderBefore,
    },
    prelude::{LLVMBuilderRef, LLVMValueRef},
};

use crate::llvm::{LLVMContext, types::instruction::CallInst};

pub(crate) struct IRBuilder<'ctx> {
    builder: LLVMBuilderRef,
    _marker: PhantomData<&'ctx LLVMContext>,
}

impl<'ctx> IRBuilder<'ctx> {
    pub(crate) fn new(context: &'ctx LLVMContext) -> Self {
        let builder = unsafe { LLVMCreateBuilderInContext(context.as_mut_ptr()) };
        Self {
            builder,
            _marker: PhantomData,
        }
    }

    pub(crate) fn position_before(&self, instruction: LLVMValueRef) {
        unsafe { LLVMPositionBuilderBefore(self.builder, instruction) };
    }

    pub(crate) fn build_call2(
        &self,
        callee: LLVMValueRef,
        args: &mut [LLVMValueRef],
    ) -> CallInst<'ctx> {
        unsafe {
            let value_ref = LLVMBuildCall2(
                self.builder,
                LLVMGlobalGetValueType(callee),
                callee,
                args.as_mut_ptr(),
                args.len().try_into().unwrap(),
                c"".as_ptr(),
            );
            CallInst::from_value_ref(value_ref)
        }
    }
}

impl Drop for IRBuilder<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeBuilder(self.builder) };
    }
}
