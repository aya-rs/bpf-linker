use std::{
    ffi::{CStr, NulError},
    marker::PhantomData,
    ptr::NonNull,
};

use gimli::DwTag;
use llvm_sys::{
    core::{
        LLVMGetMDString, LLVMGetNumOperands, LLVMGetOperand, LLVMReplaceMDNodeOperandWith,
        LLVMValueAsMetadata,
    },
    debuginfo::{
        LLVMDIFileGetFilename, LLVMDIFlags, LLVMDIScopeGetFile, LLVMDITypeGetFlags,
        LLVMDITypeGetLine, LLVMDITypeGetName, LLVMDITypeGetOffsetInBits, LLVMGetDINodeTag,
    },
    prelude::{LLVMContextRef, LLVMMetadataRef, LLVMValueRef},
};

use super::ir::{MDNode, Metadata};

/// Represents a debug info node.
///
/// `DINode` is a fundamental structure used in the construction of LLVM's
/// debugging information ecosystem. It serves as a building block for more
/// complex debug information entities such as scopes, types and variables.
pub struct DINode<'ctx> {
    pub(super) value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DINode<'ctx> {
    /// Constructs a new [`DINode`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DINode`](https://llvm.org/doxygen/classllvm_1_1DINode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    /// Returns the low level `LLVMMetadataRef` corresponding to this node.
    pub fn metadata(&self) -> LLVMMetadataRef {
        unsafe { LLVMValueAsMetadata(self.value_ref) }
    }

    /// Returns a DWARF tag for the given debug info node.
    pub fn tag(&self) -> DwTag {
        DwTag(unsafe { LLVMGetDINodeTag(self.metadata()) })
    }
}

impl<'ctx> TryFrom<DIDerivedType<'ctx>> for DINode<'ctx> {
    type Error = ();

    fn try_from(di_derived_type: DIDerivedType) -> Result<Self, Self::Error> {
        // FIXME: perform a check
        Ok(unsafe { Self::from_value_ref(di_derived_type.value_ref) })
    }
}

impl<'ctx> TryFrom<DICompositeType<'ctx>> for DINode<'ctx> {
    type Error = ();

    fn try_from(di_composite_type: DICompositeType) -> Result<Self, Self::Error> {
        // FIXME: perform a check
        Ok(unsafe { Self::from_value_ref(di_composite_type.value_ref) })
    }
}

/// Represents the debug information for a code scope.
pub struct DIScope<'ctx> {
    pub(super) value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DIScope<'ctx> {
    /// Constructs a new [`DIScope`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `value` corresponds to a valid
    /// instance of [LLVM `DIScope`](https://llvm.org/doxygen/classllvm_1_1DIScope.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    /// Returns the low level `LLVMMetadataRef` corresponding to this node.
    pub fn metadata(&self) -> LLVMMetadataRef {
        unsafe { LLVMValueAsMetadata(self.value_ref) }
    }

    pub fn file(&self) -> DIFile {
        unsafe {
            let metadata = LLVMDIScopeGetFile(self.metadata());
            DIFile::from_metadata_ref(metadata)
        }
    }
}

impl<'ctx> TryFrom<DICompositeType<'ctx>> for DIScope<'ctx> {
    type Error = ();

    fn try_from(di_composite_type: DICompositeType) -> Result<Self, Self::Error> {
        // FIXME: perform a check
        Ok(unsafe { Self::from_value_ref(di_composite_type.value_ref) })
    }
}

/// Represents a source code file in debug infomation.
///
/// A `DIFile` debug info node, which represents a given file, is referenced by
/// other debug info nodes which belong to the file.
pub struct DIFile<'ctx> {
    pub(super) metadata_ref: LLVMMetadataRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DIFile<'ctx> {
    /// Constructs a new [`DIFile`] from the given `metadata`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `metadata` corresponds to a valid
    /// instance of [LLVM `DIFile`](https://llvm.org/doxygen/classllvm_1_1DIFile.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_metadata_ref(metadata_ref: LLVMMetadataRef) -> Self {
        Self {
            metadata_ref,
            _marker: PhantomData,
        }
    }

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

/// Represents the debug information for a primitive type in LLVM IR.
pub struct DIType<'ctx> {
    pub(super) metadata_ref: LLVMMetadataRef,
    pub(super) value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DIType<'ctx> {
    /// Constructs a new [`DIType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `value` corresponds to a valid
    /// instance of [LLVM `DIType`](https://llvm.org/doxygen/classllvm_1_1DIType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        let metadata_ref = unsafe { LLVMValueAsMetadata(value_ref) };
        Self {
            metadata_ref,
            value_ref,
            _marker: PhantomData,
        }
    }

    /// Returns the name of the type.
    pub fn name(&self) -> Option<&CStr> {
        let mut len = 0;
        // `LLVMDITypeGetName` doesn't allocate any memory, it just returns
        // a pointer to the string which is already a part of `DIType`:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/lib/IR/DebugInfo.cpp#L1489-L1493
        //
        // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
        // gets freed when calling `LLVMDisposeDIBuilder`. Example:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/tools/llvm-c-test/debuginfo.c#L249-L255
        let ptr = unsafe { LLVMDITypeGetName(self.metadata_ref, &mut len) };
        NonNull::new(ptr as *mut _).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
    }

    /// Returns the flags associated with the type.
    pub fn flags(&self) -> LLVMDIFlags {
        unsafe { LLVMDITypeGetFlags(self.metadata_ref) }
    }

    /// Returns the offset of the type in bits. This offset is used in case the
    /// type is a member of a composite type.
    pub fn offset_in_bits(&self) -> usize {
        unsafe { LLVMDITypeGetOffsetInBits(self.metadata_ref) as usize }
    }

    /// Returns the line number in the source code where the type is defined.
    pub fn line(&self) -> u32 {
        unsafe { LLVMDITypeGetLine(self.metadata_ref) }
    }

    /// Replaces the name of the type with a new name.
    ///
    /// # Errors
    ///
    /// Returns a `NulError` if the new name contains a NUL byte, as it cannot
    /// be converted into a `CString`.
    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        super::ir::replace_name(self.value_ref, context, DITypeOperand::Name as u32, name)
    }
}

