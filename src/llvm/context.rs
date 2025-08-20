use std::{ffi::CString, marker::PhantomData};

use llvm_sys::{
    core::{LLVMContextCreate, LLVMContextDispose, LLVMModuleCreateWithNameInContext},
    prelude::LLVMContextRef,
};

use crate::llvm::LLVMModuleWrapped;

pub struct LLVMContextWrapped {
    pub(super) context: LLVMContextRef,
}

impl LLVMContextWrapped {
    pub unsafe fn new() -> Self {
        let context = LLVMContextCreate();
        Self { context }
    }

    pub unsafe fn create_module<'ctx>(&'ctx self, name: &str) -> Option<LLVMModuleWrapped<'ctx>> {
        let c_name = CString::new(name).unwrap();
        let module = LLVMModuleCreateWithNameInContext(c_name.as_ptr(), self.context);

        if module.is_null() {
            return None;
        }

        Some(LLVMModuleWrapped {
            module,
            _marker: PhantomData,
        })
    }
}

impl Drop for LLVMContextWrapped {
    fn drop(&mut self) {
        unsafe {
            LLVMContextDispose(self.context);
        }
    }
}
