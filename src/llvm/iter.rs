use std::marker::PhantomData;

use llvm_sys::{
    core::{
        LLVMGetFirstBasicBlock, LLVMGetFirstFunction, LLVMGetFirstGlobal, LLVMGetFirstGlobalAlias,
        LLVMGetFirstInstruction, LLVMGetLastBasicBlock, LLVMGetLastFunction, LLVMGetLastGlobal,
        LLVMGetLastGlobalAlias, LLVMGetLastInstruction, LLVMGetNextBasicBlock, LLVMGetNextFunction,
        LLVMGetNextGlobal, LLVMGetNextGlobalAlias, LLVMGetNextInstruction,
    },
    prelude::{LLVMBasicBlockRef, LLVMValueRef},
};

use crate::llvm::types::ir::{BasicBlock, Function, Instruction, Module, Value};

macro_rules! llvm_iterator {
    (
        $trait_name:ident,
        $iterator_name:ident,
        $iterable:ident,
        $method_name:ident,
        $mut_method_name:ident,
        $item_ptr_ty:ident,
        $item_wrapper_ty:ident,
        $first:expr,
        $last:expr,
        $next:expr $(,)?
    ) => {
        pub trait $trait_name {
            fn $method_name(&self) -> $iterator_name;
            fn $mut_method_name(&mut self) -> $iterator_name;
        }

        pub struct $iterator_name<'a> {
            lifetime: PhantomData<&'a $iterable<'a>>,
            next: $item_ptr_ty,
            last: $item_ptr_ty,
        }

        impl<'ctx> $trait_name for $iterable<'ctx> {
            fn $method_name(&self) -> $iterator_name {
                use $crate::llvm::LLVMTypeWrapper;
                let first = unsafe { $first(self.as_ptr()) };
                let last = unsafe { $last(self.as_ptr()) };
                assert_eq!(first.is_null(), last.is_null());
                $iterator_name {
                    lifetime: PhantomData,
                    next: first,
                    last,
                }
            }

            fn $mut_method_name(&mut self) -> $iterator_name {
                use $crate::llvm::LLVMTypeWrapper;
                let first = unsafe { $first(self.as_ptr()) };
                let last = unsafe { $last(self.as_ptr()) };
                assert_eq!(first.is_null(), last.is_null());
                $iterator_name {
                    lifetime: PhantomData,
                    next: first,
                    last,
                }
            }
        }

        impl<'a> Iterator for $iterator_name<'a> {
            type Item = $item_wrapper_ty<'a>;

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
                let item = unsafe {
                    <$item_wrapper_ty as $crate::llvm::types::LLVMTypeWrapper>::from_ptr(item)
                };
                Some(item)
            }
        }
    };
}

llvm_iterator! {
    IterModuleGlobals,
    GlobalsIter,
    Module,
    globals_iter,
    globals_iter_mut,
    LLVMValueRef,
    Value,
    LLVMGetFirstGlobal,
    LLVMGetLastGlobal,
    LLVMGetNextGlobal,
}

llvm_iterator! {
    IterModuleGlobalAliases,
    GlobalAliasesIter,
    Module,
    global_aliases_iter,
    global_aliases_iter_mut,
    LLVMValueRef,
    Value,
    LLVMGetFirstGlobalAlias,
    LLVMGetLastGlobalAlias,
    LLVMGetNextGlobalAlias,
}

llvm_iterator! {
    IterModuleFunctions,
    FunctionsIter,
    Module,
    functions_iter,
    functions_iter_mut,
    LLVMValueRef,
    Function,
    LLVMGetFirstFunction,
    LLVMGetLastFunction,
    LLVMGetNextFunction,
}

llvm_iterator!(
    IterBasicBlocks,
    BasicBlockIter,
    Function,
    basic_blocks_iter,
    basic_blocks_iter_mut,
    LLVMBasicBlockRef,
    BasicBlock,
    LLVMGetFirstBasicBlock,
    LLVMGetLastBasicBlock,
    LLVMGetNextBasicBlock
);

llvm_iterator!(
    IterInstructions,
    InstructionsIter,
    BasicBlock,
    instructions_iter,
    instructions_iter_mut,
    LLVMValueRef,
    Instruction,
    LLVMGetFirstInstruction,
    LLVMGetLastInstruction,
    LLVMGetNextInstruction
);
