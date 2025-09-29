// The following macros are adapted from from https://github.com/TheDan64/inkwell,
// licensed under Apache-2.0.
// Original source: https://github.com/TheDan64/inkwell/blob/0b0a2c0b2eb5e458767093c2ab8c56cbd05ec4c9/src/lib.rs#L85-L112

#![expect(unused_crate_dependencies, reason = "used in bin")]

macro_rules! assert_unique_features {
    () => {};
    ($first:tt $(,$rest:tt)*) => {
        $(
            #[cfg(all(feature = $first, feature = $rest))]
            compile_error!(concat!("features \"", $first, "\" and \"", $rest, "\" cannot be used together"));
        )*
        assert_unique_features!($($rest),*);
    }
}

macro_rules! assert_used_features {
    ($($all:tt),*) => {
        #[cfg(not(any($(feature = $all),*)))]
        compile_error!(concat!("One of the LLVM feature flags must be provided: ", $($all, " "),*));
    }
}

macro_rules! assert_unique_used_features {
    ($($all:tt),*) => {
        assert_unique_features!($($all),*);
        assert_used_features!($($all),*);
    }
}

assert_unique_used_features! {
    "llvm-19",
    "llvm-20",
    "llvm-21"
}

#[cfg(feature = "llvm-19")]
pub extern crate llvm_sys_19 as llvm_sys;
#[cfg(feature = "llvm-20")]
pub extern crate llvm_sys_20 as llvm_sys;
#[cfg(feature = "llvm-21")]
pub extern crate llvm_sys_21 as llvm_sys;

mod linker;
mod llvm;

pub use linker::*;
