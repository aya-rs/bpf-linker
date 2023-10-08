use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    ffi::{CStr, CString, NulError},
    hash::Hasher,
    ptr::NonNull,
};

use gimli::{constants::DwTag, DW_TAG_pointer_type, DW_TAG_structure_type, DW_TAG_variant_part};
use llvm_sys::{core::*, debuginfo::*, prelude::*};
use log::{trace, warn};

use super::{
    ir::{MDNode, Metadata, MetadataKind, Value, ValueType},
    symbol_name, Message,
};
use crate::llvm::iter::*;

// KSYM_NAME_LEN from linux kernel intentionally set
// to lower value found accross kernel versions to ensure
// backward compatibility
const MAX_KSYM_NAME_LEN: usize = 128;

pub struct DINode {
    pub md_node: MDNode,
}

impl DINode {
    /// Constructs a new [`DINode`] from the given `metadata`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `metadata` corresponds to a valid
    /// instance of [LLVM `DINode`](https://llvm.org/doxygen/classllvm_1_1DINode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Self {
        let md_node = MDNode::from_metadata_ref(context, metadata);
        Self { md_node }
    }

    /// Constructs a new [`DINode`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DINode`](https://llvm.org/doxygen/classllvm_1_1DINode.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let md_node = MDNode::from_value_ref(value);
        Self { md_node }
    }

    pub fn tag(&self) -> DwTag {
        DwTag(unsafe { LLVMGetDINodeTag(self.md_node.metadata.metadata) })
    }
}

pub struct DIScope {
    pub di_node: DINode,
}

impl DIScope {
    /// Constructs a new [`DIScope`] from the given `metadata`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `metadata` corresponds to a valid
    /// instance of [LLVM `DIScope`](https://llvm.org/doxygen/classllvm_1_1DIScope.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Self {
        let di_node = DINode::from_metadata_ref(context, metadata);
        DIScope { di_node }
    }

    /// Constructs a new [`DIScope`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `value` corresponds to a valid
    /// instance of [LLVM `DIScope`](https://llvm.org/doxygen/classllvm_1_1DIScope.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_node = DINode::from_value_ref(value);
        Self { di_node }
    }

    pub fn file(&self, context: LLVMContextRef) -> DIFile {
        unsafe {
            let metadata = LLVMDIScopeGetFile(self.di_node.md_node.metadata.metadata);
            DIFile::from_metadata_ref(context, metadata)
        }
    }
}

pub struct DIFile {
    pub di_scope: DIScope,
}

impl DIFile {
    /// Constructs a new [`DIFile`] from the given `metadata`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `metadata` corresponds to a valid
    /// instance of [LLVM `DIFile`](https://llvm.org/doxygen/classllvm_1_1DIFile.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_metadata_ref(
        context: LLVMContextRef,
        metadata: LLVMMetadataRef,
    ) -> Self {
        let di_scope = DIScope::from_metadata_ref(context, metadata);
        Self { di_scope }
    }

    pub fn filename(&self) -> Option<&CStr> {
        let mut len = 0;
        // `LLVMDIFileGetName` doesn't allocate any memory, it just returns
        // a pointer to the string which is already a part of `DIFile`:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/lib/IR/DebugInfo.cpp#L1175-L1179
        //
        // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
        // gets freed when calling `LLVMDisposeDIBuilder`.
        let ptr = unsafe {
            LLVMDIFileGetFilename(self.di_scope.di_node.md_node.metadata.metadata, &mut len)
        };
        NonNull::new(ptr as *mut _).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
    }
}

#[repr(u32)]
enum DITypeOperand {
    /// Name of the type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L743)
    /// (`DIComppsiteType` inherits the `getName()` method from `DIType`).
    Name = 2,
}

pub struct DIType {
    pub di_scope: DIScope,
}

