use std::{
    borrow::Cow,
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    ffi::c_char,
    hash::Hasher,
    marker::PhantomData,
    ptr,
};

use gimli::{DW_TAG_pointer_type, DW_TAG_structure_type, DW_TAG_variant_part};
use llvm_sys::{core::*, debuginfo::*, prelude::*};
use tracing::{span, trace, warn, Level};

use super::types::{
    di::DIType,
    ir::{Function, MDNode, Metadata, Value},
};
use crate::llvm::{iter::*, types::di::DISubprogram, LLVMContext, LLVMModule};

// KSYM_NAME_LEN from linux kernel intentionally set
// to lower value found accross kernel versions to ensure
// backward compatibility
const MAX_KSYM_NAME_LEN: usize = 128;

pub struct DISanitizer<'ctx> {
    context: LLVMContextRef,
    module: LLVMModuleRef,
    builder: LLVMDIBuilderRef,
    visited_nodes: HashSet<u64>,
    replace_operands: HashMap<u64, LLVMMetadataRef>,
    skipped_types: Vec<String>,
    _marker: PhantomData<LLVMModule<'ctx>>,
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

impl<'ctx> DISanitizer<'ctx> {
    pub fn new(context: &'ctx LLVMContext, module: &mut LLVMModule<'ctx>) -> DISanitizer<'ctx> {
        DISanitizer {
            context: context.as_mut_ptr(),
            module: module.as_mut_ptr(),
            builder: unsafe { LLVMCreateDIBuilder(module.as_mut_ptr()) },
            visited_nodes: HashSet::new(),
            replace_operands: HashMap::new(),
            skipped_types: Vec::new(),
            _marker: PhantomData,
        }
    }

    fn visit_mdnode(&mut self, mdnode: MDNode) {
        match mdnode.try_into().expect("MDNode is not Metadata") {
            Metadata::DICompositeType(mut di_composite_type) => {
                #[allow(clippy::single_match)]
                #[allow(non_upper_case_globals)]
                match di_composite_type.tag() {
                    DW_TAG_structure_type => {
                        let names = match di_composite_type.name() {
                            Some(name) => {
                                let original_name = name.to_string_lossy().to_string();
                                let sanitized_name = sanitize_type_name(&original_name);

                                Some((original_name, sanitized_name))
                            }
                            None => None,
                        };

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

                                            let name = match names {
                                                Some((ref original_name, _)) => {
                                                    original_name.to_owned()
                                                }
                                                None => "(anon)".to_owned(),
                                            };
                                            let filename = match filename {
                                                Some(filename) => {
                                                    filename.to_string_lossy().to_string()
                                                }
                                                None => "<unknown>".to_owned(),
                                            };

                                            trace!(
                                                "found data carrying enum {name} ({filename}:{line}), not emitting the debug info for it"
                                            );
                                            self.skipped_types.push(name);

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
                            // `AyaBtfMapMarker` is a type which is used in fields of BTF map
                            // structs. We need to make such structs anonymous in order to get
                            // BTF maps accepted by the Linux kernel.
                            di_composite_type.replace_name(self.context, "").unwrap();
                        } else if let Some((_, sanitized_name)) = names {
                            // Clear the name from characters incompatible with C.
                            di_composite_type
                                .replace_name(self.context, sanitized_name.as_str())
                                .unwrap();
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
    fn visit_item(&mut self, mut item: Item) {
        let value_ref = item.value_ref();
        let value_id = item.value_id();

        let item_span = span!(Level::TRACE, "item", value_id);
        let _enter = item_span.enter();
        trace!(?item, value = ?value_ref, "visiting item");

        let value = match (value_ref, &item) {
            // An operand with no value is valid and means that the operand is
            // not set
            (v, Item::Operand { .. }) if v.is_null() => return,
            (v, _) if !v.is_null() => Value::new(v),
            // All other items should have values
            (_, item) => panic!("{item:?} has no value"),
        };

        if let Item::Operand(operand) = &mut item {
            // When we have an operand to replace, we must do so regardless of whether we've already
            // seen its value or not, since the same value can appear as an operand in multiple
            // nodes in the tree.
            if let Some(new_metadata) = self.replace_operands.get(&value_id) {
                operand.replace(unsafe { LLVMMetadataAsValue(self.context, *new_metadata) })
            }
        }

        let first_visit = self.visited_nodes.insert(value_id);
        if !first_visit {
            trace!("already visited");
            return;
        }

        if let Value::MDNode(mdnode) = value.clone() {
            self.visit_mdnode(mdnode)
        }

        if let Some(operands) = value.operands() {
            for (index, operand) in operands.enumerate() {
                self.visit_item(Item::Operand(Operand {
                    parent: value_ref,
                    value: operand,
                    index: index as u32,
                }))
            }
        }

        if let Some(entries) = value.metadata_entries() {
            for (index, (metadata, kind)) in entries.iter().enumerate() {
                let metadata_value = unsafe { LLVMMetadataAsValue(self.context, metadata) };
                self.visit_item(Item::MetadataEntry(metadata_value, kind, index));
            }
        }

        // If an item has sub items that are not operands nor metadata entries, we need to visit
        // those too.
        if let Value::Function(fun) = value {
            for param in fun.params() {
                self.visit_item(Item::FunctionParam(param));
            }

            for basic_block in fun.basic_blocks() {
                for instruction in basic_block.instructions_iter() {
                    self.visit_item(Item::Instruction(instruction));
                }
            }
        }
    }

    pub fn run(mut self, exported_symbols: &HashSet<Cow<'static, str>>) {
        let module = self.module;

        self.replace_operands = self.fix_subprogram_linkage(exported_symbols);

        for value in module.globals_iter() {
            self.visit_item(Item::GlobalVariable(value));
        }
        for value in module.global_aliases_iter() {
            self.visit_item(Item::GlobalAlias(value));
        }

        for function in module.functions_iter() {
            self.visit_item(Item::Function(function));
        }

        if !self.skipped_types.is_empty() {
            warn!(
                "debug info was not emitted for the following types: {}",
                self.skipped_types.join(", ")
            );
        }

        unsafe { LLVMDisposeDIBuilder(self.builder) };
    }

    // Make it so that only exported symbols (programs marked as #[no_mangle]) get BTF
    // linkage=global. For all other functions we want linkage=static. This avoid issues like:
    //
    //     Global function write() doesn't return scalar. Only those are supported.
    //     verification time 18 usec
    //     stack depth 0+0
    //     ...
    //
    // This is an error we used to get compiling aya-log. Global functions are verified
    // independently from their callers, so the verifier has less context and as a result globals
    // are harder to verify successfully.
    //
    // See tests/btf/assembly/exported-symbols.rs .
    fn fix_subprogram_linkage(
        &mut self,
        export_symbols: &HashSet<Cow<'static, str>>,
    ) -> HashMap<u64, LLVMMetadataRef> {
        let mut replace = HashMap::new();

        for mut function in self
            .module
            .functions_iter()
            .map(|value| unsafe { Function::from_value_ref(value) })
        {
            if export_symbols.contains(function.name()) {
                continue;
            }

            // Skip functions that don't have subprograms.
            let Some(mut subprogram) = function.subprogram(self.context) else {
                continue;
            };

            let name = subprogram.name().unwrap();
            let linkage_name = subprogram.linkage_name();
            let ty = subprogram.ty();

            // Create a new subprogram that has DISPFlagLocalToUnit set, so the BTF backend emits it
            // with linkage=static
            let mut new_program = unsafe {
                let new_program = LLVMDIBuilderCreateFunction(
                    self.builder,
                    subprogram.scope().unwrap(),
                    name.as_ptr() as *const c_char,
                    name.len(),
                    linkage_name.map(|s| s.as_ptr()).unwrap_or(ptr::null()) as *const c_char,
                    linkage_name.unwrap_or("").len(),
                    subprogram.file(),
                    subprogram.line(),
                    ty,
                    1,
                    1,
                    subprogram.line(),
                    subprogram.type_flags(),
                    1,
                );
                // Technically this must be called as part of the builder API, but effectively does
                // nothing because we don't add any variables through the builder API, instead we
                // replace retained nodes manually below.
                LLVMDIBuilderFinalizeSubprogram(self.builder, new_program);

                DISubprogram::from_value_ref(LLVMMetadataAsValue(self.context, new_program))
            };

            // Point the function to the new subprogram.
            function.set_subprogram(&new_program);

            // There's no way to set the unit with LLVMDIBuilderCreateFunction
            // so we set it after creation.
            if let Some(unit) = subprogram.unit() {
                new_program.set_unit(unit);
            }

            // Add retained nodes from the old program. This is needed to preserve local debug
            // variables, including function arguments which otherwise become "anon". See
            // LLVMDIBuilderFinalizeSubprogram and DISubprogram::replaceRetainedNodes.
            if let Some(retained_nodes) = subprogram.retained_nodes() {
                new_program.set_retained_nodes(retained_nodes);
            }

            // Remove retained nodes from the old program or we'll hit a debug assertion since
            // its debug variables no longer point to the program. See the
            // NumAbstractSubprograms assertion in DwarfDebug::endFunctionImpl in LLVM.
            let empty_node =
                unsafe { LLVMMDNodeInContext2(self.context, core::ptr::null_mut(), 0) };
            subprogram.set_retained_nodes(empty_node);

            let ret = replace.insert(subprogram.value_ref as u64, unsafe {
                LLVMValueAsMetadata(new_program.value_ref)
            });
            assert!(ret.is_none());
        }

        replace
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

impl Operand {
    fn replace(&mut self, value: LLVMValueRef) {
        unsafe {
            if !LLVMIsAMDNode(self.parent).is_null() {
                let value = LLVMValueAsMetadata(value);
                LLVMReplaceMDNodeOperandWith(self.parent, self.index, value);
            } else if !LLVMIsAUser(self.parent).is_null() {
                LLVMSetOperand(self.parent, self.index, value);
            }
        }
    }
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
