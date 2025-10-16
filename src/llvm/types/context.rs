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
    context: LLVMContextRef,
    /// Optional diagnostic handler set for the context.
    ///
    /// The diagnostic handler pointer must remain valid until either
    /// a new handler is installed or the context is disposed.
    /// To guarantee this, we keep a strong reference to the handler
    /// inside the wrapper.
    /// The type of the diagnostic handler is erased to make the
    /// context wrapper non generic.
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
    /// The caller must ensure that the [`LLVMContext`] outlives the pointer this
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

    /// Install a context-local diagnostic handler.
    pub(crate) fn set_diagnostic_handler<T>(&mut self, handler: T) -> InstalledDiagnosticHandler<T>
    where
        T: LLVMDiagnosticHandler + 'static,
    {
        // Heap-allocate and pin the handler so its address is stable
        // for the C API
        let pinrc = Rc::pin(handler);

        // Get a opaque raw pointer to the new memory stable object
        let handler_ptr = ptr::from_ref(Pin::as_ref(&pinrc).get_ref()) as *mut c_void;

        unsafe {
            LLVMContextSetDiagnosticHandler(
                self.context,
                Some(diagnostic_handler::<T>),
                handler_ptr,
            )
        };

        // Keep the handler alive for at least as long as the context
        // by storing a type-erased pinned clone in the context. This
        // guards against the handler being dropped while LLVM still
        // holds the callback pointer.
        self.diagnostic_handler = Some(StoredHandler {
            _handler: pinrc.clone(),
        });

        // Return a typed handle that keeps a strong, pinned reference to `T`.
        //
        // This lets the caller interact with the installed diagnostic handler
        // directly (via `with_view`) without needing to query the context or
        // deal with an Option. It also contributes to keeping the handler alive
        // for as long as the handle (or the context-held clone) exists.
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
