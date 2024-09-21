#[cfg(not(feature = "rust-llvm"))]
use std::{env, fs, path::Path};

#[cfg(not(feature = "rust-llvm"))]
fn main() {
    // Retrieve the directory where we are going to look for static LLVM
    // libraries.
    let libdir = env::var("BPF_LINKER_LLVM_LIBDIR")
        .expect("Please provide BPF_LINKER_LLVM_LIBDIR, which points to static LLVM libraries");

    // Add the directory to the linker search path.
    println!("cargo:rustc-link-search=native={}", libdir);

    let libdir_path = Path::new(&libdir);
    if !libdir_path.exists() {
        panic!("Directory {} does not exist", libdir);
    }

    // Find all *.a files and link them.
    for entry in fs::read_dir(libdir_path).expect("Could not read directory") {
        let entry = entry.expect("Could not get directory entry");
        let path = entry.path();

        let ext = match path.extension() {
            Some(ext) => ext,
            None => continue,
        };
        if ext != "a" {
            continue;
        }

        let file_name = match path.file_name() {
            Some(file_name) => file_name,
            None => continue,
        };
        let file_name = match file_name.to_str() {
            Some(file_name) => file_name,
            None => continue,
        };
        // Strip "lib" prefix and ".a" suffix.
        if file_name.starts_with("lib") && file_name.ends_with(".a") {
            let lib_name = &file_name[3..file_name.len() - 2];
            println!("cargo:rustc-link-lib=static={}", lib_name);
        }
    }
}

#[cfg(feature = "rust-llvm")]
fn main() {}
