use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    ffi::CStr,
    hash::Hasher,
};

use gimli::{DW_TAG_pointer_type, DW_TAG_structure_type, DW_TAG_variant_part};
use llvm_sys::{core::*, debuginfo::*, prelude::*};
use log::{trace, warn};

use super::{
    symbol_name,
    types::{
        di::DIType,
        ir::{MDNode, Metadata, Value},
    },
    Message,
};
use crate::llvm::iter::*;

// KSYM_NAME_LEN from linux kernel intentionally set
// to lower value found accross kernel versions to ensure
// backward compatibility
const MAX_KSYM_NAME_LEN: usize = 128;

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
        match mdnode.try_into().expect("MDNode is not Metadata") {
            Metadata::DICompositeType(mut di_composite_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_composite_type.as_node().tag() {
                    DW_TAG_structure_type => {
                        if let Some(name) = di_composite_type.as_type().name() {
                            let name = name.to_string_lossy();
                            // Clear the name from generics.
                            let name = sanitize_type_name(name);
                            di_composite_type
                                .as_type()
                                .replace_name(self.context, name.as_str())
                                .unwrap();
                        }

                        // This is a forward declaration. We don't need to do
                        // anything on the declaration, we're going to process
                        // the actual definition.
                        if di_composite_type.as_type().flags() == LLVMDIFlagFwdDecl {
                            return;
                        }

                        let mut is_data_carrying_enum = false;
                        let mut remove_name = false;
                        let mut members: Vec<DIType> = Vec::new();
                        for element in di_composite_type.elements() {
                            match element {
                                Metadata::DICompositeType(di_composite_type_inner) => {
                                    // The presence of a composite type with `DW_TAG_variant_part`
                                    // as a member of another composite type means that we are
                                    // processing a data-carrying enum. Such types are not supported
                                    // by the Linux kernel. We need to remove the children, so BTF
                                    // doesn't contain data carried by the enum variant.
                                    match di_composite_type_inner.as_node().tag() {
                                        DW_TAG_variant_part => {
                                            let line = di_composite_type.as_type().line();
                                            let scope = di_composite_type.as_scope();
                                            let file = scope.file();
                                            let filename = file.filename();

                                            let name = match di_composite_type.as_type().name() {
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
                                                "found data carrying enum {name} ({filename}:{line}), not emitting
                                                the debug info for it"
                                            );

                                            is_data_carrying_enum = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                Metadata::DIDerivedType(di_derived_type) => {
                                    let base_type = di_derived_type.base_type();

                                    match base_type {
                                        Metadata::DICompositeType(base_type_di_composite_type) => {
                                            let base_type = base_type_di_composite_type.as_type();
                                            let base_type_name = base_type.name();
                                            if let Some(base_type_name) = base_type_name {
                                                let base_type_name =
                                                    base_type_name.to_string_lossy();
                                                // `AyaBtfMapMarker` is a type which is used in fields of BTF map
                                                // structs. We need to make such structs anonymous in order to get
                                                // BTF maps accepted by the Linux kernel.
                                                if base_type_name == "AyaBtfMapMarker" {
                                                    // Remove the name from the struct.
                                                    remove_name = true;
                                                    // And don't include the field in the sanitized DI.
                                                } else {
                                                    members.push(di_derived_type.into());
                                                }
                                            } else {
                                                members.push(di_derived_type.into());
                                            }
                                        }
                                        _ => {
                                            members.push(di_derived_type.into());
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if members.is_empty() {
                            members.sort_by_cached_key(|di_type| di_type.offset_in_bits());
                            let sorted_elements =
                                MDNode::with_elements(self.context, members.as_mut_slice());
                            di_composite_type.replace_elements(sorted_elements);
                        }
                        if is_data_carrying_enum {
                            di_composite_type.replace_elements(MDNode::empty(self.context));
                        }
                        if remove_name {
                            di_composite_type
                                .as_type()
                                .replace_name(self.context, "")
                                .unwrap();
                        }
                    }
                    _ => (),
                }
            }
            Metadata::DIDerivedType(di_derived_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_derived_type.as_node().tag() {
                    DW_TAG_pointer_type => {
                        // remove rust names
                        di_derived_type
                            .as_type()
                            .replace_name(self.context, "")
                            .unwrap();
                    }
                    _ => (),
                }
            }
            // Sanitize function (subprogram) names.
            Metadata::DISubprogram(mut di_subprogram) => {
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

        if let Value::MDNode(mdnode) = Value::new(value) {
            let metadata_kind = LLVMGetMetadataKind(mdnode.metadata());
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
