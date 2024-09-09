use std::{
    cell::RefCell,
    ffi::{CStr, CString, NulError},
    marker::PhantomData,
    ptr::NonNull,
};

use llvm_sys::{
    core::{
        LLVMContextCreate, LLVMContextDispose, LLVMContextSetDiagnosticHandler, LLVMCountParams,
        LLVMDisposeModule, LLVMDisposeValueMetadataEntries, LLVMGetModuleInlineAsm,
        LLVMGetNumOperands, LLVMGetOperand, LLVMGetParam, LLVMGetTarget, LLVMGlobalCopyAllMetadata,
        LLVMIsAFunction, LLVMIsAGlobalObject, LLVMIsAInstruction, LLVMIsAMDNode, LLVMIsAUser,
        LLVMMDNodeInContext2, LLVMMDStringInContext2, LLVMMetadataAsValue,
        LLVMModuleCreateWithNameInContext, LLVMPrintValueToString, LLVMReplaceMDNodeOperandWith,
        LLVMSetModuleInlineAsm2, LLVMValueAsMetadata, LLVMValueMetadataEntriesGetKind,
        LLVMValueMetadataEntriesGetMetadata,
    },
    debuginfo::{
        LLVMCreateDIBuilder, LLVMGetMetadataKind, LLVMGetSubprogram, LLVMMetadataKind,
        LLVMSetSubprogram, LLVMStripModuleDebugInfo,
    },
    prelude::{
        LLVMBasicBlockRef, LLVMContextRef, LLVMMetadataRef, LLVMModuleRef, LLVMValueMetadataEntry,
        LLVMValueRef,
    },
};

use crate::{
    llvm::{
        diagnostic_handler, symbol_name,
        types::{
            di::{DIBuilder, DICompositeType, DIDerivedType, DISubprogram, DIType},
            target::Target,
            LLVMTypeWrapper,
        },
        LLVMDiagnosticHandler, LLVMError, Message,
    },
    DiagnosticHandler,
};

pub(crate) fn replace_name(
    value_ref: LLVMValueRef,
    context: &Context,
    name_operand_index: u32,
    name: &str,
) -> Result<(), NulError> {
    let cstr = CString::new(name)?;
    let name = unsafe { LLVMMDStringInContext2(context.as_ptr(), cstr.as_ptr(), name.len()) };
    unsafe { LLVMReplaceMDNodeOperandWith(value_ref, name_operand_index, name) };
    Ok(())
}

pub struct Context {
    context_ref: LLVMContextRef,
}

impl Drop for Context {
    fn drop(&mut self) {
        tracing::debug!("dropping context");
        unsafe { LLVMContextDispose(self.context_ref) }
    }
}

impl LLVMTypeWrapper for Context {
    type Target = LLVMContextRef;

    unsafe fn from_ptr(context_ref: Self::Target) -> Self {
        Self { context_ref }
    }

    fn as_ptr(&self) -> Self::Target {
        self.context_ref
    }
}

impl Context {
    pub fn new() -> Self {
        let context_ref = unsafe { LLVMContextCreate() };
        Self { context_ref }
    }

    pub fn create_module<'ctx>(&mut self, name: &str) -> Module<'ctx> {
        let c_name = CString::new(name).unwrap();
        let module_ref =
            unsafe { LLVMModuleCreateWithNameInContext(c_name.as_ptr(), self.context_ref) };

        unsafe { Module::from_ptr(module_ref) }
    }

    pub fn set_diagnostic_handler<T>(&mut self, handler: &mut T)
    where
        T: LLVMDiagnosticHandler,
    {
        unsafe {
            LLVMContextSetDiagnosticHandler(
                self.context_ref,
                Some(diagnostic_handler::<DiagnosticHandler>),
                handler as *mut _ as _,
            )
        }
    }
}

pub struct Module<'ctx> {
    module_ref: LLVMModuleRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> Drop for Module<'ctx> {
    fn drop(&mut self) {
        tracing::debug!("dropping module");
        unsafe { LLVMDisposeModule(self.module_ref) }
    }
}

impl<'ctx> LLVMTypeWrapper for Module<'ctx> {
    type Target = LLVMModuleRef;

    unsafe fn from_ptr(module_ref: Self::Target) -> Self {
        Self {
            module_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.module_ref
    }
}

impl<'ctx> Module<'ctx> {
    pub fn create_di_builder(&self) -> DIBuilder {
        let di_builder_ref = unsafe { LLVMCreateDIBuilder(self.module_ref) };
        unsafe { DIBuilder::from_ptr(di_builder_ref) }
    }