impl DIType {
    /// Constructs a new [`DIType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the given `value` corresponds to a valid
    /// instance of [LLVM `DIType`](https://llvm.org/doxygen/classllvm_1_1DIType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_scope = DIScope::from_value_ref(value);
        Self { di_scope }
    }

    pub fn name(&self) -> Option<&CStr> {
        let mut len = 0;
        // `LLVMDITypeGetName` doesn't allocate any memory, it just returns
        // a pointer to the string which is already a part of `DIType`:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/lib/IR/DebugInfo.cpp#L1489-L1493
        //
        // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
        // gets freed when calling `LLVMDisposeDIBuilder`. Example:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/tools/llvm-c-test/debuginfo.c#L249-L255
        let ptr =
            unsafe { LLVMDITypeGetName(self.di_scope.di_node.md_node.metadata.metadata, &mut len) };
        NonNull::new(ptr as *mut _).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
    }

    pub fn flags(&self) -> LLVMDIFlags {
        unsafe { LLVMDITypeGetFlags(self.di_scope.di_node.md_node.metadata.metadata) }
    }

    pub fn offset_in_bits(&self) -> usize {
        unsafe {
            LLVMDITypeGetOffsetInBits(self.di_scope.di_node.md_node.metadata.metadata) as usize
        }
    }

    pub fn line(&self) -> u32 {
        unsafe { LLVMDITypeGetLine(self.di_scope.di_node.md_node.metadata.metadata) }
    }

    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        unsafe {
            let name = LLVMMDStringInContext2(context, CString::new(name)?.as_ptr(), name.len());
            LLVMReplaceMDNodeOperandWith(
                self.di_scope.di_node.md_node.metadata.value,
                DITypeOperand::Name as u32,
                name,
            )
        }
        Ok(())
    }
}

#[repr(u32)]
enum DIDerivedTypeOperand {
    /// [`DIType`] representing a base type of the given derived type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L1032).
    BaseType = 3,
}

pub struct DIDerivedType {
    di_type: DIType,
}

impl DIDerivedType {
    /// Constructs a new [`DIDerivedType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DIDerivedType`](https://llvm.org/doxygen/classllvm_1_1DIDerivedType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_type = DIType::from_value_ref(value);
        Self { di_type }
    }

    pub fn base_type(&self) -> Metadata {
        unsafe {
            let value = LLVMGetOperand(
                self.di_type.di_scope.di_node.md_node.metadata.value,
                DIDerivedTypeOperand::BaseType as u32,
            );
            Metadata::from_value_ref(value)
        }
    }

    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        self.di_type.replace_name(context, name)
    }
}

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
pub struct DICompositeType {
    di_type: DIType,
}

impl DICompositeType {
    /// Constructs a new [`DICompositeType`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DICompositeType`](https://llvm.org/doxygen/classllvm_1_1DICompositeType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_type = DIType::from_value_ref(value);
        Self { di_type }
    }

    pub fn name(&self) -> Option<&CStr> {
        self.di_type.name()
    }

    pub fn flags(&self) -> LLVMDIFlags {
        self.di_type.flags()
    }

    pub fn elements(&self) -> impl Iterator<Item = Metadata> {
        let elements = unsafe {
            LLVMGetOperand(
                self.di_type.di_scope.di_node.md_node.metadata.value,
                DICompositeTypeOperand::Elements as u32,
            )
        };
        let operands = unsafe { LLVMGetNumOperands(elements) };

        (0..operands)
            .map(move |i| unsafe { Metadata::from_value_ref(LLVMGetOperand(elements, i as u32)) })
    }

    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        self.di_type.replace_name(context, name)
    }

    pub fn replace_elements(&mut self, mdnode: MDNode) {
        let value = self.di_type.di_scope.di_node.md_node.metadata.value;
        unsafe {
            LLVMReplaceMDNodeOperandWith(
                value,
                DICompositeTypeOperand::Elements as u32,
                mdnode.metadata.metadata,
            )
        }
    }
}

/// Represents the debug information for a variable in LLVM IR.
pub struct DIVariable {
    pub di_node: DINode,
}

impl DIVariable {
    /// Constructs a new [`DIVariable`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DIVariable`](https://llvm.org/doxygen/classllvm_1_1DIVariable.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_node = DINode::from_value_ref(value);
        Self { di_node }
    }
}

/// Represents the debug information for a global variable in LLVM IR.
pub struct DIGlobalVariable {
    pub di_variable: DIVariable,
}

