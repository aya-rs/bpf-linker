use std::marker::PhantomData;

use llvm_sys::{
    core::{
        LLVMGetFirstBasicBlock, LLVMGetFirstFunction, LLVMGetFirstGlobal, LLVMGetFirstGlobalAlias,
        LLVMGetFirstInstruction, LLVMGetNextBasicBlock, LLVMGetNextFunction, LLVMGetNextGlobal,
        LLVMGetNextGlobalAlias, LLVMGetNextInstruction, LLVMGetPreviousBasicBlock,
        LLVMGetPreviousFunction, LLVMGetPreviousGlobal, LLVMGetPreviousGlobalAlias,
        LLVMGetPreviousInstruction,
    },
    LLVMBasicBlock, LLVMValue,
};

use crate::llvm::types::ir::{
    BasicBlock, Function, GlobalAlias, GlobalVariable, Instruction, Module,
};

macro_rules! llvm_iterator {
    (
        $trait_name:ident,
        $iterator_name:ident,
        $iterable:ident,
        $method_name:ident,
        $ptr_ty:ty,
        $item_ty:ident,
        $first:expr,
        $last:expr,
        $next:expr,
        $prev:expr $(,)?
    ) => {
        pub trait $trait_name {
            fn $method_name(&self) -> $iterator_name;
        }

        pub struct $iterator_name<'ctx> {
            lifetime: PhantomData<&'ctx $iterable<'ctx>>,
            current: Option<::std::ptr::NonNull<$ptr_ty>>,
        }

        impl $trait_name for $iterable<'_> {
            fn $method_name(&self) -> $iterator_name {
                #[allow(unused_imports)]
                use $crate::llvm::types::LLVMTypeWrapper as _;

                let first = unsafe { $first(self.as_ptr()) };
                let first = ::std::ptr::NonNull::new(first);

                $iterator_name {
                    lifetime: PhantomData,
                    current: first,
                }
            }
        }

        impl<'ctx> Iterator for $iterator_name<'ctx> {
            type Item = $item_ty<'ctx>;

            fn next(&mut self) -> Option<Self::Item> {
                #[allow(unused_imports)]
                use $crate::llvm::types::LLVMTypeWrapper as _;

                if let Some(item) = self.current {
                    let next = unsafe { $next(item.as_ptr()) };
                    let next = std::ptr::NonNull::new(next);
                    self.current = next;

                    let item = $item_ty::from_ptr(item).unwrap();

                    Some(item)
                } else {
                    None
                }
            }
        }

        impl<'ctx> DoubleEndedIterator for $iterator_name<'ctx> {
            fn next_back(&mut self) -> Option<Self::Item> {
                #[allow(unused_imports)]
                use $crate::llvm::types::LLVMTypeWrapper as _;

                if let Some(item) = self.current {
                    let prev = unsafe { $prev(item.as_ptr()) };
                    let prev = std::ptr::NonNull::new(prev);
                    self.current = prev;

                    let item = $item_ty::from_ptr(item).unwrap();

                    Some(item)
                } else {
                    None
                }
            }
        }
    };
}

llvm_iterator! {
    IterModuleGlobals,
    GlobalsIter,
    Module,
    globals_iter,
    LLVMValue,
    GlobalVariable,
    LLVMGetFirstGlobal,
    LLVMGetLastGlobal,
    LLVMGetNextGlobal,
    LLVMGetPreviousGlobal,
}

llvm_iterator! {
    IterModuleGlobalAliases,
    GlobalAliasesIter,
    Module,
    global_aliases_iter,
    LLVMValue,
    GlobalAlias,
    LLVMGetFirstGlobalAlias,
    LLVMGetLastGlobalAlias,
    LLVMGetNextGlobalAlias,
    LLVMGetPreviousGlobalAlias,
}

llvm_iterator! {
    IterModuleFunctions,
    FunctionsIter,
    Module,
    functions_iter,
    LLVMValue,
    Function,
    LLVMGetFirstFunction,
    LLVMGetLastFunction,
    LLVMGetNextFunction,
    LLVMGetPreviousFunction,
}

llvm_iterator!(
    IterBasicBlocks,
    BasicBlockIter,
    Function,
    basic_blocks,
    LLVMBasicBlock,
    BasicBlock,
    LLVMGetFirstBasicBlock,
    LLVMGetLastBasicBlock,
    LLVMGetNextBasicBlock,
    LLVMGetPreviousBasicBlock,
);

llvm_iterator!(
    IterInstructions,
    InstructionsIter,
    BasicBlock,
    instructions_iter,
    LLVMValue,
    Instruction,
    LLVMGetFirstInstruction,
    LLVMGetLastInstruction,
    LLVMGetNextInstruction,
    LLVMGetPreviousInstruction
);
