use std::marker::PhantomData;

use llvm_sys::{
    core::LLVMMetadataAsValue,
    debuginfo::{
        LLVMCreateDIBuilder, LLVMDIBuilderCreateFunction, LLVMDIBuilderFinalizeSubprogram,
        LLVMDisposeDIBuilder,
    },
    prelude::{LLVMDIBuilderRef, LLVMMetadataRef},
};

use crate::llvm::{LLVMContext, LLVMModule, types::di::DISubprogram};

pub(crate) struct DIBuilder<'ctx> {
    builder: LLVMDIBuilderRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DIBuilder<'ctx> {
    pub(crate) fn new(module: &'ctx LLVMModule<'ctx>) -> Self {
        let builder = unsafe { LLVMCreateDIBuilder(module.as_mut_ptr()) };
        Self {
            builder,
            _marker: PhantomData,
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub(crate) fn create_function(
        &self,
        context: &'ctx LLVMContext,
        scope: LLVMMetadataRef,
        name: Option<&[u8]>,
        linkage_name: Option<&[u8]>,
        file: LLVMMetadataRef,
        line: u32,
        ty: LLVMMetadataRef,
        is_local_to_unit: bool,
        is_definition: bool,
        scope_line: u32,
        flags: i32,
        is_optimized: bool,
    ) -> DISubprogram<'ctx> {
        let (name, name_len) = name.map_or((std::ptr::null(), 0), |s| (s.as_ptr(), s.len()));
        let (linkage_name, linkage_name_len) =
            linkage_name.map_or((std::ptr::null(), 0), |s| (s.as_ptr(), s.len()));

        let subprogram = unsafe {
            LLVMDIBuilderCreateFunction(
                self.builder,
                scope,
                name.cast(),
                name_len,
                linkage_name.cast(),
                linkage_name_len,
                file,
                line,
                ty,
                i32::from(is_local_to_unit),
                i32::from(is_definition),
                scope_line,
                flags,
                i32::from(is_optimized),
            )
        };

        unsafe {
            DISubprogram::from_value_ref(LLVMMetadataAsValue(context.as_mut_ptr(), subprogram))
        }
    }

    pub(crate) fn finalize_subprogram(&self, subprogram: &DISubprogram<'_>) {
        unsafe {
            LLVMDIBuilderFinalizeSubprogram(
                self.builder,
                llvm_sys::core::LLVMValueAsMetadata(subprogram.value_ref),
            )
        };
    }
}

impl Drop for DIBuilder<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeDIBuilder(self.builder) };
    }
}