impl DIGlobalVariable {
    /// Constructs a new [`DIGlobalVariable`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DIGlobalVariable`](https://llvm.org/doxygen/classllvm_1_1DIGlobalVariable.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_variable = DIVariable::from_value_ref(value);
        Self { di_variable }
    }
}

/// Represents the debug information for a common block in LLVM IR.
pub struct DICommonBlock {
    pub di_scope: DIScope,
}

impl DICommonBlock {
    /// Constructs a new [`DICommonBlock`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DICommonBlack`](https://llvm.org/doxygen/classllvm_1_1DICommonBlock.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_scope = DIScope::from_value_ref(value);
        Self { di_scope }
    }
}

/// Represents the debug information for a local scope in LLVM IR.
pub struct DILocalScope {
    pub di_scope: DIScope,
}

impl DILocalScope {
    /// Constructs a new [`DILocalScope`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DILocalScope`](https://llvm.org/doxygen/classllvm_1_1DILocalScope.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_scope = DIScope::from_value_ref(value);
        Self { di_scope }
    }
}

/// Represents the operands for a [`DISubprogram`]. The enum values correspond
/// to the operand indices within metadata nodes.
#[repr(u32)]
enum DISubprogramOperand {
    Name = 2,
}

/// Represents the debug information for a subprogram (function) in LLVM IR.
pub struct DISubprogram {
    pub di_local_scope: DILocalScope,
}

impl DISubprogram {
    /// Constructs a new [`DISubprogram`] from the given `value`.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided `value` corresponds to a valid
    /// instance of [LLVM `DISubprogram`](https://llvm.org/doxygen/classllvm_1_1DISubprogram.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub(crate) unsafe fn from_value_ref(value: LLVMValueRef) -> Self {
        let di_local_scope = DILocalScope::from_value_ref(value);
        DISubprogram { di_local_scope }
    }

    pub fn name(&self) -> Option<&CStr> {
        let value = self.di_local_scope.di_scope.di_node.md_node.metadata.value;
        let operand = unsafe { LLVMGetOperand(value, DISubprogramOperand::Name as u32) };
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
        let value = self.di_local_scope.di_scope.di_node.md_node.metadata.value;
        let name =
            unsafe { LLVMMDStringInContext2(context, CString::new(name)?.as_ptr(), name.len()) };
        unsafe { LLVMReplaceMDNodeOperandWith(value, DISubprogramOperand::Name as u32, name) };
        Ok(())
    }
}

pub struct DISanitizer {
    context: LLVMContextRef,
    module: LLVMModuleRef,
    builder: LLVMDIBuilderRef,
    cache: Cache,
    node_stack: Vec<LLVMValueRef>,
}

// Sanitize Rust type names to be valid C type names.
fn sanitize_type_name<T: AsRef<str>>(name: T) -> String {
    let n: String = name
        .as_ref()
        .chars()
        .map(|ch| {
            // Characters which are valid in C type names (alphanumeric and `_`).
            if matches!(ch, '0'..='9' | 'A'..='Z' | 'a'..='z' | '_') {
                ch.to_string()
            } else {
                format!("_{:X}_", ch as u32)
            }
        })
        .collect();

    // we trim type name if it is too long
    if n.len() > MAX_KSYM_NAME_LEN {
        let mut hasher = DefaultHasher::new();
        hasher.write(n.as_bytes());
        let hash = format!("{:x}", hasher.finish());
        // leave space for underscore
        let trim = MAX_KSYM_NAME_LEN - hash.len() - 1;
        return format!("{}_{hash}", &n[..trim]);
    }

    n
}

impl DISanitizer {
    pub unsafe fn new(context: LLVMContextRef, module: LLVMModuleRef) -> DISanitizer {
        DISanitizer {
            context,
            module,
            builder: LLVMCreateDIBuilder(module),
            cache: Cache::new(),
            node_stack: Vec::new(),
        }
    }

