use std::marker::PhantomData;

use llvm_sys::core::*;
use llvm_sys::prelude::*;

macro_rules! llvm_iterator {
    ($trait_name:ident, $iterator_name:ident, $iterable:ty, $method_name:ident, $item_ty:ty, $first:expr, $next:expr) => {
        pub trait $trait_name {
            fn $method_name(&self) -> $iterator_name;
        }

        impl $trait_name for $iterable {
            fn $method_name(&self) -> $iterator_name {
                $iterator_name {
                    iterable: PhantomData,
                    next: unsafe { $first(*self) },
                }
            }
        }

        pub struct $iterator_name<'a> {
            iterable: PhantomData<&'a $iterable>,
            next: $item_ty,
        }

        impl<'a> Iterator for $iterator_name<'a> {
            type Item = $item_ty;

            fn next(&mut self) -> Option<Self::Item> {
                let next = self.next;
                if !next.is_null() {
                    self.next = unsafe { $next(next) };
                    Some(next)
                } else {
                    None
                }
            }
        }
    };
}

llvm_iterator!(
    IterModuleFunctions,
    FunctionsIter,
    LLVMModuleRef,
    functions_iter,
    LLVMValueRef,
    LLVMGetFirstFunction,
    LLVMGetNextFunction
);

llvm_iterator!(
    IterModuleGlobals,
    GlobalsIter,
    LLVMModuleRef,
    globals_iter,
    LLVMValueRef,
    LLVMGetFirstGlobal,
    LLVMGetNextGlobal
);

llvm_iterator!(
    IterModuleGlobalAliases,
    GlobalAliasessIter,
    LLVMModuleRef,
    global_aliases_iter,
    LLVMValueRef,
    LLVMGetFirstGlobalAlias,
    LLVMGetNextGlobalAlias
);