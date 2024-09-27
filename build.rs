//! Build script which optionally links libLLVM.

use std::{env, fs::read_dir, path::Path, str::FromStr};

use target_lexicon::{Architecture, Environment, OperatingSystem, Triple};

fn main() {
    // Execute the build script only when custom LLVM path is provided.
    let llvm_prefix = match env::var("BPF_LINKER_LLVM_PREFIX") {
        Ok(llvm_prefix) => llvm_prefix,
        Err(_) => return,
    };
    println!("cargo::rustc-link-search={llvm_prefix}/lib");

    let target = env::var("TARGET").unwrap();
    let target = Triple::from_str(&target).unwrap();

    // Use the expected sysroot directories. Otherwise, fall back to the
    // standard ones.
    match (
        target.architecture,
        target.operating_system,
        target.environment,
    ) {
        (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Musl) => {
            println!("cargo::rustc-link-arg=--sysroot=/usr/aarch64-unknown-linux-musl");
            println!("cargo::rustc-link-search=/usr/aarch64-unknown-linux-musl/lib");
            println!("cargo::rustc-link-search=/usr/aarch64-unknown-linux-musl/usr/lib");
        }
        (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Musl) => {
            println!("cargo::rustc-link-arg=--sysroot=/usr/aarch64-unknown-linux-musl");
            println!("cargo::rustc-link-search=/usr/riscv64-unknown-linux-musl/lib");
            println!("cargo::rustc-link-search=/usr/riscv64-unknown-linux-musl/usr/lib");
        }
        (Architecture::X86_64, OperatingSystem::Linux, Environment::Musl) => {
            println!("cargo::rustc-link-arg=--sysroot=/usr/x86_64-unknown-linux-musl");
            println!("cargo::rustc-link-search=/usr/x86_64-unknown-linux-musl/lib");
            println!("cargo::rustc-link-search=/usr/x86_64-unknown-linux-musl/usr/lib");
        }
        (_, _, _) => {
            panic!("Unsupported target. Please use `LLVM_SYS_*_PREFIX` and llvm-sys (without `no-llvm-linking` feature) to link LLVM.")
        }
    }

    println!("cargo::rustc-link-lib=static=c++_static");
    println!("cargo::rustc-link-lib=static=c++abi");

    println!("cargo::rustc-link-lib=static=rt");
    println!("cargo::rustc-link-lib=static=dl");
    println!("cargo::rustc-link-lib=static=m");
    println!("cargo::rustc-link-lib=static=z");
    println!("cargo::rustc-link-lib=static=zstd");

    // Link libLLVM using the artifacts from the provided prefix.
    for entry in
        read_dir(Path::new(&llvm_prefix).join("lib")).expect("LLVM build directory not found")
    {
        let entry = entry.expect("failed to retrieve the file in the LLVM build directory");
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("a") {
            let libname = path.file_name().unwrap().to_string_lossy();
            let libname = libname
                .strip_prefix("lib")
                .unwrap()
                .strip_suffix(".a")
                .unwrap();
            println!("cargo::rustc-link-lib=static={libname}")
        }
    }
}
