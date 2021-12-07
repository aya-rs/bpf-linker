#![deny(clippy::all)]

extern crate libc;

mod llvm;
mod linker;

pub use linker::*;
