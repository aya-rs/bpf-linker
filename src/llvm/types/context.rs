use std::{
    any::Any,
    ffi::{c_void, CStr},
    marker::PhantomData,
    pin::Pin,
    ptr,
    rc::Rc,
};

use llvm_sys::{
    core::{
        LLVMContextCreate, LLVMContextDispose, LLVMContextSetDiagnosticHandler,
        LLVMModuleCreateWithNameInContext,
    },
    prelude::LLVMContextRef,
};

use crate::llvm::{diagnostic_handler, types::module::LLVMModule, LLVMDiagnosticHandler};

pub(crate) struct LLVMContext {
    pub(super) context: LLVMContextRef,
    diagnostic_handler: Option<StoredHandler>,
}

impl LLVMContext {
    pub(crate) fn new() -> Self {
        let context = unsafe { LLVMContextCreate() };
        Self {
            context,
            diagnostic_handler: None,
        }
    }

    /// Returns an unsafe mutable pointer to the LLVM context.
    ///
    /// The caller must ensure that the [LLVMContext] outlives the pointer this
    /// function returns, or else it will end up dangling.
    pub(in crate::llvm) const fn as_mut_ptr(&self) -> LLVMContextRef {
        self.context
    }

    pub(crate) fn create_module<'ctx>(&'ctx self, name: &CStr) -> Option<LLVMModule<'ctx>> {
        let module = unsafe { LLVMModuleCreateWithNameInContext(name.as_ptr(), self.context) };

        if module.is_null() {
            return None;
        }

        Some(LLVMModule {
            module,
            _marker: PhantomData,
        })
    }

    pub(crate) fn set_diagnostic_handler<T>(&mut self, handler: T) -> InstalledDiagnosticHandler<T>
    where
        T: LLVMDiagnosticHandler + 'static,
    {
        let pinrc = Rc::pin(handler);
        self.diagnostic_handler = Some(StoredHandler {
            _handler: pinrc.clone(),
        });

        let handler_ptr = ptr::from_ref(Pin::as_ref(&pinrc).get_ref()) as *mut c_void;

        unsafe {
            LLVMContextSetDiagnosticHandler(
                self.context,
                Some(diagnostic_handler::<T>),
                handler_ptr,
            )
        };

        InstalledDiagnosticHandler { inner: pinrc }
    }
}

impl Drop for LLVMContext {
    fn drop(&mut self) {
        unsafe {
            LLVMContextDispose(self.context);
        }
    }
}

struct StoredHandler {
    _handler: Pin<Rc<dyn Any>>,
}

#[derive(Clone)]
pub(crate) struct InstalledDiagnosticHandler<T: LLVMDiagnosticHandler> {
    inner: Pin<Rc<T>>,
}

impl<T: LLVMDiagnosticHandler> InstalledDiagnosticHandler<T> {
    pub(crate) fn with_view<R, F: FnOnce(&T) -> R>(&self, f: F) -> R {
        f(Pin::as_ref(&self.inner).get_ref())
    }
}
