#![deny(warnings)]
#![allow(non_snake_case, unused_imports, unused_macros, deprecated)]

//! Dynamically proxy LLVM calls into Rust own shared library! 🎉
//!
//! ## Use cases
//! Normally there is no much need for the crate, except a couple of exotic cases:
//!
//! * Your crate is some kind build process helper that leverages LLVM (e.g. [ptx-linker](https://github.com/denzp/rust-ptx-linker)),
//! * Your crate needs to stay up to date with Rust LLVM version (again [ptx-linker](https://github.com/denzp/rust-ptx-linker)),
//! * You would prefer not to have dependencies on host LLVM libs (as always [ptx-linker](https://github.com/denzp/rust-ptx-linker)).
//!
//! ## Usage
//! First, you need to make sure no other crate links your binary against system LLVM library.
//! In case you are using `llvm-sys`, this can be achieved with a special feature:
//!
//! ``` toml
//! [dependencies.llvm-sys]
//! version = "70"
//! features = ["no-llvm-linking"]
//! ```
//!
//! Then all you need to do is to include the crate into your project:
//!
//! ``` toml
//! [dependencies]
//! rustc-llvm-proxy = "0.1"
//! ```
//!
//! ``` rust
//! extern crate rustc_llvm_proxy;
//! ```

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate failure;

extern crate libc;
extern crate libloading as lib;
extern crate llvm_sys;

use lib::Library;

mod path;
use path::find_lib_path;

pub mod init;

lazy_static! {
    static ref SHARED_LIB: Library = {
        let lib_path = match find_lib_path() {
            Ok(path) => path,

            Err(error) => {
                eprintln!("{}", error);
                panic!();
            }
        };

        match Library::new(lib_path) {
            Ok(path) => path,

            Err(error) => {
                eprintln!("Unable to open LLVM shared lib: {}", error);
                panic!();
            }
        }
    };
}

/// Commonly used LLVM CAPI symbols with dynamic resolving
pub mod proxy {
    use super::SHARED_LIB;

    use llvm_sys::analysis::*;
    use llvm_sys::debuginfo::*;
    use llvm_sys::disassembler::*;
    use llvm_sys::error::*;
    use llvm_sys::error_handling::*;
    use llvm_sys::execution_engine::*;
    use llvm_sys::lto::*;
    use llvm_sys::object::*;
    use llvm_sys::orc2::ee::*;
    use llvm_sys::orc2::lljit::*;
    use llvm_sys::orc2::*;
    use llvm_sys::prelude::*;
    use llvm_sys::target::*;
    use llvm_sys::target_machine::*;
    use llvm_sys::transforms::pass_manager_builder::*;
    use llvm_sys::*;

    macro_rules! create_proxy {
        ($name:ident ; $ret_ty:ty ; $($arg:ident : $arg_ty:ty),*) => {
            #[no_mangle]
            pub unsafe extern "C" fn $name($($arg: $arg_ty),*) -> $ret_ty {
                let entrypoint = {
                    SHARED_LIB
                        .get::<unsafe extern "C" fn($($arg: $arg_ty),*) -> $ret_ty>(stringify!($name).as_bytes())
                };

                match entrypoint {
                    Ok(entrypoint) => entrypoint($($arg),*),

                    Err(_) => {
                        eprintln!("Unable to find symbol '{}' in the LLVM shared lib", stringify!($name));
                        panic!();
                    }
                }
            }
        };
    }

    include!(concat!(env!("OUT_DIR"), "/llvm_gen.rs"));
}
