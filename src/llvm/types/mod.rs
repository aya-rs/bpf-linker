use std::ptr::NonNull;

use thiserror::Error;

pub mod di;
pub mod ir;

#[derive(Debug, Error)]
pub enum LLVMTypeError {
    #[error("invalid pointer type, expected {0}")]
    InvalidPointerType(&'static str),
    #[error("null pointer")]
    NullPointer,
}

pub trait LLVMTypeWrapper {
    type Target: Sized;

    /// Constructs a new [`Self`] from the given pointer `ptr`.
    fn from_ptr(ptr: NonNull<Self::Target>) -> Result<Self, LLVMTypeError>
    where
        Self: Sized;
    fn as_ptr(&self) -> *mut Self::Target;
}