    pub fn inline_asm(&self) -> Option<&CStr> {
        let mut len = 0;
        let ptr = unsafe { LLVMGetModuleInlineAsm(self.module_ref, &mut len) };
        NonNull::new(ptr as *mut _).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
    }

    pub fn set_module_inline_asm(&mut self, asm: &CStr) {
        unsafe { LLVMSetModuleInlineAsm2(self.module_ref, asm.as_ptr(), asm.count_bytes()) }
    }

    pub fn strip_debug_into(&mut self) -> bool {
        unsafe { LLVMStripModuleDebugInfo(self.module_ref) != 0 }
    }

    pub fn target(&self) -> Result<Target, LLVMError> {
        Target::from_triple(self.target_triple()?)
    }

    pub fn target_triple(&self) -> Result<&CStr, LLVMError> {
        let ptr = unsafe { LLVMGetTarget(self.module_ref) };
        NonNull::new(ptr as *mut _)
            .map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
            .ok_or(LLVMError::ModuleNoTargetTriple)
    }
}

#[derive(Clone)]
pub enum Value<'ctx> {
    MDNode(MDNode<'ctx>),
    Function(Function<'ctx>),
    Other(LLVMValueRef),
}

impl<'ctx> std::fmt::Debug for Value<'ctx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value_to_string = |value| {
            Message {
                ptr: unsafe { LLVMPrintValueToString(value) },
            }
            .as_c_str()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
        };
        match self {
            Self::MDNode(node) => f
                .debug_struct("MDNode")
                .field("value", &value_to_string(node.value_ref))
                .finish(),
            Self::Function(fun) => f
                .debug_struct("Function")
                .field("value", &value_to_string(fun.value_ref))
                .finish(),
            Self::Other(value) => f
                .debug_struct("Other")
                .field("value", &value_to_string(*value))
                .finish(),
        }
    }
}

impl<'ctx> LLVMTypeWrapper for Value<'ctx> {
    type Target = LLVMValueRef;

    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        if unsafe { !LLVMIsAMDNode(value_ref).is_null() } {
            let mdnode = unsafe { MDNode::from_ptr(value_ref) };
            return Value::MDNode(mdnode);
        } else if unsafe { !LLVMIsAFunction(value_ref).is_null() } {
            return Value::Function(unsafe { Function::from_ptr(value_ref) });
        }
        Value::Other(value_ref)
    }

    fn as_ptr(&self) -> Self::Target {
        match self {
            Value::MDNode(mdnode) => mdnode.as_ptr(),
            Value::Function(f) => f.as_ptr(),
            Value::Other(value_ref) => *value_ref,
        }
    }
}

impl<'ctx> Value<'ctx> {
    pub fn metadata_entries(&self) -> Option<MetadataEntries> {
        let value = match self {
            Value::MDNode(node) => node.value_ref,
            Value::Function(f) => f.value_ref,
            Value::Other(value) => *value,
        };
        MetadataEntries::new(value)
    }

    pub fn operands(&self) -> Option<impl Iterator<Item = LLVMValueRef>> {
        let value = match self {
            Value::MDNode(node) => Some(node.value_ref),
            Value::Function(f) => Some(f.value_ref),
            Value::Other(value) if unsafe { !LLVMIsAUser(*value).is_null() } => Some(*value),
            _ => None,
        };

        value.map(|value| unsafe {
            (0..LLVMGetNumOperands(value)).map(move |i| LLVMGetOperand(value, i as u32))
        })
    }
}

