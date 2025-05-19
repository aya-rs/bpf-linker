use std::{
    borrow::Cow,
    ffi::{c_uchar, CString, NulError},
    marker::PhantomData,
    ptr::NonNull,
    slice,
};

use llvm_sys::{
    core::{
        LLVMCountParams, LLVMDisposeValueMetadataEntries, LLVMGetModuleInlineAsm,
        LLVMGetNumOperands, LLVMGetOperand, LLVMGetParam, LLVMGetValueName2,
        LLVMGlobalCopyAllMetadata, LLVMIsAArgument, LLVMIsAFunction, LLVMIsAGlobalAlias,
        LLVMIsAGlobalObject, LLVMIsAGlobalVariable, LLVMIsAInstruction, LLVMIsAMDNode, LLVMIsAUser,
        LLVMMDNodeInContext2, LLVMMDStringInContext2, LLVMMetadataAsValue,
        LLVMModuleCreateWithNameInContext, LLVMPrintValueToString, LLVMReplaceMDNodeOperandWith,
        LLVMSetLinkage, LLVMSetModuleInlineAsm2, LLVMSetVisibility, LLVMValueAsMetadata,
        LLVMValueMetadataEntriesGetKind, LLVMValueMetadataEntriesGetMetadata,
    },
    debuginfo::{LLVMGetMetadataKind, LLVMGetSubprogram, LLVMMetadataKind, LLVMSetSubprogram},
    prelude::{LLVMContextRef, LLVMMetadataRef, LLVMValueMetadataEntry, LLVMValueRef},
    LLVMBasicBlock, LLVMLinkage, LLVMModule, LLVMValue, LLVMVisibility,
};

use crate::llvm::{
    types::{
        di::{DICompositeType, DIDerivedType, DISubprogram, DIType},
        LLVMTypeError, LLVMTypeWrapper,
    },
    Message,
};

pub struct Module<'ctx> {
    module: NonNull<LLVMModule>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for Module<'_> {
    type Target = LLVMModule;

    fn from_ptr(module: NonNull<Self::Target>) -> Result<Self, LLVMTypeError>
    where
        Self: Sized,
    {
        Ok(Self {
            module,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.module.as_ptr()
    }
}

impl Module<'_> {
    pub fn new(name: &str, context: LLVMContextRef) -> Self {
        let name = CString::new(name).unwrap();
        let module = unsafe { LLVMModuleCreateWithNameInContext(name.as_ptr(), context) };
        let module = NonNull::new(module).expect("");
        Self {
            module,
            _marker: PhantomData,
        }
    }

    pub fn inline_asm(&self) -> Cow<'_, str> {
        let mut len = 0;
        let ptr = unsafe { LLVMGetModuleInlineAsm(self.module.as_ptr(), &mut len) };
        let asm = unsafe { slice::from_raw_parts(ptr as *const c_uchar, len) };
        String::from_utf8_lossy(asm)
    }

    pub fn set_inline_asm(&mut self, asm: &str) {
        let len = asm.len();
        let asm = CString::new(asm).unwrap();
        unsafe {
            LLVMSetModuleInlineAsm2(self.module.as_ptr(), asm.as_ptr(), len);
        }
    }
}

pub(crate) fn symbol_name<'a>(value: LLVMValueRef) -> Cow<'a, str> {
    let mut len = 0;
    let ptr = unsafe { LLVMGetValueName2(value, &mut len) };
    let symbol_name = unsafe { slice::from_raw_parts(ptr as *const c_uchar, len) };
    String::from_utf8_lossy(symbol_name)
}

pub(crate) fn replace_name(
    value_ref: LLVMValueRef,
    context: LLVMContextRef,
    name_operand_index: u32,
    name: &str,
) -> Result<(), NulError> {
    let cstr = CString::new(name)?;
    let name = unsafe { LLVMMDStringInContext2(context, cstr.as_ptr(), name.len()) };
    unsafe { LLVMReplaceMDNodeOperandWith(value_ref, name_operand_index, name) };
    Ok(())
}

#[derive(Clone)]
pub enum Value<'ctx> {
    MDNode(MDNode<'ctx>),
    Function(Function<'ctx>),
    Other(NonNull<LLVMValue>),
}

impl std::fmt::Debug for Value<'_> {
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
                .field("value", &value_to_string(node.value.as_ptr()))
                .finish(),
            Self::Function(fun) => f
                .debug_struct("Function")
                .field("value", &value_to_string(fun.value.as_ptr()))
                .finish(),
            Self::Other(value) => f
                .debug_struct("Other")
                .field("value", &value_to_string(value.as_ptr()))
                .finish(),
        }
    }
}

