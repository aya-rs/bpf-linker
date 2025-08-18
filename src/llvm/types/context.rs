use std::{
    ffi::{c_void, CString},
    marker::PhantomData,
};

use llvm_sys::{
    core::{
        LLVMContextCreate, LLVMContextDispose, LLVMContextSetDiagnosticHandler,
        LLVMGetDiagInfoDescription, LLVMGetDiagInfoSeverity, LLVMModuleCreateWithNameInContext,
    },
    prelude::{LLVMContextRef, LLVMDiagnosticInfoRef},
};

use crate::llvm::{types::module::LLVMModule, LLVMDiagnosticHandler, Message};

pub struct LLVMContext {
    pub(super) context: LLVMContextRef,
}

impl LLVMContext {
    pub unsafe fn new() -> Self {
        let context = LLVMContextCreate();
        Self { context }
    }

    pub(in crate::llvm) unsafe fn as_raw(&self) -> LLVMContextRef {
        self.context
    }

    pub unsafe fn create_module<'ctx>(&'ctx self, name: &str) -> Option<LLVMModule<'ctx>> {
        let c_name = CString::new(name).unwrap();
        let module = LLVMModuleCreateWithNameInContext(c_name.as_ptr(), self.context);

        if module.is_null() {
            return None;
        }

        Some(LLVMModule {
            module,
            _marker: PhantomData,
        })
    }

    pub unsafe fn set_diagnostic_handler<T: LLVMDiagnosticHandler>(&self, handler: &mut T) {
        LLVMContextSetDiagnosticHandler(
            self.context,
            Some(diagnostic_handler::<T>),
            handler as *mut _ as _,
        );
    }
}

impl Drop for LLVMContext {
    fn drop(&mut self) {
        unsafe {
            LLVMContextDispose(self.context);
        }
    }
}

extern "C" fn diagnostic_handler<T: LLVMDiagnosticHandler>(
    info: LLVMDiagnosticInfoRef,
    handler: *mut c_void,
) {
    let severity = unsafe { LLVMGetDiagInfoSeverity(info) };
    let message = Message {
        ptr: unsafe { LLVMGetDiagInfoDescription(info) },
    };
    let handler = handler as *mut T;
    unsafe { &mut *handler }
        .handle_diagnostic(severity, message.as_c_str().unwrap().to_str().unwrap());
}
