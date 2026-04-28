use std::marker::PhantomData;

use llvm_sys::{
    LLVMOpcode,
    core::{
        LLVMAddCallSiteAttribute, LLVMGetArgOperand, LLVMGetCalledValue, LLVMGetInstructionOpcode,
        LLVMGetNumArgOperands, LLVMGetOperand, LLVMSetMetadata, LLVMSetOperand,
    },
    prelude::{LLVMAttributeRef, LLVMValueRef},
};

use crate::llvm::{iter::IterDbgRecords as _, types::ir::DbgRecord};

pub(crate) trait Instruction<'ctx> {
    fn value_ref(&self) -> LLVMValueRef;

    fn opcode(&self) -> LLVMOpcode {
        unsafe { LLVMGetInstructionOpcode(self.value_ref()) }
    }

    fn called_value(&self) -> LLVMValueRef {
        unsafe { LLVMGetCalledValue(self.value_ref()) }
    }

    fn num_args(&self) -> u32 {
        unsafe { LLVMGetNumArgOperands(self.value_ref()) }
    }

    fn args(&self) -> impl Iterator<Item = LLVMValueRef> {
        (0..self.num_args()).map(|i| unsafe { LLVMGetArgOperand(self.value_ref(), i) })
    }

    fn dbg_records(&self) -> impl Iterator<Item = DbgRecord<'ctx>> + '_ {
        self.value_ref()
            .dbg_records_iter()
            .map(|value_ref| unsafe { DbgRecord::from_dbg_record_ref(value_ref) })
    }
}

pub(crate) enum InstructionKind<'ctx> {
    CallInst(CallInst<'ctx>),
    GetElementPtrInst(GetElementPtrInst<'ctx>),
    LoadInst(LoadInst<'ctx>),
    StoreInst(StoreInst<'ctx>),
    Other(LLVMValueRef),
}

impl InstructionKind<'_> {
    pub(crate) fn from_value_ref(value: LLVMValueRef) -> Self {
        // SAFETY: We check the subclass of `Instruction`.
        unsafe {
            match LLVMGetInstructionOpcode(value) {
                LLVMOpcode::LLVMCall => Self::CallInst(CallInst::from_value_ref(value)),
                LLVMOpcode::LLVMGetElementPtr => {
                    Self::GetElementPtrInst(GetElementPtrInst::from_value_ref(value))
                }
                LLVMOpcode::LLVMLoad => Self::LoadInst(LoadInst::from_value_ref(value)),
                LLVMOpcode::LLVMStore => Self::StoreInst(StoreInst::from_value_ref(value)),
                _ => Self::Other(value),
            }
        }
    }

    pub(crate) fn value_ref(&self) -> LLVMValueRef {
        match self {
            Self::CallInst(call_inst) => call_inst.value_ref(),
            Self::GetElementPtrInst(gep_inst) => gep_inst.value_ref(),
            Self::LoadInst(load_inst) => load_inst.value_ref(),
            Self::StoreInst(store_inst) => store_inst.value_ref(),
            Self::Other(value_ref) => *value_ref,
        }
    }
}

impl<'ctx> Instruction<'ctx> for InstructionKind<'ctx> {
    fn value_ref(&self) -> LLVMValueRef {
        Self::value_ref(self)
    }
}

pub(crate) struct CallInst<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl CallInst<'_> {
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    pub(crate) fn add_attribute(&mut self, index: u32, attribute_ref: LLVMAttributeRef) {
        unsafe {
            LLVMAddCallSiteAttribute(self.value_ref, index, attribute_ref);
        }
    }

    pub(crate) fn set_metadata(&mut self, kind: u32, node: LLVMValueRef) {
        unsafe {
            LLVMSetMetadata(self.value_ref, kind, node);
        }
    }
}

impl<'ctx> Instruction<'ctx> for CallInst<'ctx> {
    fn value_ref(&self) -> LLVMValueRef {
        self.value_ref
    }
}

pub(crate) struct GetElementPtrInst<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl GetElementPtrInst<'_> {
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }
}

impl<'ctx> Instruction<'ctx> for GetElementPtrInst<'ctx> {
    fn value_ref(&self) -> LLVMValueRef {
        self.value_ref
    }
}

#[repr(u32)]
enum LoadInstOperand {
    /// The pointer being accessed by the load instruction. Reference in
    /// [LLVM 22][llvm-22].
    ///
    /// [llvm-22]: https://github.com/llvm/llvm-project/blob/llvmorg-22.1.4/llvm/include/llvm/IR/Instructions.h#L260-L261
    Pointer = 0,
}

pub(crate) struct LoadInst<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl LoadInst<'_> {
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    pub(crate) fn pointer(&self) -> LLVMValueRef {
        unsafe { LLVMGetOperand(self.value_ref, LoadInstOperand::Pointer as u32) }
    }

    pub(crate) fn set_pointer(&mut self, value_ref: LLVMValueRef) {
        unsafe {
            LLVMSetOperand(self.value_ref, LoadInstOperand::Pointer as u32, value_ref);
        }
    }
}

impl<'ctx> Instruction<'ctx> for LoadInst<'ctx> {
    fn value_ref(&self) -> LLVMValueRef {
        self.value_ref
    }
}

/// Represents the operands for a [`StoreInst`]. The enum values correspond to
/// the operand indices within metadata index.
#[repr(u32)]
enum StoreInstOperand {
    /// The value being written by the store instruction. Reference in
    /// [LLVM 22][llvm-22].
    ///
    /// [llvm-22]: https://github.com/llvm/llvm-project/blob/llvmorg-22.1.4/llvm/include/llvm/IR/Instructions.h#L384-L385
    Value = 0,
    /// The destination pointer that receives the stored value. Reference in
    /// [LLVM 22][llvm-22].
    ///
    /// [llvm-22]: https://github.com/llvm/llvm-project/blob/llvmorg-22.1.4/llvm/include/llvm/IR/Instructions.h#L387-L388
    Pointer = 1,
}

pub(crate) struct StoreInst<'ctx> {
    value_ref: LLVMValueRef,
    _marker: PhantomData<&'ctx ()>,
}

impl StoreInst<'_> {
    pub(crate) unsafe fn from_value_ref(value_ref: LLVMValueRef) -> Self {
        Self {
            value_ref,
            _marker: PhantomData,
        }
    }

    pub(crate) fn value(&self) -> LLVMValueRef {
        unsafe { LLVMGetOperand(self.value_ref, StoreInstOperand::Value as u32) }
    }

    pub(crate) fn pointer(&self) -> LLVMValueRef {
        unsafe { LLVMGetOperand(self.value_ref, StoreInstOperand::Pointer as u32) }
    }
}

impl<'ctx> Instruction<'ctx> for StoreInst<'ctx> {
    fn value_ref(&self) -> LLVMValueRef {
        self.value_ref
    }
}