impl LLVMTypeWrapper for Value<'_> {
    type Target = LLVMValue;

    fn from_ptr(value_ref: NonNull<Self::Target>) -> Result<Self, LLVMTypeError> {
        if unsafe { !LLVMIsAMDNode(value_ref.as_ptr()).is_null() } {
            let mdnode = MDNode::from_ptr(value_ref)?;
            Ok(Value::MDNode(mdnode))
        } else if unsafe { !LLVMIsAFunction(value_ref.as_ptr()).is_null() } {
            Ok(Value::Function(Function::from_ptr(value_ref)?))
        } else {
            Ok(Value::Other(value_ref))
        }
    }

    fn as_ptr(&self) -> *mut Self::Target {
        match self {
            Value::MDNode(mdnode) => mdnode.as_ptr(),
            Value::Function(f) => f.as_ptr(),
            Value::Other(value) => value.as_ptr(),
        }
    }
}

impl Value<'_> {
    pub fn metadata_entries(&self) -> Option<MetadataEntries> {
        let value = match self {
            Value::MDNode(node) => node.value.as_ptr(),
            Value::Function(f) => f.value.as_ptr(),
            Value::Other(value) => value.as_ptr(),
        };
        MetadataEntries::new(value)
    }

    pub fn operands(&self) -> Option<impl Iterator<Item = LLVMValueRef>> {
        let value = match self {
            Value::MDNode(node) => Some(node.value.as_ptr()),
            Value::Function(f) => Some(f.value.as_ptr()),
            Value::Other(value) if unsafe { !LLVMIsAUser(value.as_ptr()).is_null() } => {
                Some(value.as_ptr())
            }
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
    Other(#[allow(dead_code)] NonNull<LLVMValue>),
}

impl Metadata<'_> {
    /// Constructs a new [`Metadata`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `Metadata`](https://llvm.org/doxygen/classllvm_1_1Metadata.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any valiation checks.
    pub(crate) fn from_value(value: NonNull<LLVMValue>) -> Result<Self, LLVMTypeError> {
        let metadata = unsafe { LLVMValueAsMetadata(value.as_ptr()) };

        match unsafe { LLVMGetMetadataKind(metadata) } {
            LLVMMetadataKind::LLVMDICompositeTypeMetadataKind => {
                let di_composite_type = DICompositeType::from_ptr(value)?;
                Ok(Metadata::DICompositeType(di_composite_type))
            }
            LLVMMetadataKind::LLVMDIDerivedTypeMetadataKind => {
                let di_derived_type = DIDerivedType::from_ptr(value)?;
                Ok(Metadata::DIDerivedType(di_derived_type))
            }
            LLVMMetadataKind::LLVMDISubprogramMetadataKind => {
                let di_subprogram = DISubprogram::from_ptr(value)?;
                Ok(Metadata::DISubprogram(di_subprogram))
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
            | LLVMMetadataKind::LLVMDIAssignIDMetadataKind => Ok(Metadata::Other(value)),
        }
    }
}

impl<'ctx> TryFrom<MDNode<'ctx>> for Metadata<'ctx> {
    type Error = LLVMTypeError;

    fn try_from(md_node: MDNode) -> Result<Self, Self::Error> {
        // FIXME: fail if md_node isn't a Metadata node
        Self::from_value(md_node.value)
    }
}

/// Represents a metadata node.
#[derive(Clone)]
pub struct MDNode<'ctx> {
    value: NonNull<LLVMValue>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for MDNode<'_> {
    type Target = LLVMValue;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError> {
        if unsafe { LLVMIsAMDNode(value.as_ptr()).is_null() } {
            return Err(LLVMTypeError::InvalidPointerType("MDNode"));
        }
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}

impl MDNode<'_> {
    /// Constructs a new [`MDNode`] from the given `metadata`.
    #[inline]
    pub(crate) fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Result<Self, LLVMTypeError> {
        let value_ref = unsafe { LLVMMetadataAsValue(context, metadata) };
        let value = NonNull::new(value_ref).ok_or(LLVMTypeError::NullPointer)?;
        MDNode::from_ptr(value)
    }

    /// Constructs an empty metadata node.
    #[inline]
    pub fn empty(context: LLVMContextRef) -> Self {
        let metadata = unsafe { LLVMMDNodeInContext2(context, core::ptr::null_mut(), 0) };
        Self::from_metadata_ref(context, metadata).expect("expected a valid MDNode")
    }

    /// Constructs a new metadata node from an array of [`DIType`] elements.
    ///
    /// This function is used to create composite metadata structures, such as
    /// arrays or tuples of different types or values, which can then be used
    /// to represent complex data structures within the metadata system.
    pub fn with_elements(context: LLVMContextRef, elements: &[DIType]) -> Self {
        let metadata = unsafe {
            let mut elements: Vec<LLVMMetadataRef> = elements
                .iter()
                .map(|di_type| LLVMValueAsMetadata(di_type.as_ptr()))
                .collect();
            LLVMMDNodeInContext2(
                context,
                elements.as_mut_slice().as_mut_ptr(),
                elements.len(),
            )
        };
        Self::from_metadata_ref(context, metadata).expect("expected a valid MDNode")
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
    value: NonNull<LLVMBasicBlock>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for BasicBlock<'_> {
    type Target = LLVMBasicBlock;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError> {
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}

pub trait GlobalValue: LLVMTypeWrapper<Target = LLVMValue> {
    fn set_linkage(&mut self, linkage: LLVMLinkage) {
        unsafe {
            LLVMSetLinkage(self.as_ptr(), linkage);
        }
    }

    fn set_visibility(&mut self, visibility: LLVMVisibility) {
        unsafe {
            LLVMSetVisibility(self.as_ptr(), visibility);
        }
    }
}

/// Formal argument to a [`Function`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Argument<'ctx> {
    value: NonNull<LLVMValue>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for Argument<'_> {
    type Target = LLVMValue;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError>
    where
        Self: Sized,
    {
        if unsafe { LLVMIsAArgument(value.as_ptr()).is_null() } {
            return Err(LLVMTypeError::InvalidPointerType("Argument"));
        }
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}

/// Represents a function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function<'ctx> {
    value: NonNull<LLVMValue>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for Function<'_> {
    type Target = LLVMValue;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError> {
        if unsafe { LLVMIsAFunction(value.as_ptr()).is_null() } {
            return Err(LLVMTypeError::InvalidPointerType("Function"));
        }
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}

impl GlobalValue for Function<'_> {}

impl<'ctx> Function<'ctx> {
    pub(crate) fn name(&self) -> Cow<'_, str> {
        symbol_name(self.value.as_ptr())
    }

    pub(crate) fn params(&self) -> impl Iterator<Item = Argument> {
        let params_count = unsafe { LLVMCountParams(self.value.as_ptr()) };
        let value = self.value.as_ptr();
        (0..params_count).map(move |i| {
            let ptr = unsafe { LLVMGetParam(value, i) };
            Argument::from_ptr(NonNull::new(ptr).expect("an argument should not be null")).unwrap()
        })
    }

    pub(crate) fn subprogram(&self, context: LLVMContextRef) -> Option<DISubprogram<'ctx>> {
        let subprogram = unsafe { LLVMGetSubprogram(self.value.as_ptr()) };
        let subprogram = NonNull::new(subprogram)?;
        let value = unsafe { LLVMMetadataAsValue(context, subprogram.as_ptr()) };
        let value = NonNull::new(value)?;
        Some(DISubprogram::from_ptr(value).unwrap())
    }

    pub(crate) fn set_subprogram(&mut self, subprogram: &DISubprogram) {
        unsafe {
            LLVMSetSubprogram(
                self.value.as_ptr(),
                LLVMValueAsMetadata(subprogram.as_ptr()),
            )
        };
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalAlias<'ctx> {
    value: NonNull<LLVMValue>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for GlobalAlias<'_> {
    type Target = LLVMValue;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError> {
        if unsafe { LLVMIsAGlobalAlias(value.as_ptr()).is_null() } {
            return Err(LLVMTypeError::InvalidPointerType("GlobalAlias"));
        }
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}

impl GlobalValue for GlobalAlias<'_> {}

impl GlobalAlias<'_> {
    pub fn name<'a>(&self) -> Cow<'a, str> {
        symbol_name(self.value.as_ptr())
    }
}

/// Represents a global variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalVariable<'ctx> {
    value: NonNull<LLVMValue>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for GlobalVariable<'_> {
    type Target = LLVMValue;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError> {
        if unsafe { LLVMIsAGlobalVariable(value.as_ptr()).is_null() } {
            return Err(LLVMTypeError::InvalidPointerType("GlobalVariable"));
        }
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}

impl GlobalValue for GlobalVariable<'_> {}

impl GlobalVariable<'_> {
    pub fn name<'a>(&self) -> Cow<'a, str> {
        symbol_name(self.value.as_ptr())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction<'ctx> {
    value: NonNull<LLVMValue>,
    _marker: PhantomData<&'ctx ()>,
}

impl LLVMTypeWrapper for Instruction<'_> {
    type Target = LLVMValue;

    fn from_ptr(value: NonNull<Self::Target>) -> Result<Self, LLVMTypeError>
    where
        Self: Sized,
    {
        Ok(Self {
            value,
            _marker: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut Self::Target {
        self.value.as_ptr()
    }
}
