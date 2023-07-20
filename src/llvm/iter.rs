use std::marker::PhantomData;

use llvm_sys::core::*;
use llvm_sys::prelude::*;

macro_rules! llvm_iterator {
    ($trait_name:ident, $iterator_name:ident, $iterable:ty, $method_name:ident, $item_ty:ty, $first:expr, $last:expr, $next:expr $(,)?) => {
        pub trait $trait_name {
            fn $method_name(&self) -> $iterator_name;
        }

        pub struct $iterator_name<'a> {
            lifetime: PhantomData<&'a $iterable>,
            next: $item_ty,
            last: $item_ty,
        }

        impl $trait_name for $iterable {
            fn $method_name(&self) -> $iterator_name {
                let first = unsafe { $first(*self) };
                let last = unsafe { $last(*self) };
                assert_eq!(first.is_null(), last.is_null());
                $iterator_name {
                    lifetime: PhantomData,
                    next: first,
                    last,
                }
            }
        }

        impl<'a> Iterator for $iterator_name<'a> {
            type Item = $item_ty;

            fn next(&mut self) -> Option<Self::Item> {
                let Self {
                    lifetime: _,
                    next,
                    last,
                } = self;
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

llvm_iterator! {
    IterFunctionBasicBlocks,
    BasicBlockIter,
    LLVMValueRef,
    basic_blocks_iter,
    LLVMBasicBlockRef,
    LLVMGetFirstBasicBlock,
    LLVMGetLastBasicBlock,
    LLVMGetNextBasicBlock,
}

llvm_iterator! {
    IterBasicBlockInstructions,
    InstructionsIter,
    LLVMBasicBlockRef,
    instructions_iter,
    LLVMValueRef,
    LLVMGetFirstInstruction,
    LLVMGetLastInstruction,
    LLVMGetNextInstruction,
}
