use std::{
    collections::{HashMap, HashSet},
    ffi::CStr,
};

use gimli::{DW_TAG_array_type, DW_TAG_pointer_type, DW_TAG_union_type};
use llvm_sys::{
    LLVMDbgRecordKind, LLVMOpcode, LLVMTypeKind,
    core::{
        LLVMArrayType2, LLVMConstInt, LLVMConstIntGetZExtValue, LLVMCreateTypeAttribute,
        LLVMGetElementType, LLVMGetEnumAttributeKindForName, LLVMGetGEPSourceElementType,
        LLVMGetInstructionOpcode, LLVMGetIntTypeWidth, LLVMGetIntrinsicDeclaration,
        LLVMGetIntrinsicID, LLVMGetNumArgOperands, LLVMGetNumOperands, LLVMGetOperand,
        LLVMGetTypeKind, LLVMInstructionEraseFromParent, LLVMInt8TypeInContext,
        LLVMInt32TypeInContext, LLVMIntTypeInContext, LLVMIsAConstantInt, LLVMIsAInstruction,
        LLVMLookupIntrinsicID, LLVMPointerType, LLVMReplaceAllUsesWith, LLVMSetOperand,
        LLVMStructGetTypeAtIndex, LLVMStructTypeInContext, LLVMTypeOf,
    },
    debuginfo::{LLVMDITypeGetSizeInBits, LLVMGetMetadataKind, LLVMMetadataKind},
    prelude::LLVMValueRef,
};
use thiserror::Error;

use crate::llvm::{
    DataLayout, IRBuilder, LLVMContext, LLVMModule,
    types::{
        di::{DICompositeType, DIDerivedType, DISubroutineType},
        instruction::{
            CallInst, GetElementPtrInst, Instruction as _, InstructionKind, LoadInst, StoreInst,
        },
        ir::Metadata,
    },
};

const ELEMENTTYPE: &CStr = c"elementtype";
// Metadata kind attached to preserve-access intrinsic calls. The metadata payload carries the
// debug-info type used by BPF CO-RE relocation handling.
const PRESERVE_ACCESS_MD_NAME: &CStr = c"llvm.preserve.access.index";
// Intrinsic name for an array-element access. This preserves the selected array index and the
// element type through the CO-RE relocation pipeline.
const PRESERVE_ARRAY_ACCESS_INTRINSIC_NAME: &CStr = c"llvm.preserve.array.access.index";
// Intrinsic name for a struct-member access that behaves like a struct GEP while preserving
// relocation-relevant access indices.
const PRESERVE_STRUCT_ACCESS_INTRINSIC_NAME: &CStr = c"llvm.preserve.struct.access.index";
// Intrinsic name for a union-member access. Unlike the struct variant, this identifies the
// selected member by field index without an `elementtype` attribute.
const PRESERVE_UNION_ACCESS_INTRINSIC_NAME: &CStr = c"llvm.preserve.union.access.index";
const DBG_VALUE_INTRINSIC_NAME: &CStr = c"llvm.dbg.value";
const BITS_PER_BYTE: u64 = 8;
const DILOCAL_VARIABLE_TYPE_OPERAND: u32 = 3;

pub(crate) struct CoreRelocPass<'ctx, 'module, 'types> {
    context: &'ctx LLVMContext,
    module: &'module mut LLVMModule<'ctx>,
    builder: IRBuilder<'ctx>,
    data_layout: DataLayout<'ctx>,
    preserve_access_md_kind: u32,
    elementtype_attr_kind: u32,
    dbg_value_intrinsic_id: u32,
    preserve_access_types: &'types HashSet<usize>,
    pointer_types: HashMap<LLVMValueRef, LLVMValueRef>,
    synthetic_types: HashMap<LLVMValueRef, llvm_sys::prelude::LLVMTypeRef>,
}

struct ResolvedAccess {
    base: LLVMValueRef,
    composite_type: LLVMValueRef,
    offset_bytes: u64,
}

struct FieldMatch {
    member: LLVMValueRef,
    member_index: u32,
}

struct ContainingMemberMatch {
    member_index: u32,
    nested_composite: LLVMValueRef,
    remainder_bytes: u64,
}

struct AccessPath {
    ptr: LLVMValueRef,
    nested_composite: Option<LLVMValueRef>,
}

