use std::ffi::{CString, NulError};

use llvm_sys::{
    core::{
        LLVMIsAMDNode, LLVMMDNodeInContext2, LLVMMDStringInContext2, LLVMMetadataAsValue,
        LLVMReplaceMDNodeOperandWith, LLVMValueAsMetadata,
    },
    debuginfo::{LLVMGetMetadataKind, LLVMMetadataKind},
    prelude::{LLVMContextRef, LLVMMetadataRef, LLVMValueRef},
};

use super::di::{
    DICommonBlock, DICompositeType, DIDerivedType, DIGlobalVariable, DISubprogram, DIType,
};

pub enum Value {
    MDNode(MDNode),
    Other(LLVMValueRef),
}

impl Value {
    pub fn new(value: LLVMValueRef) -> Self {
        if unsafe { !LLVMIsAMDNode(value).is_null() } {
            let mdnode = unsafe { MDNode::from_value_ref(value) };
            return Value::MDNode(mdnode);
        }
        Value::Other(value)
    }
}

pub enum Metadata {
    DICompositeType(DICompositeType),
    DIGlobalVariable(DIGlobalVariable),
    DICommonBlock(DICommonBlock),
    DIDerivedType(DIDerivedType),
    DISubprogram(DISubprogram),
    Other(LLVMValueRef),
}

impl Metadata {
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
                let di_composite_type = unsafe { DICompositeType::from_value_ref(value) };
                Metadata::DICompositeType(di_composite_type)
            }
            LLVMMetadataKind::LLVMDIGlobalVariableMetadataKind => {
                let di_global_variale = unsafe { DIGlobalVariable::from_value_ref(value) };
                Metadata::DIGlobalVariable(di_global_variale)
            }
            LLVMMetadataKind::LLVMDICommonBlockMetadataKind => {
                let di_common_block = unsafe { DICommonBlock::from_value_ref(value) };
                Metadata::DICommonBlock(di_common_block)
            }
            LLVMMetadataKind::LLVMDIDerivedTypeMetadataKind => {
                let di_derived_type = unsafe { DIDerivedType::from_value_ref(value) };
                Metadata::DIDerivedType(di_derived_type)
            }
            LLVMMetadataKind::LLVMDISubprogramMetadataKind => {
                let di_subprogram = unsafe { DISubprogram::from_value_ref(value) };
                Metadata::DISubprogram(di_subprogram)
            }
            LLVMMetadataKind::LLVMMDStringMetadataKind
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

impl TryFrom<MDNode> for Metadata {
    type Error = ();

    fn try_from(md_node: MDNode) -> Result<Self, Self::Error> {
        // FIXME: fail if md_node isn't a Metadata node
        Ok(unsafe { Self::from_value_ref(md_node.value) })
    }
}

/// Represents a metadata node.
pub struct MDNode {
    pub value: LLVMValueRef,
}

impl MDNode {
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
        MDNode::from_value_ref(LLVMMetadataAsValue(context, metadata))
    }

    /// Constructs a new [`MDNode`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `MDNode`](https://llvm.org/doxygen/classllvm_1_1MDNode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any valiation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        Self { value }
    }

    /// Returns the low level `LLVMMetadataRef` corresponding to this node.
    pub fn metadata(&self) -> LLVMMetadataRef {
        unsafe { LLVMValueAsMetadata(self.value) }
    }

    /// Constructs an empty metadata node.
    pub fn empty(context: LLVMContextRef) -> Self {
        let metadata = unsafe { LLVMMDNodeInContext2(context, core::ptr::null_mut(), 0) };
        unsafe { Self::from_metadata_ref(context, metadata) }
    }

    /// Replaces the name of the subprogram with a new name.
    ///
    /// # Errors
    ///
    /// Returns a `NulError` if the new name contains a NUL byte, as it cannot
    /// be converted into a `CString`.
    pub(crate) fn replace_name(
        &mut self,
        context: LLVMContextRef,
        name_operand_index: u32,
        name: &str,
    ) -> Result<(), NulError> {
        let value = self.value;
        let cstr = CString::new(name)?;
        let name = unsafe { LLVMMDStringInContext2(context, cstr.as_ptr(), name.len()) };
        unsafe { LLVMReplaceMDNodeOperandWith(value, name_operand_index, name) };
        Ok(())
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
                .map(|di_type| LLVMValueAsMetadata(di_type.di_scope.di_node.md_node.value))
                .collect();
            LLVMMDNodeInContext2(
                context,
                elements.as_mut_slice().as_mut_ptr(),
                elements.len(),
            )
        };
        unsafe { Self::from_metadata_ref(context, metadata) }
    }
}