    fn mdnode(&mut self, mdnode: MDNode) {
        match mdnode.metadata.to_metadata_kind() {
            MetadataKind::DICompositeType(mut di_composite_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_composite_type.di_type.di_scope.di_node.tag() {
                    DW_TAG_structure_type => {
                        if let Some(name) = di_composite_type.name() {
                            let name = name.to_string_lossy();
                            // Clear the name from generics.
                            let name = sanitize_type_name(name);
                            di_composite_type
                                .replace_name(self.context, name.as_str())
                                .unwrap();
                        }

                        // This is a forward declaration. We don't need to do
                        // anything on the declaration, we're going to process
                        // the actual definition.
                        if di_composite_type.flags() == LLVMDIFlagFwdDecl {
                            return;
                        }

                        // variadic enum not supported => emit warning and strip out the children array
                        // i.e. pub enum Foo { Bar, Baz(u32), Bad(u64, u64) }

                        // we detect this is a variadic enum if the child element is a DW_TAG_variant_part
                        let mut members = Vec::new();
                        for element in di_composite_type.elements() {
                            match element.to_metadata_kind() {
                                MetadataKind::DICompositeType(mut di_composite_type) => {
                                    // The presence of `DW_TAG_variant_part` in a composite type
                                    // means that we are processing a data-carrying enum. Such
                                    // type is not supported by the Linux kernel, so we need to
                                    // remove the children, so BTF doesn't contain data carried
                                    // by the enum variant.
                                    match di_composite_type.di_type.di_scope.di_node.tag() {
                                        DW_TAG_variant_part => {
                                            let line = di_composite_type.di_type.line();
                                            let file = di_composite_type
                                                .di_type
                                                .di_scope
                                                .file(self.context);
                                            let filename = file.filename();

                                            let name = match di_composite_type.di_type.name() {
                                                Some(name) => name.to_string_lossy().to_string(),
                                                None => "(anon)".to_owned(),
                                            };
                                            let filename = match filename {
                                                Some(filename) => {
                                                    filename.to_string_lossy().to_string()
                                                }
                                                None => "<unknown>".to_owned(),
                                            };

                                            warn!(
                                                "at {}:{}: enum {}: not emitting BTF",
                                                filename, line, name
                                            );

                                            // Remove children.
                                            // TODO(vadorovsky): We might be leaking memory here,
                                            // let's double-check if we can dispose the children.
                                            di_composite_type
                                                .replace_elements(MDNode::empty(self.context));
                                            // Remove name.
                                            di_composite_type
                                                .replace_name(self.context, "")
                                                .unwrap();
                                        }
                                        _ => {}
                                    }
                                }
                                MetadataKind::DIDerivedType(di_derived_type) => {
                                    let base_type = di_derived_type.base_type();

                                    match base_type.to_metadata_kind() {
                                        MetadataKind::DICompositeType(
                                            base_type_di_composite_type,
                                        ) => {
                                            let base_type_name = base_type_di_composite_type.name();
                                            if let Some(base_type_name) = base_type_name {
                                                let base_type_name =
                                                    base_type_name.to_string_lossy();
                                                // `AyaBtfMapMarker` is a type which is used in fields of BTF map
                                                // structs. We need to make such structs anonymous in order to get
                                                // BTF maps accepted by the Linux kernel.
                                                if base_type_name == "AyaBtfMapMarker" {
                                                    // Remove the name from the struct.
                                                    di_composite_type
                                                        .replace_name(self.context, "")
                                                        .unwrap();
                                                    // And don't include the field in the sanitized DI.
                                                } else {
                                                    members.push(di_derived_type.di_type);
                                                }
                                            } else {
                                                members.push(di_derived_type.di_type);
                                            }
                                        }
                                        _ => {
                                            members.push(di_derived_type.di_type);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !members.is_empty() {
                            members.sort_by_cached_key(|di_type| di_type.offset_in_bits());
                            let sorted_elements =
                                MDNode::with_elements(self.context, members.as_mut_slice());
                            di_composite_type.replace_elements(sorted_elements);
                        }
                    }
                    _ => (),
                }
            }
            MetadataKind::DIDerivedType(mut di_derived_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_derived_type.di_type.di_scope.di_node.tag() {
                    DW_TAG_pointer_type => {
                        // remove rust names
                        di_derived_type.replace_name(self.context, "").unwrap();
                    }
                    _ => (),
                }
            }
            // Sanitize function (subprogram) names.
            MetadataKind::DISubprogram(mut di_subprogram) => {
                if let Some(name) = di_subprogram.name() {
                    let name = sanitize_type_name(name.to_string_lossy());
                    di_subprogram
                        .replace_name(self.context, name.as_str())
                        .unwrap();
                }
            }
            _ => (),
        }
    }

    // navigate the tree of LLVMValueRefs (DFS-pre-order)
    unsafe fn discover(&mut self, value: LLVMValueRef, depth: usize) {
        let one = "    ";

        if value.is_null() {
            trace!("{one:depth$}skipping null node");
            return;
        }

        // TODO: doing this on the pointer value is not good
        let key = if is_mdnode(value) {
            LLVMValueAsMetadata(value) as u64
        } else {
            value as u64
        };
        if self.cache.hit(key) {
            trace!("{one:depth$}skipping already visited node");
            return;
        }

        self.node_stack.push(value);

        if let ValueType::MDNode(mdnode) = Value::new(value).to_value_type() {
            let metadata_kind = LLVMGetMetadataKind(mdnode.metadata.metadata);
            trace!(
                "{one:depth$}mdnode kind:{:?} n_operands:{} value: {}",
                metadata_kind,
                LLVMGetMDNodeNumOperands(value),
                Message {
                    ptr: LLVMPrintValueToString(value)
                }
                .as_c_str()
                .unwrap()
                .to_str()
                .unwrap()
            );

            self.mdnode(mdnode)
        } else {
            trace!(
                "{one:depth$}node value: {}",
                Message {
                    ptr: LLVMPrintValueToString(value)
                }
                .as_c_str()
                .unwrap()
                .to_str()
                .unwrap()
            );
        }

        if can_get_all_metadata(value) {
            for (index, (kind, metadata)) in iter_metadata_copy(value).enumerate() {
                let metadata_value = LLVMMetadataAsValue(self.context, metadata);
                trace!("{one:depth$}all_metadata entry: index:{}", index);
                self.discover(metadata_value, depth + 1);

                if is_instruction(value) {
                    LLVMSetMetadata(value, kind, metadata_value);
                } else {
                    LLVMGlobalSetMetadata(value, kind, metadata);
                }
            }
        }

        if can_get_operands(value) {
            for (index, operand) in iter_operands(value).enumerate() {
                trace!(
                    "{one:depth$}operand index:{} name:{} value:{}",
                    index,
                    symbol_name(value),
                    Message {
                        ptr: LLVMPrintValueToString(value)
                    }
                    .as_c_str()
                    .unwrap()
                    .to_str()
                    .unwrap()
                );
                self.discover(operand, depth + 1)
            }
        }

        assert_eq!(self.node_stack.pop(), Some(value));
    }

    pub unsafe fn run(&mut self) {
        for sym in self.module.named_metadata_iter() {
            let mut len: usize = 0;
            let name = CStr::from_ptr(LLVMGetNamedMetadataName(sym, &mut len))
                .to_str()
                .unwrap();
            // just for debugging, we are not visiting those nodes for the moment
            trace!("named metadata name:{}", name);
        }

        let module = self.module;
        for (i, sym) in module.globals_iter().enumerate() {
            trace!("global index:{} name:{}", i, symbol_name(sym));
            self.discover(sym, 0);
        }

        for (i, sym) in module.global_aliases_iter().enumerate() {
            trace!("global aliases index:{} name:{}", i, symbol_name(sym));
            self.discover(sym, 0);
        }

        for function in module.functions_iter() {
            trace!("function > name:{}", symbol_name(function));
            self.discover(function, 0);

            let params_count = LLVMCountParams(function);
            for i in 0..params_count {
                let param = LLVMGetParam(function, i);
                trace!("function param name:{} index:{}", symbol_name(param), i);
                self.discover(param, 1);
            }

            for basic_block in function.basic_blocks_iter() {
                trace!("function block");
                for instruction in basic_block.instructions_iter() {
                    let n_operands = LLVMGetNumOperands(instruction);
                    trace!("function block instruction num_operands: {}", n_operands);
                    for index in 0..n_operands {
                        let operand = LLVMGetOperand(instruction, index as u32);
                        if is_instruction(operand) {
                            self.discover(operand, 2);
                        }
                    }

                    self.discover(instruction, 1);
                }
            }
        }

        LLVMDisposeDIBuilder(self.builder);
    }
}

// utils

unsafe fn iter_operands(v: LLVMValueRef) -> impl Iterator<Item = LLVMValueRef> {
    (0..LLVMGetNumOperands(v)).map(move |i| LLVMGetOperand(v, i as u32))
}

unsafe fn iter_metadata_copy(v: LLVMValueRef) -> impl Iterator<Item = (u32, LLVMMetadataRef)> {
    let mut count = 0;
    let entries = LLVMGlobalCopyAllMetadata(v, &mut count);
    (0..count).map(move |index| {
        (
            LLVMValueMetadataEntriesGetKind(entries, index as u32),
            LLVMValueMetadataEntriesGetMetadata(entries, index as u32),
        )
    })
}

unsafe fn is_instruction(v: LLVMValueRef) -> bool {
    !LLVMIsAInstruction(v).is_null()
}

unsafe fn is_mdnode(v: LLVMValueRef) -> bool {
    !LLVMIsAMDNode(v).is_null()
}

unsafe fn is_user(v: LLVMValueRef) -> bool {
    !LLVMIsAUser(v).is_null()
}

unsafe fn is_globalobject(v: LLVMValueRef) -> bool {
    !LLVMIsAGlobalObject(v).is_null()
}

unsafe fn _is_globalvariable(v: LLVMValueRef) -> bool {
    !LLVMIsAGlobalVariable(v).is_null()
}

unsafe fn _is_function(v: LLVMValueRef) -> bool {
    !LLVMIsAFunction(v).is_null()
}

unsafe fn can_get_all_metadata(v: LLVMValueRef) -> bool {
    is_globalobject(v) || is_instruction(v)
}

unsafe fn can_get_operands(v: LLVMValueRef) -> bool {
    is_mdnode(v) || is_user(v)
}

pub struct Cache {
    keys: HashSet<u64>,
}

impl Cache {
    pub fn new() -> Self {
        Cache {
            keys: HashSet::new(),
        }
    }

    pub fn hit(&mut self, key: u64) -> bool {
        !self.keys.insert(key)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_strip_generics() {
        let name = "MyStruct<u64>";
        assert_eq!(sanitize_type_name(name), "MyStruct_3C_u64_3E_");

        let name = "MyStruct<u64, u64>";
        assert_eq!(sanitize_type_name(name), "MyStruct_3C_u64_2C__20_u64_3E_");

        let name = "my_function<aya_bpf::BpfContext>";
        assert_eq!(
            sanitize_type_name(name),
            "my_function_3C_aya_bpf_3A__3A_BpfContext_3E_"
        );

        let name = "my_function<aya_bpf::BpfContext, aya_log_ebpf::WriteToBuf>";
        assert_eq!(
            sanitize_type_name(name),
            "my_function_3C_aya_bpf_3A__3A_BpfContext_2C__20_aya_log_ebpf_3A__3A_WriteToBuf_3E_"
        );

        let name = "PerfEventArray<[u8; 32]>";
        assert_eq!(
            sanitize_type_name(name),
            "PerfEventArray_3C__5B_u8_3B__20_32_5D__3E_"
        );

        let name = "my_function<aya_bpf::this::is::a::very::long::namespace::BpfContext, aya_log_ebpf::this::is::a::very::long::namespace::WriteToBuf>";
        let san = sanitize_type_name(name);

        assert_eq!(san.len(), 128);
        assert_eq!(
            san,
            "my_function_3C_aya_bpf_3A__3A_this_3A__3A_is_3A__3A_a_3A__3A_very_3A__3A_long_3A__3A_namespace_3A__3A_BpfContex_94e4085604b3142f"
        );
    }
}