pub enum Metadata<'ctx> {
    DICompositeType(DICompositeType<'ctx>),
    DIDerivedType(DIDerivedType<'ctx>),
    DISubprogram(DISubprogram<'ctx>),
    Other(#[allow(dead_code)] LLVMValueRef),
}

impl<'ctx> Metadata<'ctx> {
    /// Constructs a new [`Metadata`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `Metadata`](https://llvm.org/doxygen/classllvm_1_1Metadata.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any valiation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let metadata = LLVMValueAsMetadata(value);

        match unsafe { LLVMGetMetadataKind(metadata) } {
            LLVMMetadataKind::LLVMDICompositeTypeMetadataKind => {
                let di_composite_type = unsafe { DICompositeType::from_ptr(value) };
                Metadata::DICompositeType(di_composite_type)
            }
            LLVMMetadataKind::LLVMDIDerivedTypeMetadataKind => {
                let di_derived_type = unsafe { DIDerivedType::from_ptr(value) };
                Metadata::DIDerivedType(di_derived_type)
            }
            LLVMMetadataKind::LLVMDISubprogramMetadataKind => {
                let di_subprogram = unsafe { DISubprogram::from_ptr(value) };
                Metadata::DISubprogram(di_subprogram)
            }
            LLVMMetadataKind::LLVMDIGlobalVariableMetadataKind
            | LLVMMetadataKind::LLVMDICommonBlockMetadataKind
            | LLVMMetadataKind::LLVMMDStringMetadataKind
            | LLVMMetadataKind::LLVMConstantAsMetadataMetadataKind
            | LLVMMetadataKind::LLVMLocalAsMetadataMetadataKind
            | LLVMMetadataKind::LLVMDistinctMDOperandPlaceholderMetadataKind
            | LLVMMetadataKind::LLVMMDTupleMetadataKind
            | LLVMMetadataKind::LLVMDILocationMetadataKind
            | LLVMMetadataKind::LLVMDIExpressionMetadataKind
            | LLVMMetadataKind::LLVMDIGlobalVariableExpressionMetadataKind
            | LLVMMetadataKind::LLVMGenericDINodeMetadataKind
            | LLVMMetadataKind::LLVMDISubrangeMetadataKind
            | LLVMMetadataKind::LLVMDIEnumeratorMetadataKind
            | LLVMMetadataKind::LLVMDIBasicTypeMetadataKind
            | LLVMMetadataKind::LLVMDISubroutineTypeMetadataKind
            | LLVMMetadataKind::LLVMDIFileMetadataKind
            | LLVMMetadataKind::LLVMDICompileUnitMetadataKind
            | LLVMMetadataKind::LLVMDILexicalBlockMetadataKind
            | LLVMMetadataKind::LLVMDILexicalBlockFileMetadataKind
            | LLVMMetadataKind::LLVMDINamespaceMetadataKind
            | LLVMMetadataKind::LLVMDIModuleMetadataKind
            | LLVMMetadataKind::LLVMDITemplateTypeParameterMetadataKind
            | LLVMMetadataKind::LLVMDITemplateValueParameterMetadataKind
            | LLVMMetadataKind::LLVMDILocalVariableMetadataKind
            | LLVMMetadataKind::LLVMDILabelMetadataKind
            | LLVMMetadataKind::LLVMDIObjCPropertyMetadataKind
            | LLVMMetadataKind::LLVMDIImportedEntityMetadataKind
            | LLVMMetadataKind::LLVMDIMacroMetadataKind
            | LLVMMetadataKind::LLVMDIMacroFileMetadataKind
            | LLVMMetadataKind::LLVMDIStringTypeMetadataKind
            | LLVMMetadataKind::LLVMDIGenericSubrangeMetadataKind
            | LLVMMetadataKind::LLVMDIArgListMetadataKind
            | LLVMMetadataKind::LLVMDIAssignIDMetadataKind => Metadata::Other(value),
        }
    }
}

impl<'ctx> TryFrom<MDNode<'ctx>> for Metadata<'ctx> {
    type Error = ();

    fn try_from(md_node: MDNode) -> Result<Self, Self::Error> {
        // FIXME: fail if md_node isn't a Metadata node
        Ok(unsafe { Self::from_value_ref(md_node.value_ref) })
    }
}

/// Represents a metadata node.
#[derive(Clone)]
pub struct MDNode<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for MDNode<'ctx> {
    type Target = LLVMValueRef;

    /// Constructs a new [`MDNode`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `MDNode`](https://llvm.org/doxygen/classllvm_1_1MDNode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any valiation checks.
    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}

impl<'ctx> MDNode<'ctx> {
    /// Constructs a new [`MDNode`] from the given `metadata`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `metadata` corresponds to a valid
    /// instance of [LLVM `MDNode`](https://llvm.org/doxygen/classllvm_1_1MDNode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Self {
        MDNode::from_ptr(LLVMMetadataAsValue(context, metadata))
    }

    /// Constructs an empty metadata node.
    /// Constructs a new [`MDNode`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `MDNode`](https://llvm.org/doxygen/classllvm_1_1MDNode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any valiation checks.
    pub fn empty(context: &Context) -> Self {
        let metadata = unsafe { LLVMMDNodeInContext2(context.as_ptr(), core::ptr::null_mut(), 0) };
        unsafe { Self::from_metadata_ref(context.as_ptr(), metadata) }
    }

    /// Constructs a new metadata node from an array of [`DIType`] elements.
    ///
    /// This function is used to create composite metadata structures, such as
    /// arrays or tuples of different types or values, which can then be used
    /// to represent complex data structures within the metadata system.
    pub fn with_elements(context: &Context, elements: &[DIType]) -> Self {
        let metadata = unsafe {
            let mut elements: Vec<LLVMMetadataRef> = elements
                .iter()
                .map(|di_type| LLVMValueAsMetadata(di_type.as_ptr()))
                .collect();
            LLVMMDNodeInContext2(
                context.as_ptr(),
                elements.as_mut_slice().as_mut_ptr(),
                elements.len(),
            )
        };
        unsafe { Self::from_metadata_ref(context.as_ptr(), metadata) }
    }
}

