use std::{
    ffi::{CStr, NulError},
    marker::PhantomData,
    ptr::NonNull,
    str,
};

use gimli::DwTag;
use llvm_sys::{
    core::{LLVMGetNumOperands, LLVMGetOperand, LLVMReplaceMDNodeOperandWith, LLVMValueAsMetadata},
    debuginfo::{
        LLVMDIFileGetFilename, LLVMDIFlags, LLVMDIScopeGetFile, LLVMDISubprogramGetLine,
        LLVMDITypeGetFlags, LLVMDITypeGetLine, LLVMDITypeGetName, LLVMDITypeGetOffsetInBits,
        LLVMDisposeDIBuilder, LLVMGetDINodeTag,
    },
    prelude::{LLVMDIBuilderRef, LLVMMetadataRef, LLVMValueRef},
};

use crate::llvm::{
    mdstring_to_str,
    types::{
        ir::{MDNode, Metadata},
        LLVMTypeWrapper,
    },
};

use super::ir::Context;

/// Returns a DWARF tag for the given debug info node.
///
/// This function should be called in `tag` method of all LLVM debug info types
/// inheriting from [`DINode`](https://llvm.org/doxygen/classllvm_1_1DINode.html).
///
/// # Safety
///
/// This function assumes that the given `metadata_ref` corresponds to a valid
/// instance of [LLVM `DINode`](https://llvm.org/doxygen/classllvm_1_1DINode.html).
/// It's the caller's responsibility to ensure this invariant, as this function
/// doesn't perform any validation checks.
unsafe fn di_node_tag(metadata_ref: LLVMMetadataRef) -> DwTag {
    DwTag(LLVMGetDINodeTag(metadata_ref))
}

pub struct DIBuilder<'ctx> {
    di_builder_ref: LLVMDIBuilderRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> Drop for DIBuilder<'ctx> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeDIBuilder(self.di_builder_ref) }
    }
}

impl<'ctx> LLVMTypeWrapper for DIBuilder<'ctx> {
    type Target = LLVMDIBuilderRef;

    unsafe fn from_ptr(di_builder_ref: Self::Target) -> Self {
        Self {
            di_builder_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.di_builder_ref
    }
}

/// Represents a source code file in debug infomation.
///
/// A `DIFile` debug info node, which represents a given file, is referenced by
/// other debug info nodes which belong to the file.
pub struct DIFile<'ctx> {
    metadata_ref: LLVMMetadataRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for DIFile<'ctx> {
    type Target = LLVMMetadataRef;

    /// Constructs a new [`DIFile`] from the given `metadata`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `metadata` corresponds to a valid
    /// instance of [LLVM `DIFile`](https://llvm.org/doxygen/classllvm_1_1DIFile.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    unsafe fn from_ptr(metadata_ref: Self::Target) -> Self {
        Self {
            metadata_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.metadata_ref
    }
}

impl<'ctx> DIFile<'ctx> {
    pub fn filename(&self) -> Option<&CStr> {
        let mut len = 0;
        // `LLVMDIFileGetName` doesn't allocate any memory, it just returns
        // a pointer to the string which is already a part of `DIFile`:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/lib/IR/DebugInfo.cpp#L1175-L1179
        //
        // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
        // gets freed when calling `LLVMDisposeDIBuilder`.
        let ptr = unsafe { LLVMDIFileGetFilename(self.metadata_ref, &mut len) };
        NonNull::new(ptr as *mut _).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
    }
}

/// Represents the operands for a [`DIType`]. The enum values correspond to the
/// operand indices within metadata nodes.
#[repr(u32)]
enum DITypeOperand {
    /// Name of the type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L743).
    Name = 2,
}

/// Returns the name of the type.
///
/// This function should be called in `name` method of `DIType` and all other
/// LLVM debug info types inheriting from it.
///
/// # Safety
///
/// This function assumes that the given `metadata_ref` corresponds to a valid
/// instance of [LLVM `DIType`](https://llvm.org/doxygen/classllvm_1_1DIType.html).
/// It's the caller's responsibility to ensure this invariant, as this function
/// doesn't perform any validation checks.
unsafe fn di_type_name<'a>(metadata_ref: LLVMMetadataRef) -> Option<&'a CStr> {
    let mut len = 0;
    // `LLVMDITypeGetName` doesn't allocate any memory, it just returns
    // a pointer to the string which is already a part of `DIType`:
    // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/lib/IR/DebugInfo.cpp#L1489-L1493
    //
    // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
    // gets freed when calling `LLVMDisposeDIBuilder`. Example:
    // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/tools/llvm-c-test/debuginfo.c#L249-L255
    let ptr = LLVMDITypeGetName(metadata_ref, &mut len);
    NonNull::new(ptr as *mut _).map(|ptr| CStr::from_ptr(ptr.as_ptr()))
}

