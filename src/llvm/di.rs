use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    ffi::{CStr, CString, NulError},
    hash::Hasher,
    ptr::NonNull,
};

use gimli::{
    constants::DwTag, DW_TAG_member, DW_TAG_pointer_type, DW_TAG_structure_type,
    DW_TAG_variant_part,
};
use llvm_sys::{core::*, debuginfo::*, prelude::*};
use log::*;

use super::{symbol_name, Message};
use crate::llvm::iter::*;

// KSYM_NAME_LEN from linux kernel intentionally set
// to lower value found accross kernel versions to ensure
// backward compatibility
const MAX_KSYM_NAME_LEN: usize = 128;

#[repr(u32)]
enum DITypeOperand {
    /// Name of the type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L743)
    /// (`DIComppsiteType` inherits the `getName()` method from `DIType`).
    Name = 2,
}

pub struct DIType {
    metadata: LLVMMetadataRef,
    value: LLVMValueRef,
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
    pub unsafe fn new(value: LLVMValueRef) -> Self {
        let metadata = LLVMValueAsMetadata(value);
        Self { metadata, value }
    }

    pub fn name(&self) -> Option<&CStr> {
        let mut len = 0;
        // `LLVMDITypeGetName` doesn't allocate any memory, it just returns
        // an existing pointer:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/lib/IR/DebugInfo.cpp#L1489-L1493
        //
        // Therefore, we don't need to call `LLVMDisposeMessage`. The memory
        // gets freed when calling `LLVMDisposeDIBuilder`. Example:
        // https://github.com/llvm/llvm-project/blob/eee1f7cef856241ad7d66b715c584d29b1c89ca9/llvm/tools/llvm-c-test/debuginfo.c#L249-L255
        let ptr = unsafe { LLVMDITypeGetName(self.metadata, &mut len) };
        NonNull::new(ptr as *mut _).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) })
    }

    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        unsafe {
            let name = LLVMMDStringInContext2(context, CString::new(name)?.as_ptr(), name.len());
            LLVMReplaceMDNodeOperandWith(self.value, DITypeOperand::Name as u32, name)
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
    /// instance of [`DIDerivedType`](https://llvm.org/doxygen/classllvm_1_1DIDerivedType.html).
    /// It's the caller's responsibility to ensure this invariant, as this
    /// method doesn't perform any validation checks.
    pub unsafe fn new(value: LLVMValueRef) -> Self {
        let di_type = DIType::new(value);
        Self { di_type }
    }

    pub fn base_type(&self) -> LLVMValueRef {
        unsafe { LLVMGetOperand(self.di_type.value, DIDerivedTypeOperand::BaseType as u32) }
    }
}

#[repr(u32)]
enum DICompositeTypeOperand {
    /// Elements of the composite type.
    /// [Reference in LLVM code](https://github.com/llvm/llvm-project/blob/llvmorg-17.0.3/llvm/include/llvm/IR/DebugInfoMetadata.h#L1230).
    Elements = 4,
}

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
    pub unsafe fn new(value: LLVMValueRef) -> Self {
        let di_type = DIType::new(value);
        Self { di_type }
    }

    pub fn name(&self) -> Option<&CStr> {
        self.di_type.name()
    }

    pub fn elements(&self) -> impl Iterator<Item = LLVMValueRef> {
        let elements =
            unsafe { LLVMGetOperand(self.di_type.value, DICompositeTypeOperand::Elements as u32) };
        let operands = unsafe { LLVMGetNumOperands(elements) };

        (0..operands).map(move |i| unsafe { LLVMGetOperand(elements, i as u32) })
    }

    pub fn replace_name(&mut self, context: LLVMContextRef, name: &str) -> Result<(), NulError> {
        self.di_type.replace_name(context, name)
    }

    pub fn replace_elements(&mut self, metadata: LLVMMetadataRef) {
        unsafe {
            LLVMReplaceMDNodeOperandWith(
                self.di_type.value,
                DICompositeTypeOperand::Elements as u32,
                metadata,
            )
        }
    }
}