#[derive(Debug, Error)]
pub enum CoreRelocError {
    #[error("failed to compute GEP offset: {0}")]
    GepOffset(#[from] GepOffsetError),
    #[error("failed to build array access path: {0}")]
    ArrayAccess(#[from] ArrayAccessError),
}

#[derive(Debug, Error)]
#[expect(missing_copy_implementations, reason = "not needed")]
pub enum GepOffsetError {
    #[error("index operand is not a constant integer")]
    NonConstantIndex,
    #[error("invalid struct field index")]
    BadStructIndex,
    #[error("byte offset overflow")]
    OffsetOverflow,
    #[error("unsupported indexed type kind")]
    UnsupportedTypeKind,
}

#[derive(Debug, Error)]
#[expect(missing_copy_implementations, reason = "not needed")]
pub enum ArrayAccessError {
    #[error("array element type is missing")]
    MissingElementType,
    #[error("array element size is not byte-aligned")]
    NonByteAlignedElementSize,
    #[error("array element size is zero")]
    ZeroSizedElement,
    #[error("array index does not fit into u32")]
    IndexOverflow,
}

impl<'ctx, 'module, 'types> CoreRelocPass<'ctx, 'module, 'types> {
    pub(crate) fn new(
        context: &'ctx LLVMContext,
        module: &'module mut LLVMModule<'ctx>,
        preserve_access_types: &'types HashSet<usize>,
    ) -> Self {
        let data_layout = module.data_layout();
        let preserve_access_md_kind = context.md_kind_id(PRESERVE_ACCESS_MD_NAME);
        Self {
            context,
            module,
            builder: IRBuilder::new(context),
            data_layout,
            preserve_access_md_kind,
            elementtype_attr_kind: unsafe {
                LLVMGetEnumAttributeKindForName(ELEMENTTYPE.as_ptr(), ELEMENTTYPE.to_bytes().len())
            },
            dbg_value_intrinsic_id: unsafe {
                LLVMLookupIntrinsicID(
                    DBG_VALUE_INTRINSIC_NAME.as_ptr().cast(),
                    DBG_VALUE_INTRINSIC_NAME.to_bytes().len(),
                )
            },
            preserve_access_types,
            pointer_types: HashMap::new(),
            synthetic_types: HashMap::new(),
        }
    }

    pub(crate) fn run(&mut self) -> Result<(), CoreRelocError> {
        let functions: Vec<_> = self.module.functions().collect();
        for function in functions {
            self.pointer_types.clear();

            if let Some(subprogram) = function.subprogram(self.context.as_mut_ptr()) {
                let subroutine_type = unsafe {
                    DISubroutineType::from_metadata_ref(self.context.as_mut_ptr(), subprogram.ty())
                };

                for (param, metadata) in function.params().zip(subroutine_type.type_array().skip(1))
                {
                    let Some(metadata) = metadata else {
                        continue;
                    };
                    let Some(composite) = self.pointer_composite_from_metadata(metadata) else {
                        continue;
                    };
                    let _prev = self.pointer_types.insert(param, composite);
                }
            }

            let basic_blocks: Vec<_> = function.basic_blocks().collect();
            let mut declared_pointer_slots = HashMap::new();
            let mut pending_pointer_stores = HashMap::<LLVMValueRef, Vec<LLVMValueRef>>::new();

            for bb in &basic_blocks {
                for instruction in bb.instructions() {
                    self.seed_dbg_records(
                        &instruction,
                        &mut declared_pointer_slots,
                        &mut pending_pointer_stores,
                    );
                    self.seed_dbg_value(&instruction);
                    self.seed_store_pointer_type(
                        &instruction,
                        &declared_pointer_slots,
                        &mut pending_pointer_stores,
                    );
                }
            }

            for bb in &basic_blocks {
                for instruction in bb.instructions() {
                    match instruction {
                        InstructionKind::LoadInst(load_inst) => self.rewrite_load(load_inst)?,
                        InstructionKind::StoreInst(store_inst) => self.rewrite_store(store_inst)?,
                        InstructionKind::GetElementPtrInst(gep_inst) => {
                            self.rewrite_gep(gep_inst)?
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    fn seed_dbg_records(
        &mut self,
        instruction: &InstructionKind<'_>,
        declared_pointer_slots: &mut HashMap<LLVMValueRef, LLVMValueRef>,
        pending_pointer_stores: &mut HashMap<LLVMValueRef, Vec<LLVMValueRef>>,
    ) {
        for record in instruction.dbg_records() {
            match record.kind() {
                LLVMDbgRecordKind::LLVMDbgRecordValue => {
                    let value = record.value(0);
                    let variable_md = record.variable();
                    self.seed_pointer_type_from_variable(value, variable_md);
                }
                LLVMDbgRecordKind::LLVMDbgRecordDeclare => {
                    let slot = record.value(0);
                    let variable_md = record.variable();
                    let Some(composite) = self.pointer_composite_from_variable(variable_md) else {
                        continue;
                    };

                    let _prev = declared_pointer_slots.insert(slot, composite);
                    if let Some(values) = pending_pointer_stores.remove(&slot) {
                        for value in values {
                            let _prev = self.pointer_types.insert(value, composite);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn seed_dbg_value(&mut self, instruction: &InstructionKind<'_>) {
        if instruction.opcode() != LLVMOpcode::LLVMCall {
            return;
        }

        let callee = instruction.called_value();
        if callee.is_null()
            || unsafe { LLVMGetIntrinsicID(callee) } != self.dbg_value_intrinsic_id
            || unsafe { LLVMGetNumArgOperands(instruction.value_ref()) } < 2
        {
            return;
        }

        let args: Vec<_> = instruction.args().collect();
        let (value, variable) = match args.as_slice() {
            [value, variable] => (*value, *variable),
            _ => return,
        };
        if value.is_null() || variable.is_null() {
            return;
        }

        let variable_md = unsafe { llvm_sys::core::LLVMValueAsMetadata(variable) };
        self.seed_pointer_type_from_variable(value, variable_md);
    }

    fn seed_pointer_type_from_variable(
        &mut self,
        value: LLVMValueRef,
        variable_md: llvm_sys::prelude::LLVMMetadataRef,
    ) {
        if value.is_null() {
            return;
        }

        let Some(composite) = self.pointer_composite_from_variable(variable_md) else {
            return;
        };

        let _prev = self.pointer_types.insert(value, composite);
    }

    fn seed_store_pointer_type(
        &mut self,
        instruction: &InstructionKind<'_>,
        declared_pointer_slots: &HashMap<LLVMValueRef, LLVMValueRef>,
        pending_pointer_stores: &mut HashMap<LLVMValueRef, Vec<LLVMValueRef>>,
    ) {
        if instruction.opcode() != LLVMOpcode::LLVMStore {
            return;
        }

        let slot = unsafe { LLVMGetOperand(instruction.value_ref(), 1) };
        let value = unsafe { LLVMGetOperand(instruction.value_ref(), 0) };
        if let Some(&composite) = declared_pointer_slots.get(&slot) {
            let _prev = self.pointer_types.insert(value, composite);
        } else {
            pending_pointer_stores.entry(slot).or_default().push(value);
        }
    }

    fn pointer_composite_from_variable(
        &self,
        variable_md: llvm_sys::prelude::LLVMMetadataRef,
    ) -> Option<LLVMValueRef> {
        if variable_md.is_null()
            || !matches!(
                unsafe { LLVMGetMetadataKind(variable_md) },
                LLVMMetadataKind::LLVMDILocalVariableMetadataKind
            )
        {
            return None;
        }

        let variable =
            unsafe { llvm_sys::core::LLVMMetadataAsValue(self.context.as_mut_ptr(), variable_md) };
        let variable_type = unsafe { LLVMGetOperand(variable, DILOCAL_VARIABLE_TYPE_OPERAND) };
        if variable_type.is_null() {
            return None;
        }

        self.pointer_composite_from_metadata(unsafe { Metadata::from_value_ref(variable_type) })
    }

    fn rewrite_load(&mut self, mut load: LoadInst<'_>) -> Result<(), CoreRelocError> {
        let Some(resolved) = self.resolve_access(load.pointer())? else {
            return Ok(());
        };

        let Some(access_path) = self.build_access_path(
            load.value_ref(),
            resolved.base,
            resolved.composite_type,
            resolved.offset_bytes,
            Some(unsafe { LLVMTypeOf(load.value_ref()) }),
        )?
        else {
            return Ok(());
        };
        load.set_pointer(access_path.ptr);

        if self.is_pointer_type(unsafe { LLVMTypeOf(load.value_ref()) })
            && let Some(pointee) = access_path.nested_composite
        {
            let _prev = self.pointer_types.insert(load.value_ref(), pointee);
        }

        Ok(())
    }

    fn rewrite_store(&mut self, store: StoreInst<'_>) -> Result<(), CoreRelocError> {
        let Some(resolved) = self.resolve_access(store.pointer())? else {
            return Ok(());
        };

        let stored_value = store.value();
        let Some(access_path) = self.build_access_path(
            store.value_ref(),
            resolved.base,
            resolved.composite_type,
            resolved.offset_bytes,
            Some(unsafe { LLVMTypeOf(stored_value) }),
        )?
        else {
            return Ok(());
        };
        unsafe { LLVMSetOperand(store.value_ref(), 1, access_path.ptr) };

        Ok(())
    }

    fn rewrite_gep(&mut self, gep: GetElementPtrInst<'_>) -> Result<(), CoreRelocError> {
        let Some(resolved) = self.resolve_access(gep.value_ref())? else {
            return Ok(());
        };

        let composite = unsafe { DICompositeType::from_value_ref(resolved.composite_type) };
        if resolved.offset_bytes == 0 && composite.tag() != DW_TAG_array_type {
            return Ok(());
        }

        let Some(access_path) = self.build_access_path(
            gep.value_ref(),
            resolved.base,
            resolved.composite_type,
            resolved.offset_bytes,
            None,
        )?
        else {
            return Ok(());
        };

        if let Some(nested) = access_path.nested_composite {
            let _prev = self.pointer_types.insert(access_path.ptr, nested);
        }

        unsafe {
            LLVMReplaceAllUsesWith(gep.value_ref(), access_path.ptr);
            LLVMInstructionEraseFromParent(gep.value_ref());
        }

        Ok(())
    }

    fn resolve_access(
        &self,
        value: LLVMValueRef,
    ) -> Result<Option<ResolvedAccess>, CoreRelocError> {
        if let Some(&composite_type) = self.pointer_types.get(&value) {
            return Ok(Some(ResolvedAccess {
                base: value,
                composite_type,
                offset_bytes: 0,
            }));
        }

        if unsafe { LLVMIsAInstruction(value).is_null() } {
            return Ok(None);
        }

        let opcode = unsafe { LLVMGetInstructionOpcode(value) };
        match opcode {
            LLVMOpcode::LLVMBitCast | LLVMOpcode::LLVMAddrSpaceCast => {
                self.resolve_access(unsafe { LLVMGetOperand(value, 0) })
            }
            LLVMOpcode::LLVMGetElementPtr => {
                let base = unsafe { LLVMGetOperand(value, 0) };
                let Some(mut resolved) = self.resolve_access(base)? else {
                    return Ok(None);
                };
                let offset = self.constant_gep_offset(value)?;
                resolved.offset_bytes = resolved
                    .offset_bytes
                    .checked_add(offset)
                    .ok_or(GepOffsetError::OffsetOverflow)?;
                Ok(Some(resolved))
            }
            _ => Ok(None),
        }
    }

    fn constant_gep_offset(&self, gep: LLVMValueRef) -> Result<u64, GepOffsetError> {
        let mut current_type = unsafe { LLVMGetGEPSourceElementType(gep) };
        let mut offset_bytes = 0_u64;
        let num_operands = unsafe { LLVMGetNumOperands(gep) };

        for operand_index in 1..num_operands {
            let index_operand = unsafe { LLVMGetOperand(gep, operand_index as u32) };
            let index = if unsafe { LLVMIsAConstantInt(index_operand).is_null() } {
                return Err(GepOffsetError::NonConstantIndex);
            } else {
                unsafe { LLVMConstIntGetZExtValue(index_operand) }
            };

            if operand_index == 1 {
                let stride = self.data_layout.abi_size_of_type(current_type);
                offset_bytes = offset_bytes
                    .checked_add(
                        index
                            .checked_mul(stride)
                            .ok_or(GepOffsetError::OffsetOverflow)?,
                    )
                    .ok_or(GepOffsetError::OffsetOverflow)?;
                continue;
            }

            match unsafe { LLVMGetTypeKind(current_type) } {
                LLVMTypeKind::LLVMStructTypeKind => {
                    let field_index = index
                        .try_into()
                        .map_err(|_| GepOffsetError::BadStructIndex)?;
                    let field_offset = self
                        .data_layout
                        .offset_of_element(current_type, field_index);
                    offset_bytes = offset_bytes
                        .checked_add(field_offset)
                        .ok_or(GepOffsetError::OffsetOverflow)?;
                    current_type = unsafe { LLVMStructGetTypeAtIndex(current_type, field_index) };
                }
                LLVMTypeKind::LLVMArrayTypeKind | LLVMTypeKind::LLVMVectorTypeKind => {
                    let element_type = unsafe { LLVMGetElementType(current_type) };
                    let stride = self.data_layout.abi_size_of_type(element_type);
                    offset_bytes = offset_bytes
                        .checked_add(
                            index
                                .checked_mul(stride)
                                .ok_or(GepOffsetError::OffsetOverflow)?,
                        )
                        .ok_or(GepOffsetError::OffsetOverflow)?;
                    current_type = element_type;
                }
                _ => return Err(GepOffsetError::UnsupportedTypeKind),
            }
        }

        Ok(offset_bytes)
    }

    fn match_field(
        &self,
        composite_type: LLVMValueRef,
        offset_bytes: u64,
        expected_type: Option<llvm_sys::prelude::LLVMTypeRef>,
    ) -> Option<FieldMatch> {
        let composite = unsafe { DICompositeType::from_value_ref(composite_type) };
        let offset_bits = offset_bytes.checked_mul(BITS_PER_BYTE)?;

        for (index, element) in composite.elements().enumerate() {
            let Metadata::DIDerivedType(member) = element else {
                continue;
            };

            if member.offset_in_bits() != offset_bits {
                continue;
            }

            if let Some(expected_type) = expected_type
                && !self.member_matches_llvm_type(&member, expected_type)
            {
                continue;
            }

            return Some(FieldMatch {
                member: member.value_ref(),
                member_index: index.try_into().unwrap(),
            });
        }

        None
    }

    fn member_matches_llvm_type(
        &self,
        member: &DIDerivedType<'_>,
        expected_type: llvm_sys::prelude::LLVMTypeRef,
    ) -> bool {
        self.metadata_matches_llvm_type(member.base_type(), expected_type)
    }

    fn metadata_matches_llvm_type(
        &self,
        metadata: Metadata<'_>,
        expected_type: llvm_sys::prelude::LLVMTypeRef,
    ) -> bool {
        match unsafe { LLVMGetTypeKind(expected_type) } {
            LLVMTypeKind::LLVMPointerTypeKind => {
                matches!(metadata, Metadata::DIDerivedType(ref ty) if ty.tag() == DW_TAG_pointer_type)
            }
            LLVMTypeKind::LLVMIntegerTypeKind => {
                self.metadata_size_in_bits(metadata).is_some_and(|size| {
                    size == u64::from(unsafe { LLVMGetIntTypeWidth(expected_type) })
                })
            }
            _ => !matches!(
                metadata,
                Metadata::DIDerivedType(ref ty) if ty.tag() == DW_TAG_pointer_type
            ),
        }
    }

    fn member_value_composite(&self, member: &DIDerivedType<'_>) -> Option<LLVMValueRef> {
        match member.base_type() {
            Metadata::DICompositeType(composite) => Some(composite.value_ref()),
            _ => None,
        }
    }

    fn member_pointee_composite(&self, member: &DIDerivedType<'_>) -> Option<LLVMValueRef> {
        let Metadata::DIDerivedType(pointer_type) = member.base_type() else {
            return None;
        };
        if pointer_type.tag() != DW_TAG_pointer_type {
            return None;
        }

        match pointer_type.base_type() {
            Metadata::DICompositeType(composite) => Some(composite.value_ref()),
            _ => None,
        }
    }

    fn pointer_composite_from_metadata(&self, metadata: Metadata<'_>) -> Option<LLVMValueRef> {
        let Metadata::DIDerivedType(pointer_type) = metadata else {
            return None;
        };
        if pointer_type.tag() != DW_TAG_pointer_type {
            return None;
        }

        match pointer_type.base_type() {
            Metadata::DICompositeType(composite)
                if self
                    .preserve_access_types
                    .contains(&(composite.value_ref() as usize)) =>
            {
                Some(composite.value_ref())
            }
            _ => None,
        }
    }

    fn build_preserve_access(
        &mut self,
        before: LLVMValueRef,
        base: LLVMValueRef,
        composite_type: LLVMValueRef,
        member_index: u32,
    ) -> Option<CallInst<'ctx>> {
        let composite = unsafe { DICompositeType::from_value_ref(composite_type) };
        let field_index = unsafe {
            LLVMConstInt(
                LLVMInt32TypeInContext(self.context.as_mut_ptr()),
                member_index.into(),
                0,
            )
        };

        let mut call = if composite.tag() == DW_TAG_union_type {
            let callee = self.preserve_union_access_function(base);
            let mut args = [base, field_index];
            self.builder.position_before(before);
            self.builder.build_call2(callee, &mut args)
        } else {
            let callee = self.preserve_struct_access_function(base);
            let element_type = self.synthetic_struct_type(composite_type)?;
            let mut args = [base, field_index, field_index];
            self.builder.position_before(before);
            let mut call = self.builder.build_call2(callee, &mut args);
            let elementtype_attr = unsafe {
                LLVMCreateTypeAttribute(
                    self.context.as_mut_ptr(),
                    self.elementtype_attr_kind,
                    element_type,
                )
            };
            call.add_attribute(1, elementtype_attr);
            call
        };
        call.set_metadata(self.preserve_access_md_kind, composite_type);
        Some(call)
    }

    fn build_preserve_array_access(
        &mut self,
        before: LLVMValueRef,
        base: LLVMValueRef,
        composite_type: &DICompositeType<'_>,
        index: u32,
    ) -> Option<CallInst<'ctx>> {
        let callee = self.preserve_array_access_function(base);
        let element_type = self.synthetic_llvm_type(composite_type.base_type()?)?;
        let dimension =
            unsafe { LLVMConstInt(LLVMInt32TypeInContext(self.context.as_mut_ptr()), 1, 0) };
        let last_index = unsafe {
            LLVMConstInt(
                LLVMInt32TypeInContext(self.context.as_mut_ptr()),
                index.into(),
                0,
            )
        };
        let mut args = [base, dimension, last_index];

        self.builder.position_before(before);
        let mut call = self.builder.build_call2(callee, &mut args);
        let elementtype_attr = self
            .context
            .create_type_attribute(self.elementtype_attr_kind, element_type);
        call.add_attribute(1, elementtype_attr);
        call.set_metadata(self.preserve_access_md_kind, composite_type.value_ref());
        Some(call)
    }

    fn preserve_array_access_function(&self, base: LLVMValueRef) -> LLVMValueRef {
        let ptr_ty = unsafe { LLVMTypeOf(base) };
        let mut overloaded_tys = [ptr_ty, ptr_ty];
        let intrinsic_id = unsafe {
            LLVMLookupIntrinsicID(
                PRESERVE_ARRAY_ACCESS_INTRINSIC_NAME.as_ptr(),
                PRESERVE_ARRAY_ACCESS_INTRINSIC_NAME.to_bytes().len(),
            )
        };

        unsafe {
            LLVMGetIntrinsicDeclaration(
                self.module.as_mut_ptr(),
                intrinsic_id,
                overloaded_tys.as_mut_ptr(),
                overloaded_tys.len(),
            )
        }
    }

    fn preserve_struct_access_function(&self, base: LLVMValueRef) -> LLVMValueRef {
        let ptr_ty = unsafe { LLVMTypeOf(base) };
        let mut overloaded_tys = [ptr_ty, ptr_ty];
        let intrinsic_id = unsafe {
            LLVMLookupIntrinsicID(
                PRESERVE_STRUCT_ACCESS_INTRINSIC_NAME.as_ptr(),
                PRESERVE_STRUCT_ACCESS_INTRINSIC_NAME.to_bytes().len(),
            )
        };

        unsafe {
            LLVMGetIntrinsicDeclaration(
                self.module.as_mut_ptr(),
                intrinsic_id,
                overloaded_tys.as_mut_ptr(),
                overloaded_tys.len(),
            )
        }
    }

    fn preserve_union_access_function(&self, base: LLVMValueRef) -> LLVMValueRef {
        let ptr_ty = unsafe { LLVMTypeOf(base) };
        let mut overloaded_tys = [ptr_ty, ptr_ty];
        let intrinsic_id = unsafe {
            LLVMLookupIntrinsicID(
                PRESERVE_UNION_ACCESS_INTRINSIC_NAME.as_ptr(),
                PRESERVE_UNION_ACCESS_INTRINSIC_NAME.to_bytes().len(),
            )
        };

        unsafe {
            LLVMGetIntrinsicDeclaration(
                self.module.as_mut_ptr(),
                intrinsic_id,
                overloaded_tys.as_mut_ptr(),
                overloaded_tys.len(),
            )
        }
    }

    fn is_pointer_type(&self, ty: llvm_sys::prelude::LLVMTypeRef) -> bool {
        matches!(
            unsafe { LLVMGetTypeKind(ty) },
            LLVMTypeKind::LLVMPointerTypeKind
        )
    }

    fn synthetic_struct_type(
        &mut self,
        composite_type: LLVMValueRef,
    ) -> Option<llvm_sys::prelude::LLVMTypeRef> {
        if let Some(&ty) = self.synthetic_types.get(&composite_type) {
            return Some(ty);
        }

        let composite = unsafe { DICompositeType::from_value_ref(composite_type) };
        let mut element_types = Vec::new();
        for element in composite.elements() {
            let Metadata::DIDerivedType(member) = element else {
                continue;
            };
            let element_type = self.synthetic_llvm_type(member.base_type())?;
            element_types.push(element_type);
        }

        let ty = unsafe {
            LLVMStructTypeInContext(
                self.context.as_mut_ptr(),
                element_types.as_mut_ptr(),
                element_types.len().try_into().unwrap(),
                0,
            )
        };
        let _prev = self.synthetic_types.insert(composite_type, ty);
        Some(ty)
    }

    fn synthetic_array_type(
        &mut self,
        composite_type: LLVMValueRef,
    ) -> Option<llvm_sys::prelude::LLVMTypeRef> {
        if let Some(&ty) = self.synthetic_types.get(&composite_type) {
            return Some(ty);
        }

        let composite = unsafe { DICompositeType::from_value_ref(composite_type) };
        let element_type = self.synthetic_llvm_type(composite.base_type()?)?;
        let base_size_bits = self.metadata_size_in_bits(composite.base_type()?)?;
        if base_size_bits == 0 {
            return None;
        }
        let element_count = composite.size_in_bits().checked_div(base_size_bits)?;
        let ty = unsafe { LLVMArrayType2(element_type, element_count) };
        let _prev = self.synthetic_types.insert(composite_type, ty);
        Some(ty)
    }

    fn synthetic_llvm_type(
        &mut self,
        metadata: Metadata<'_>,
    ) -> Option<llvm_sys::prelude::LLVMTypeRef> {
        match metadata {
            Metadata::DIDerivedType(derived) if derived.tag() == DW_TAG_pointer_type => {
                Some(unsafe {
                    LLVMPointerType(LLVMInt8TypeInContext(self.context.as_mut_ptr()), 0)
                })
            }
            Metadata::DICompositeType(composite) => {
                if composite.tag() == DW_TAG_array_type {
                    self.synthetic_array_type(composite.value_ref())
                } else {
                    self.synthetic_struct_type(composite.value_ref())
                }
            }
            Metadata::Other(value_ref) => {
                let metadata_ref = unsafe { llvm_sys::core::LLVMValueAsMetadata(value_ref) };
                match unsafe { LLVMGetMetadataKind(metadata_ref) } {
                    LLVMMetadataKind::LLVMDIBasicTypeMetadataKind => {
                        let size_bits = unsafe { LLVMDITypeGetSizeInBits(metadata_ref) };
                        Some(unsafe {
                            LLVMIntTypeInContext(
                                self.context.as_mut_ptr(),
                                size_bits.try_into().unwrap(),
                            )
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn array_element_composite(&self, composite: &DICompositeType<'_>) -> Option<LLVMValueRef> {
        match composite.base_type()? {
            Metadata::DICompositeType(element) => Some(element.value_ref()),
            _ => None,
        }
    }

    fn build_access_path(
        &mut self,
        before: LLVMValueRef,
        base: LLVMValueRef,
        composite_type: LLVMValueRef,
        offset_bytes: u64,
        expected_type: Option<llvm_sys::prelude::LLVMTypeRef>,
    ) -> Result<Option<AccessPath>, CoreRelocError> {
        let composite = unsafe { DICompositeType::from_value_ref(composite_type) };
        if composite.tag() == DW_TAG_array_type {
            return self.build_array_access_path(
                before,
                base,
                &composite,
                offset_bytes,
                expected_type,
            );
        }

        if let Some(field) = self.match_field(composite_type, offset_bytes, expected_type) {
            let ptr = match self.build_preserve_access(
                before,
                base,
                composite_type,
                field.member_index,
            ) {
                Some(ptr) => ptr,
                None => return Ok(None),
            };
            let member = unsafe { DIDerivedType::from_value_ref(field.member) };
            let nested_composite = if expected_type.is_some_and(|ty| self.is_pointer_type(ty)) {
                self.member_pointee_composite(&member)
            } else {
                self.member_value_composite(&member)
            };
            return Ok(Some(AccessPath {
                ptr: ptr.value_ref(),
                nested_composite,
            }));
        }

        let Some(containing) = self.match_containing_member(composite_type, offset_bytes) else {
            return Ok(None);
        };
        let ptr =
            match self.build_preserve_access(before, base, composite_type, containing.member_index)
            {
                Some(ptr) => ptr,
                None => return Ok(None),
            };
        self.build_access_path(
            before,
            ptr.value_ref(),
            containing.nested_composite,
            containing.remainder_bytes,
            expected_type,
        )
    }

    fn build_array_access_path(
        &mut self,
        before: LLVMValueRef,
        base: LLVMValueRef,
        composite: &DICompositeType<'_>,
        offset_bytes: u64,
        expected_type: Option<llvm_sys::prelude::LLVMTypeRef>,
    ) -> Result<Option<AccessPath>, CoreRelocError> {
        let element_size_bits = self
            .metadata_size_in_bits(
                composite
                    .base_type()
                    .ok_or(ArrayAccessError::MissingElementType)?,
            )
            .ok_or(ArrayAccessError::MissingElementType)?;
        if element_size_bits % BITS_PER_BYTE != 0 {
            return Err(ArrayAccessError::NonByteAlignedElementSize.into());
        }
        let element_size_bytes = element_size_bits / BITS_PER_BYTE;
        if element_size_bytes == 0 {
            return Err(ArrayAccessError::ZeroSizedElement.into());
        }

        let index = offset_bytes / element_size_bytes;
        let remainder_bytes = offset_bytes % element_size_bytes;
        let index: u32 = index
            .try_into()
            .map_err(|_| ArrayAccessError::IndexOverflow)?;
        let Some(call) = self.build_preserve_array_access(before, base, composite, index) else {
            return Ok(None);
        };
        let call_value = call.value_ref();

        if remainder_bytes == 0 {
            if let Some(expected_type) = expected_type
                && !self.metadata_matches_llvm_type(
                    composite
                        .base_type()
                        .ok_or(ArrayAccessError::MissingElementType)?,
                    expected_type,
                )
            {
                return Ok(None);
            }

            return Ok(Some(AccessPath {
                ptr: call_value,
                nested_composite: self.array_element_composite(composite),
            }));
        }

        let Some(nested) = self.array_element_composite(composite) else {
            return Ok(None);
        };
        self.build_access_path(before, call_value, nested, remainder_bytes, expected_type)
    }

    fn match_containing_member(
        &self,
        composite_type: LLVMValueRef,
        offset_bytes: u64,
    ) -> Option<ContainingMemberMatch> {
        let composite = unsafe { DICompositeType::from_value_ref(composite_type) };
        let offset_bits = offset_bytes.checked_mul(BITS_PER_BYTE)?;

        for (index, element) in composite.elements().enumerate() {
            let Metadata::DIDerivedType(member) = element else {
                continue;
            };
            let member_offset_bits = member.offset_in_bits();
            if offset_bits < member_offset_bits {
                continue;
            }

            let Metadata::DICompositeType(nested) = member.base_type() else {
                continue;
            };
            let nested_size_bits = nested.size_in_bits();
            if offset_bits >= member_offset_bits.checked_add(nested_size_bits)? {
                continue;
            }

            return Some(ContainingMemberMatch {
                member_index: index.try_into().unwrap(),
                nested_composite: nested.value_ref(),
                remainder_bytes: (offset_bits - member_offset_bits) / BITS_PER_BYTE,
            });
        }

        None
    }

    fn metadata_size_in_bits(&self, metadata: Metadata<'_>) -> Option<u64> {
        match metadata {
            Metadata::DIDerivedType(derived) => Some(derived.size_in_bits()),
            Metadata::DICompositeType(composite) => Some(composite.size_in_bits()),
            Metadata::Other(value_ref) => {
                let metadata_ref = unsafe { llvm_sys::core::LLVMValueAsMetadata(value_ref) };
                matches!(
                    unsafe { LLVMGetMetadataKind(metadata_ref) },
                    LLVMMetadataKind::LLVMDIBasicTypeMetadataKind
                )
                .then(|| unsafe { LLVMDITypeGetSizeInBits(metadata_ref) })
            }
            _ => None,
        }
    }
}