impl<'ctx> TryFrom<DIDerivedType<'ctx>> for DIType<'ctx> {
    type Error = ();

    fn try_from(di_derived_type: DIDerivedType) -> Result<Self, Self::Error> {
        // FIXME: Perform a check
        Ok(unsafe { Self::from_value_ref(di_derived_type.value_ref) })
    }
}

impl<'ctx> TryFrom<DICompositeType<'ctx>> for DIType<'ctx> {
    type Error = ();

    fn try_from(di_composite_type: DICompositeType) -> Result<Self, Self::Error> {
        // FIXME: Perform a check
        Ok(unsafe { Self::from_value_ref(di_composite_type.value_ref) })
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
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DIDerivedType<'ctx> {
    /// Constructs a new [`DIDerivedType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DIDerivedType`](https://llvm.org/doxygen/classllvm_1_1DIDerivedType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    pub fn as_node(&self) -> DINode<'ctx> {
        DINode {
            value_ref: self.value_ref,
            _marker: PhantomData,
        }
    }

    pub fn as_type(&self) -> DIType<'ctx> {
        let value_ref = self.value_ref;
        let metadata_ref = unsafe { LLVMValueAsMetadata(value_ref) };
        DIType {
            value_ref,
            metadata_ref,
            _marker: PhantomData,
        }
    }

    /// Returns the base type of this derived type.
    pub fn base_type(&self) -> Metadata {
        unsafe {
            let value = LLVMGetOperand(self.value_ref, DIDerivedTypeOperand::BaseType as u32);
            Metadata::from_value_ref(value)
        }
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
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DICompositeType<'ctx> {
    /// Constructs a new [`DICompositeType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DICompositeType`](https://llvm.org/doxygen/classllvm_1_1DICompositeType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    pub fn as_node(&self) -> DINode<'ctx> {
        DINode {
            value_ref: self.value_ref,
            _marker: PhantomData,
        }
    }

    pub fn as_scope(&self) -> DIScope<'ctx> {
        DIScope {
            value_ref: self.value_ref,
            _marker: PhantomData,
        }
    }

    pub fn as_type(&self) -> DIType<'ctx> {
        let value_ref = self.value_ref;
        let metadata_ref = unsafe { LLVMValueAsMetadata(self.value_ref) };
        DIType {
            value_ref,
            metadata_ref,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over elements (struct fields, enum variants, etc.)
    /// of the composite type.
    pub fn elements(&self) -> impl Iterator<Item = Metadata> {
        let elements =
            unsafe { LLVMGetOperand(self.value_ref, DICompositeTypeOperand::Elements as u32) };
        let operands = unsafe { LLVMGetNumOperands(elements) };

        (0..operands)
            .map(move |i| unsafe { Metadata::from_value_ref(LLVMGetOperand(elements, i as u32)) })
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
                LLVMValueAsMetadata(mdnode.value_ref),
            )
        }
    }
}

/// Represents the operands for a [`DISubprogram`]. The enum values correspond
/// to the operand indices within metadata nodes.
#[repr(u32)]
enum DISubprogramOperand {
    Name = 2,
}

/// Represents the debug information for a subprogram (function) in LLVM IR.
pub struct DISubprogram<'ctx> {
    pub(super) value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl<'ctx> DISubprogram<'ctx> {
    /// Constructs a new [`DISubprogram`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DISubprogram`](https://llvm.org/doxygen/classllvm_1_1DISubprogram.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        DISubprogram {
            value_ref,
            _marker: PhantomData,
        }
    }

    /// Returns the name of the subprogram.
    pub fn name(&self) -> Option<&CStr> {
        let operand = unsafe { LLVMGetOperand(self.value_ref, DISubprogramOperand::Name as u32) };
        let mut len = 0;
        // `LLVMGetMDString` doesn't allocate any memory, it just returns a
        // pointer to the string which is already a part of the `Metadata`
        // representing the operand:
        // https://github.com/llvm/llvm-project/blob/cd6022916bff1d6fab007b554810b631549ba43c/llvm/lib/IR/Core.cpp#L1257-L1265
        //
        // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
        // gets freed when calling `LLVMDisposeDIBuilder`.
        let ptr = unsafe { LLVMGetMDString(operand, &mut len) };
        (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) })
    }

    /// Replaces the name of the subprogram with a new name.
    ///
    /// # Errors
    ///
    /// Returns a `NulError` if the new name contains a NUL byte, as it cannot
    /// be converted into a `CString`.
    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        super::ir::replace_name(
            self.value_ref,
            context,
            DISubprogramOperand::Name as u32,
            name,
        )
    }
}