pub struct DIFix {
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

impl DIFix {
    pub unsafe fn new(context: LLVMContextRef, module: LLVMModuleRef) -> DIFix {
        DIFix {
            context,
            module,
            builder: LLVMCreateDIBuilder(module),
            cache: Cache::new(),
            node_stack: Vec::new(),
        }
    }

    unsafe fn mdnode(&mut self, value: LLVMValueRef) {
        let metadata = LLVMValueAsMetadata(value);
        let di_type = DIType::new(value);
        let metadata_kind = LLVMGetMetadataKind(metadata);

        let empty = to_mdstring(self.context, "");

        match metadata_kind {
            LLVMMetadataKind::LLVMDICompositeTypeMetadataKind => {
                let mut di_composite_type = DICompositeType::new(value);
                let tag = get_tag(metadata);

                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match tag {
                    DW_TAG_structure_type => {
                        if let Some(name) = di_composite_type.name() {
                            let name = name.to_string_lossy();
                            // Clear the name from generics.
                            let name = sanitize_type_name(name);
                            di_composite_type
                                .replace_name(self.context, name.as_str())
                                .unwrap();
                        }

                        // variadic enum not supported => emit warning and strip out the children array
                        // i.e. pub enum Foo { Bar, Baz(u32), Bad(u64, u64) }

                        let flags = LLVMDITypeGetFlags(metadata);

                        // This is a forward declaration. We don't need to do
                        // anything on the declaration, we're going to process
                        // the actual definition.
                        if flags == LLVMDIFlagFwdDecl {
                            return;
                        }

                        // we detect this is a variadic enum if the child element is a DW_TAG_variant_part
                        let mut members = Vec::new();

                        for (i, element) in di_composite_type.elements().enumerate() {
                            let tag = get_tag(LLVMValueAsMetadata(element));
                            if i == 0 && tag == DW_TAG_variant_part {
                                // TODO: check: the following always returns <unknown>:0 - however its strange...
                                let _line = LLVMDITypeGetLine(LLVMValueAsMetadata(value)); // always returns 0
                                let scope = LLVMDIVariableGetScope(metadata);
                                let file = LLVMDIScopeGetFile(scope);
                                let mut len = 0;
                                let _filename =
                                    CStr::from_ptr(LLVMDIFileGetFilename(file, &mut len)); // still getting <undefined>

                                // FIX: shadowing prev values with "correct" ones, found looking at parent nodes
                                let (filename, line) = self
                                    .node_stack
                                    .iter()
                                    .rev()
                                    .find_map(|v: &LLVMValueRef| -> Option<(&str, u32)> {
                                        let v = *v;
                                        if !is_mdnode(v) {
                                            return None;
                                        }
                                        let m = LLVMValueAsMetadata(v);
                                        let metadata_kind = LLVMGetMetadataKind(m);
                                        let file_operand_index = match metadata_kind {
                                            LLVMMetadataKind::LLVMDIGlobalVariableMetadataKind => {
                                                Some(2)
                                            }
                                            LLVMMetadataKind::LLVMDICommonBlockMetadataKind => {
                                                Some(3)
                                            }
                                            // TODO: add more cases based on asmwriter.cpp
                                            _ => None,
                                        }?;
                                        let file = LLVMGetOperand(v, file_operand_index);
                                        let mut len = 0;
                                        let filename = CStr::from_ptr(LLVMDIFileGetFilename(
                                            LLVMValueAsMetadata(file),
                                            &mut len,
                                        ))
                                        .to_str()
                                        .unwrap();
                                        if filename == "<unknown>" {
                                            return None;
                                        }
                                        // since this node has plausible filename, we also trust the corresponding line
                                        let line = LLVMDITypeGetLine(m);
                                        Some((filename, line))
                                    })
                                    .unwrap_or(("unknown", 0));

                                // finally emit warning
                                match di_composite_type.name() {
                                    Some(name) => warn!(
                                        "not emitting BTF for type {} at {}:{}",
                                        name.to_string_lossy(),
                                        filename,
                                        line
                                    ),
                                    None => {
                                        warn!(
                                            "not emitting BTF for anonymous type at {}:{}",
                                            filename, line
                                        )
                                    }
                                }

                                // strip out children
                                let empty_node =
                                    LLVMMDNodeInContext2(self.context, core::ptr::null_mut(), 0);
                                di_composite_type.replace_elements(empty_node);

                                // remove rust names
                                di_composite_type.replace_name(self.context, "").unwrap();

                                break;
                            }

                            if tag == DW_TAG_member {
                                let member = LLVMValueAsMetadata(element);
                                let di_derived_type = DIDerivedType::new(element);
                                let base_type = di_derived_type.base_type();
                                let base_type_metadata = LLVMValueAsMetadata(base_type);
                                let base_type_metadata_kind =
                                    LLVMGetMetadataKind(base_type_metadata);

                                match base_type_metadata_kind {
                                    LLVMMetadataKind::LLVMDICompositeTypeMetadataKind => {
                                        let base_type_di_composite_type =
                                            DICompositeType::new(base_type);
                                        let base_type_name = base_type_di_composite_type.name();
                                        if let Some(base_type_name) = base_type_name {
                                            let base_type_name = base_type_name.to_string_lossy();
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
                                                members.push(member);
                                            }
                                        } else {
                                            members.push(member);
                                        }
                                    }
                                    _ => {
                                        members.push(member);
                                    }
                                }
                            }
                        }
                        if !members.is_empty() {
                            members.sort_by_cached_key(|metadata| {
                                LLVMDITypeGetOffsetInBits(*metadata)
                            });
                            let md = LLVMMDNodeInContext2(
                                self.context,
                                members.as_mut_ptr(),
                                members.len(),
                            );
                            LLVMReplaceMDNodeOperandWith(value, 4, md);
                        }
                    }
                    _ => (),
                }
            }
            LLVMMetadataKind::LLVMDIDerivedTypeMetadataKind => {
                let tag = get_tag(metadata);

                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match tag {
                    DW_TAG_pointer_type => {
                        // remove rust names
                        LLVMReplaceMDNodeOperandWith(value, 2, empty);
                    }
                    _ => (),
                }
            }
            // Sanitize function (subprogram) names.
            LLVMMetadataKind::LLVMDISubprogramMetadataKind => {
                if let Some(name) = di_type.name() {
                    // Clear the name from generics.
                    let name = sanitize_type_name(name.to_string_lossy());
                    let name = to_mdstring(self.context, &name);
                    LLVMReplaceMDNodeOperandWith(value, 2, name);
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

        if is_mdnode(value) {
            let metadata = LLVMValueAsMetadata(value);
            let metadata_kind = LLVMGetMetadataKind(metadata);

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

            self.mdnode(value)
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
            for (index, (kind, metadata)) in iter_medatada_copy(value).enumerate() {
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

unsafe fn to_mdstring(context: LLVMContextRef, s: &str) -> LLVMMetadataRef {
    LLVMMDStringInContext2(context, s.as_ptr() as _, s.len())
}

unsafe fn iter_operands(v: LLVMValueRef) -> impl Iterator<Item = LLVMValueRef> {
    (0..LLVMGetNumOperands(v)).map(move |i| LLVMGetOperand(v, i as u32))
}

unsafe fn iter_medatada_copy(v: LLVMValueRef) -> impl Iterator<Item = (u32, LLVMMetadataRef)> {
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

unsafe fn get_tag(metadata: LLVMMetadataRef) -> DwTag {
    DwTag(LLVMGetDINodeTag(metadata))
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