/// Represents the debug information for a primitive type in LLVM IR.
pub struct DIType<'ctx> {
    metadata_ref: LLVMMetadataRef,
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for DIType<'ctx> {
    type Target = LLVMValueRef;

    /// Constructs a new [`DIType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `value` corresponds to a valid
    /// instance of [LLVM `DIType`](https://llvm.org/doxygen/classllvm_1_1DIType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        let metadata_ref = unsafe { LLVMValueAsMetadata(value_ref) };
        Self {
            metadata_ref,
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}

impl<'ctx> DIType<'ctx> {
    /// Returns the offset of the type in bits. This offset is used in case the
    /// type is a member of a composite type.
    pub fn offset_in_bits(&self) -> usize {
        unsafe { LLVMDITypeGetOffsetInBits(self.metadata_ref) as usize }
    }
}

impl<'ctx> From<DIDerivedType<'ctx>> for DIType<'ctx> {
    fn from(di_derived_type: DIDerivedType) -> Self {
        unsafe { Self::from_ptr(di_derived_type.value_ref) }
    }
}

/// Represents the operands for a [`DIDerivedType`]. The enum values correspond
/// to the operand indices within metadata nodes.
#[repr(u32)]
enum DIDerivedTypeOperand {
    /// [`DIType`] representing a base type of the given derived type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L1032).
    BaseType = 3,
}

/// Represents the debug information for a derived type in LLVM IR.
///
/// The types derived from other types usually add a level of indirection or an
/// alternative name. The examples of derived types are pointers, references,
/// typedefs, etc.
pub struct DIDerivedType<'ctx> {
    metadata_ref: LLVMMetadataRef,
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for DIDerivedType<'ctx> {
    type Target = LLVMValueRef;

    /// Constructs a new [`DIDerivedType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DIDerivedType`](https://llvm.org/doxygen/classllvm_1_1DIDerivedType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        let metadata_ref = LLVMValueAsMetadata(value_ref);
        Self {
            metadata_ref,
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}

impl<'ctx> DIDerivedType<'ctx> {
    /// Returns the base type of this derived type.
    pub fn base_type(&self) -> Metadata {
        unsafe {
            let value = LLVMGetOperand(self.value_ref, DIDerivedTypeOperand::BaseType as u32);
            Metadata::from_value_ref(value)
        }
    }

    /// Replaces the name of the type with a new name.
    ///
    /// # Errors
    ///
    /// Returns a `NulError` if the new name contains a NUL byte, as it cannot
    /// be converted into a `CString`.
    pub fn replace_name(&mut self, context: &Context, name: &str) -> Result<(), NulError> {
        super::ir::replace_name(self.value_ref, context, DITypeOperand::Name as u32, name)
    }

    /// Returns a DWARF tag of the given derived type.
    pub fn tag(&self) -> DwTag {
        unsafe { di_node_tag(self.metadata_ref) }
    }
}

/// Represents the operands for a [`DICompositeType`]. The enum values
/// correspond to the operand indices within metadata nodes.
#[repr(u32)]
enum DICompositeTypeOperand {
    /// Elements of the composite type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L1230).
    Elements = 4,
}

/// Represents the debug info for a composite type in LLVM IR.
///
/// Composite type is a kind of type that can include other types, such as
/// structures, enums, unions, etc.
pub struct DICompositeType<'ctx> {
    metadata_ref: LLVMMetadataRef,
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for DICompositeType<'ctx> {
    type Target = LLVMValueRef;

    /// Constructs a new [`DICompositeType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DICompositeType`](https://llvm.org/doxygen/classllvm_1_1DICompositeType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        let metadata_ref = LLVMValueAsMetadata(value_ref);
        Self {
            metadata_ref,
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}

impl<'ctx> DICompositeType<'ctx> {
    /// Returns an iterator over elements (struct fields, enum variants, etc.)
    /// of the composite type.
    pub fn elements(&self) -> impl Iterator<Item = Metadata> {
        let elements =
            unsafe { LLVMGetOperand(self.value_ref, DICompositeTypeOperand::Elements as u32) };
        let operands = NonNull::new(elements)
            .map(|elements| unsafe { LLVMGetNumOperands(elements.as_ptr()) })
            .unwrap_or(0);

        (0..operands)
            .map(move |i| unsafe { Metadata::from_value_ref(LLVMGetOperand(elements, i as u32)) })
    }

    /// Returns the name of the composite type.
    pub fn name(&self) -> Option<&CStr> {
        unsafe { di_type_name(self.metadata_ref) }
    }

    /// Returns the file that the composite type belongs to.
    pub fn file(&self) -> DIFile {
        unsafe {
            let metadata = LLVMDIScopeGetFile(self.metadata_ref);
            DIFile::from_ptr(metadata)
        }
    }

    /// Returns the flags associated with the composity type.
    pub fn flags(&self) -> LLVMDIFlags {
        unsafe { LLVMDITypeGetFlags(self.metadata_ref) }
    }

    /// Returns the line number in the source code where the type is defined.
    pub fn line(&self) -> u32 {
        unsafe { LLVMDITypeGetLine(self.metadata_ref) }
    }

