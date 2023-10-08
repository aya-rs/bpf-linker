use llvm_sys::{
    core::{LLVMIsAMDNode, LLVMMDNodeInContext2, LLVMMetadataAsValue, LLVMValueAsMetadata},
    debuginfo::{LLVMGetMetadataKind, LLVMMetadataKind},
    prelude::*,
};

use super::di::{
    DICommonBlock, DICompositeType, DIDerivedType, DIGlobalVariable, DISubprogram, DIType,
};

pub enum ValueType {
    MDNode(MDNode),
    Unknown,
}

pub struct Value {
    value: LLVMValueRef,
}

impl Value {
    pub fn new(value: LLVMValueRef) -> Self {
        Self { value }
    }

    pub fn to_value_type(&self) -> ValueType {
        if unsafe { !LLVMIsAMDNode(self.value).is_null() } {
            let mdnode = unsafe { MDNode::from_value_ref(self.value) };
            return ValueType::MDNode(mdnode);
        }
        ValueType::Unknown
    }
}

pub enum MetadataKind {
    DICompositeType(DICompositeType),
    DIGlobalVariable(DIGlobalVariable),
    DICommonBlock(DICommonBlock),
    DIDerivedType(DIDerivedType),
    DISubprogram(DISubprogram),
    Unknown,
}

pub struct Metadata {
    pub(crate) metadata: LLVMMetadataRef,
    pub(crate) value: LLVMValueRef,
}

impl Metadata {
    pub(crate) unsafe fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Self {
        let value = LLVMMetadataAsValue(context, metadata);
        Self { metadata, value }
    }

    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let metadata = LLVMValueAsMetadata(value);
        Self { metadata, value }
    }

    pub fn to_metadata_kind(&self) -> MetadataKind {
        match unsafe { LLVMGetMetadataKind(self.metadata) } {
            LLVMMetadataKind::LLVMDICompositeTypeMetadataKind => {
                let di_composite_type = unsafe { DICompositeType::from_value_ref(self.value) };
                MetadataKind::DICompositeType(di_composite_type)
            }
            LLVMMetadataKind::LLVMDIGlobalVariableMetadataKind => {
                let di_global_variale = unsafe { DIGlobalVariable::from_value_ref(self.value) };
                MetadataKind::DIGlobalVariable(di_global_variale)
            }
            LLVMMetadataKind::LLVMDICommonBlockMetadataKind => {
                let di_common_block = unsafe { DICommonBlock::from_value_ref(self.value) };
                MetadataKind::DICommonBlock(di_common_block)
            }
            LLVMMetadataKind::LLVMDIDerivedTypeMetadataKind => {
                let di_derived_type = unsafe { DIDerivedType::from_value_ref(self.value) };
                MetadataKind::DIDerivedType(di_derived_type)
            }
            LLVMMetadataKind::LLVMDISubprogramMetadataKind => {
                let di_subprogram = unsafe { DISubprogram::from_value_ref(self.value) };
                MetadataKind::DISubprogram(di_subprogram)
            }
            _ => MetadataKind::Unknown,
        }
    }
}

pub struct MDNode {
    pub metadata: Metadata,
}

impl MDNode {
    pub(crate) unsafe fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Self {
        let metadata = Metadata::from_metadata_ref(context, metadata);
        Self { metadata }
    }

    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let metadata = Metadata::from_value_ref(value);
        Self { metadata }
    }

    pub fn empty(context: LLVMContextRef) -> Self {
        let metadata = unsafe {
            let metadata = LLVMMDNodeInContext2(context, core::ptr::null_mut(), 0);
            Metadata::from_metadata_ref(context, metadata)
        };
        Self { metadata }
    }

    pub fn with_elements(context: LLVMContextRef, elements: &[DIType]) -> Self {
        let metadata = unsafe {
            let mut elements: Vec<LLVMMetadataRef> = elements
                .iter()
                .map(|di_type| di_type.di_scope.di_node.md_node.metadata.metadata)
                .collect();
            let metadata = LLVMMDNodeInContext2(
                context,
                elements.as_mut_slice().as_mut_ptr(),
                elements.len(),
            );
            Metadata::from_metadata_ref(context, metadata)
        };
        Self { metadata }
    }
}
