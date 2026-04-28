use llvm_sys::{
    core::{
        LLVMGetFirstBasicBlock, LLVMGetFirstDbgRecord, LLVMGetFirstFunction, LLVMGetFirstGlobal,
        LLVMGetFirstGlobalAlias, LLVMGetFirstInstruction, LLVMGetLastBasicBlock,
        LLVMGetLastDbgRecord, LLVMGetLastFunction, LLVMGetLastGlobal, LLVMGetLastGlobalAlias,
        LLVMGetLastInstruction, LLVMGetNextBasicBlock, LLVMGetNextDbgRecord, LLVMGetNextFunction,
        LLVMGetNextGlobal, LLVMGetNextGlobalAlias, LLVMGetNextInstruction,
    },
    prelude::{LLVMBasicBlockRef, LLVMDbgRecordRef, LLVMModuleRef, LLVMValueRef},
};

macro_rules! llvm_iterator {
    ($trait_name:ident, $iterator_name:ident, $iterable:ty, $method_name:ident, $item_ty:ty, $first:expr, $last:expr, $next:expr $(,)?) => {
        pub(crate) trait $trait_name {
            fn $method_name(&self) -> $iterator_name;
        }

        pub(crate) struct $iterator_name {
            next: $item_ty,
            last: $item_ty,
        }

        impl $trait_name for $iterable {
            fn $method_name(&self) -> $iterator_name {
                let first = unsafe { $first(*self) };
                let last = unsafe { $last(*self) };
                assert_eq!(first.is_null(), last.is_null());
                $iterator_name { next: first, last }
            }
        }

        impl Iterator for $iterator_name {
            type Item = $item_ty;

            fn next(&mut self) -> Option<Self::Item> {
                let Self { next, last } = self;
                if next.is_null() {
                    return None;
                }
                let last = *next == *last;
                let item = *next;
                *next = unsafe { $next(*next) };
                assert_eq!(next.is_null(), last);
                Some(item)
            }
        }
    };
}

llvm_iterator! {
    IterModuleGlobals,
    GlobalsIter,
    LLVMModuleRef,
    globals_iter,
    LLVMValueRef,
    LLVMGetFirstGlobal,
    LLVMGetLastGlobal,
    LLVMGetNextGlobal,
}

llvm_iterator! {
    IterModuleGlobalAliases,
    GlobalAliasesIter,
    LLVMModuleRef,
    global_aliases_iter,
    LLVMValueRef,
    LLVMGetFirstGlobalAlias,
    LLVMGetLastGlobalAlias,
    LLVMGetNextGlobalAlias,
}

llvm_iterator! {
    IterModuleFunctions,
    FunctionsIter,
    LLVMModuleRef,
    functions_iter,
    LLVMValueRef,
    LLVMGetFirstFunction,
    LLVMGetLastFunction,
    LLVMGetNextFunction,
}

llvm_iterator!(
    IterBasicBlocks,
    BasicBlockIter,
    LLVMValueRef,
    basic_blocks_iter,
    LLVMBasicBlockRef,
    LLVMGetFirstBasicBlock,
    LLVMGetLastBasicBlock,
    LLVMGetNextBasicBlock
);

llvm_iterator!(
    IterInstructions,
    InstructionsIter,
    LLVMBasicBlockRef,
    instructions_iter,
    LLVMValueRef,
    LLVMGetFirstInstruction,
    LLVMGetLastInstruction,
    LLVMGetNextInstruction
);

llvm_iterator!(
    IterDbgRecords,
    DbgRecordsIter,
    LLVMValueRef,
    dbg_records_iter,
    LLVMDbgRecordRef,
    LLVMGetFirstDbgRecord,
    LLVMGetLastDbgRecord,
    LLVMGetNextDbgRecord,
);
