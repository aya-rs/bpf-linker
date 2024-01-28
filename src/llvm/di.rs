use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    hash::Hasher,
};

use gimli::{DW_TAG_pointer_type, DW_TAG_structure_type, DW_TAG_variant_part};
use llvm_sys::{core::*, debuginfo::*, prelude::*};
use log::{trace, warn};

use super::types::{
    di::DIType,
    ir::{MDNode, Metadata, Value},
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
    visited_nodes: HashSet<u64>,
    item_stack: Vec<Item>,
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
    pub fn new(context: LLVMContextRef, module: LLVMModuleRef) -> DISanitizer {
        DISanitizer {
            context,
            module,
            builder: unsafe { LLVMCreateDIBuilder(module) },
            visited_nodes: HashSet::new(),
            item_stack: Vec::new(),
        }
    }

    fn visit_mdnode(&mut self, mdnode: MDNode) {
        match mdnode.try_into().expect("MDNode is not Metadata") {
            Metadata::DICompositeType(mut di_composite_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_composite_type.tag() {
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
                                    match di_composite_type_inner.tag() {
                                        DW_TAG_variant_part => {
                                            let line = di_composite_type.line();
                                            let file = di_composite_type.file();
                                            let filename = file.filename();

                                            let name = match di_composite_type.name() {
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
                                            if let Some(base_type_name) =
                                                base_type_di_composite_type.name()
                                            {
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
                        if is_data_carrying_enum {
                            di_composite_type.replace_elements(MDNode::empty(self.context));
                        } else if !members.is_empty() {
                            members.sort_by_cached_key(|di_type| di_type.offset_in_bits());
                            let sorted_elements =
                                MDNode::with_elements(self.context, members.as_mut_slice());
                            di_composite_type.replace_elements(sorted_elements);
                        }
                        if remove_name {
                            di_composite_type.replace_name(self.context, "").unwrap();
                        }
                    }
                    _ => (),
                }
            }
            Metadata::DIDerivedType(mut di_derived_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_derived_type.tag() {
                    DW_TAG_pointer_type => {
                        // remove rust names
                        di_derived_type.replace_name(self.context, "").unwrap();
                    }
                    _ => (),
                }
            }
            Metadata::DISubprogram(mut di_subprogram) => {
                // Sanitize function names
                if let Some(name) = di_subprogram.name() {
                    let name = sanitize_type_name(name);
                    di_subprogram
                        .replace_name(self.context, name.as_str())
                        .unwrap();
                }
            }
            _ => (),
        }
    }

    // navigate the tree of LLVMValueRefs (DFS-pre-order)
    fn visit_item(&mut self, item: Item, depth: usize) {
        let value_ref = item.value_ref();
        let value_id = item.value_id();

        let log_prefix = "";
        let log_depth = depth * 4;
        trace!(
            "{log_prefix:log_depth$}visiting item: {item:?} id: {} value: {value_ref:?}",
            item.value_id(),
        );

        let value = match (value_ref, &item) {
            // An operand with no value is valid and means that the operand is
            // not set
            (v, Item::Operand { .. }) if v.is_null() => return,
            (v, _) if !v.is_null() => Value::new(v),
            // All other items should have values
            (_, item) => panic!("{item:?} has no value"),
        };

        let first_visit = self.visited_nodes.insert(value_id);
        if !first_visit {
            trace!("{log_prefix:log_depth$}already visited");
            return;
        }

        self.item_stack.push(item.clone());

        if let Value::MDNode(mdnode) = value.clone() {
            self.visit_mdnode(mdnode)
        }

        if let Some(operands) = value.operands() {
            for (index, operand) in operands.enumerate() {
                self.visit_item(
                    Item::Operand(Operand {
                        parent: value_ref,
                        value: operand,
                        index: index as u32,
                    }),
                    depth + 1,
                )
            }
        }

        if let Some(entries) = value.metadata_entries() {
            for (index, (metadata, kind)) in entries.iter().enumerate() {
                let metadata_value = unsafe { LLVMMetadataAsValue(self.context, metadata) };
                self.visit_item(Item::MetadataEntry(metadata_value, kind, index), depth + 1);
            }
        }

        // If an item has sub items that are not operands nor metadata entries, we need to visit
        // those too.
        if let Value::Function(fun) = value {
            for param in fun.params() {
                self.visit_item(Item::FunctionParam(param), depth + 1);
            }

            for basic_block in fun.basic_blocks() {
                for instruction in basic_block.instructions_iter() {
                    self.visit_item(Item::Instruction(instruction), depth + 1);
                }
            }
        }

        let _ = self.item_stack.pop().unwrap();
    }

    pub fn run(mut self) {
        let module = self.module;

        for value in module.globals_iter() {
            self.visit_item(Item::GlobalVariable(value), 0);
        }
        for value in module.global_aliases_iter() {
            self.visit_item(Item::GlobalAlias(value), 0);
        }

        for function in module.functions_iter() {
            self.visit_item(Item::Function(function), 0);
        }

        unsafe { LLVMDisposeDIBuilder(self.builder) };
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Item {
    GlobalVariable(LLVMValueRef),
    GlobalAlias(LLVMValueRef),
    Function(LLVMValueRef),
    FunctionParam(LLVMValueRef),
    Instruction(LLVMValueRef),
    Operand(Operand),
    MetadataEntry(LLVMValueRef, u32, usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Operand {
    parent: LLVMValueRef,
    value: LLVMValueRef,
    index: u32,
}

impl Item {
    fn value_ref(&self) -> LLVMValueRef {
        match self {
            Item::GlobalVariable(value)
            | Item::GlobalAlias(value)
            | Item::Function(value)
            | Item::FunctionParam(value)
            | Item::Instruction(value)
            | Item::Operand(Operand { value, .. })
            | Item::MetadataEntry(value, _, _) => *value,
        }
    }

    fn value_id(&self) -> u64 {
        self.value_ref() as u64
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