pub struct MetadataEntries {
    entries: *mut LLVMValueMetadataEntry,
    count: usize,
}

impl MetadataEntries {
    pub fn new(v: LLVMValueRef) -> Option<Self> {
        if unsafe { LLVMIsAGlobalObject(v).is_null() && LLVMIsAInstruction(v).is_null() } {
            return None;
        }

        let mut count = 0;
        let entries = unsafe { LLVMGlobalCopyAllMetadata(v, &mut count) };
        if entries.is_null() {
            return None;
        }

        Some(MetadataEntries { entries, count })
    }

    pub fn iter(&self) -> impl Iterator<Item = (LLVMMetadataRef, u32)> + '_ {
        (0..self.count).map(move |index| unsafe {
            (
                LLVMValueMetadataEntriesGetMetadata(self.entries, index as u32),
                LLVMValueMetadataEntriesGetKind(self.entries, index as u32),
            )
        })
    }
}

impl Drop for MetadataEntries {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeValueMetadataEntries(self.entries);
        }
    }
}

pub struct BasicBlock<'ctx> {
    basic_block_ref: LLVMBasicBlockRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for BasicBlock<'ctx> {
    type Target = LLVMBasicBlockRef;

    unsafe fn from_ptr(basic_block_ref: Self::Target) -> Self {
        Self {
            basic_block_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.basic_block_ref
    }
}

/// Represents a metadata node.
#[derive(Clone)]
pub struct Function<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> std::fmt::Debug for Function<'ctx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value_to_string = |value| {
            Message {
                ptr: unsafe { LLVMPrintValueToString(value) },
            }
            .as_c_str()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
        };
        f.debug_struct("Function")
            .field("value", &value_to_string(self.value_ref))
            .finish()
    }
}

impl<'ctx> LLVMTypeWrapper for Function<'ctx> {
    type Target = LLVMValueRef;

    /// Constructs a new [`Function`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `Function`](https://llvm.org/doxygen/classllvm_1_1Function.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any valiation checks.
    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}

impl<'ctx> Function<'ctx> {
    pub(crate) fn name(&self) -> &str {
        symbol_name(self.value_ref)
    }

    pub(crate) fn params(&self) -> impl Iterator<Item = LLVMValueRef> {
        let params_count = unsafe { LLVMCountParams(self.value_ref) };
        let value = self.value_ref;
        (0..params_count).map(move |i| unsafe { LLVMGetParam(value, i) })
    }

    // pub(crate) fn basic_blocks(&self) -> impl Iterator<Item = LLVMBasicBlockRef> + '_ {
    //     self.value_ref.basic_blocks_iter()
    // }

    pub(crate) fn subprogram(&self, context: &Context) -> Option<DISubprogram<'ctx>> {
        let subprogram = unsafe { LLVMGetSubprogram(self.value_ref) };
        NonNull::new(subprogram).map(|_| unsafe {
            DISubprogram::from_ptr(LLVMMetadataAsValue(context.as_ptr(), subprogram))
        })
    }

    pub(crate) fn set_subprogram(&mut self, subprogram: &DISubprogram) {
        unsafe { LLVMSetSubprogram(self.value_ref, LLVMValueAsMetadata(subprogram.as_ptr())) };
    }
}

#[derive(Clone)]
pub struct Instruction<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> std::fmt::Debug for Instruction<'ctx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value_to_string = |value| {
            Message {
                ptr: unsafe { LLVMPrintValueToString(value) },
            }
            .as_c_str()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
        };
        f.debug_struct("Instruction")
            .field("value", &value_to_string(self.value_ref))
            .finish()
    }
}

impl<'ctx> LLVMTypeWrapper for Instruction<'ctx> {
    type Target = LLVMValueRef;

    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}