    /// Replaces the elements of the composite type with a new metadata node.
    /// The provided metadata node should contain new composite type elements
    /// as operants. The metadata node can be empty if the intention is to
    /// remove all elements of the composite type.
    pub fn replace_elements(&mut self, mdnode: MDNode) {
        unsafe {
            LLVMReplaceMDNodeOperandWith(
                self.value_ref,
                DICompositeTypeOperand::Elements as u32,
                LLVMValueAsMetadata(mdnode.as_ptr()),
            )
        }
    }

    /// Replaces the name of the type with a new name.
    ///
    /// # Errors
    ///
    /// Returns a `NulError` if the new name contains a NUL byte, as it cannot
    /// be converted into a `CString`.
    pub fn replace_name(&mut self, context: &Context, name: &str) -> Result<(), NulError> {
        super::ir::replace_name(self.value_ref, context, DITypeOperand::Name as u32, name)
    }

    /// Returns a DWARF tag of the given composite type.
    pub fn tag(&self) -> DwTag {
        unsafe { di_node_tag(self.metadata_ref) }
    }
}

/// Represents the operands for a [`DISubprogram`]. The enum values correspond
/// to the operand indices within metadata nodes.
#[repr(u32)]
enum DISubprogramOperand {
    Scope = 1,
    Name = 2,
    LinkageName = 3,
    Ty = 4,
    Unit = 5,
    RetainedNodes = 7,
}

/// Represents the debug information for a subprogram (function) in LLVM IR.
pub struct DISubprogram<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> LLVMTypeWrapper for DISubprogram<'ctx> {
    type Target = LLVMValueRef;

    /// Constructs a new [`DISubprogram`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DISubprogram`](https://llvm.org/doxygen/classllvm_1_1DISubprogram.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    unsafe fn from_ptr(value_ref: Self::Target) -> Self {
        DISubprogram {
            value_ref,
            _marker: PhantomData,
        }
    }

    fn as_ptr(&self) -> Self::Target {
        self.value_ref
    }
}

impl<'ctx> DISubprogram<'ctx> {
    /// Returns the name of the subprogram.
    pub fn name(&self) -> Option<&str> {
        let operand = unsafe { LLVMGetOperand(self.value_ref, DISubprogramOperand::Name as u32) };
        NonNull::new(operand).map(|_| mdstring_to_str(operand))
    }

    /// Returns the linkage name of the subprogram.
    pub fn linkage_name(&self) -> Option<&str> {
        let operand =
            unsafe { LLVMGetOperand(self.value_ref, DISubprogramOperand::LinkageName as u32) };
        NonNull::new(operand).map(|_| mdstring_to_str(operand))
    }

    pub fn ty(&self) -> LLVMMetadataRef {
        unsafe {
            LLVMValueAsMetadata(LLVMGetOperand(
                self.value_ref,
                DISubprogramOperand::Ty as u32,
            ))
        }
    }

    pub fn file(&self) -> LLVMMetadataRef {
        unsafe { LLVMDIScopeGetFile(LLVMValueAsMetadata(self.value_ref)) }
    }

    pub fn line(&self) -> u32 {
        unsafe { LLVMDISubprogramGetLine(LLVMValueAsMetadata(self.value_ref)) }
    }

    pub fn type_flags(&self) -> i32 {
        unsafe { LLVMDITypeGetFlags(LLVMValueAsMetadata(self.value_ref)) }
    }

    /// Replaces the name of the subprogram with a new name.
    ///
    /// # Errors
    ///
    /// Returns a `NulError` if the new name contains a NUL byte, as it cannot
    /// be converted into a `CString`.
    pub fn replace_name(&mut self, context: &Context, name: &str) -> Result<(), NulError> {
        super::ir::replace_name(
            self.value_ref,
            context,
            DISubprogramOperand::Name as u32,
            name,
        )
    }

    pub fn scope(&self) -> Option<LLVMMetadataRef> {
        unsafe {
            let operand = LLVMGetOperand(self.value_ref, DISubprogramOperand::Scope as u32);
            NonNull::new(operand).map(|_| LLVMValueAsMetadata(operand))
        }
    }

    pub fn unit(&self) -> Option<LLVMMetadataRef> {
        unsafe {
            let operand = LLVMGetOperand(self.value_ref, DISubprogramOperand::Unit as u32);
            NonNull::new(operand).map(|_| LLVMValueAsMetadata(operand))
        }
    }

    pub fn set_unit(&mut self, unit: LLVMMetadataRef) {
        unsafe {
            LLVMReplaceMDNodeOperandWith(self.value_ref, DISubprogramOperand::Unit as u32, unit)
        };
    }

    pub fn retained_nodes(&self) -> Option<LLVMMetadataRef> {
        unsafe {
            let nodes = LLVMGetOperand(self.value_ref, DISubprogramOperand::RetainedNodes as u32);
            NonNull::new(nodes).map(|_| LLVMValueAsMetadata(nodes))
        }
    }

    pub fn set_retained_nodes(&mut self, nodes: LLVMMetadataRef) {
        unsafe {
            LLVMReplaceMDNodeOperandWith(
                self.value_ref,
                DISubprogramOperand::RetainedNodes as u32,
                nodes,
            )
        };
    }
}
