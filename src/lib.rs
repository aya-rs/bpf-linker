#![deny(clippy::all)]
#![deny(unused_results)]

mod linker;
pub mod llvm;

pub use linker::*;
