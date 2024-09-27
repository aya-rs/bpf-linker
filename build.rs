//! Build script which optionally links libLLVM.

use std::{env, fs::read_dir, path::Path, str::FromStr};

use target_lexicon::{Architecture, Environment, OperatingSystem, Triple};

fn main() {
    // Execute the build script only when custom LLVM path is provided.
    // It's not recommended to trigger that option manually. Using `xtask`
    // should be preferred.
    let llvm_prefix = match env::var("BPF_LINKER_LLVM_PREFIX") {
        Ok(llvm_prefix) => llvm_prefix,
        Err(_) => return,
    };
    println!("cargo::rustc-link-search={llvm_prefix}/lib");

    let target = env::var("TARGET").unwrap();
    let target = Triple::from_str(&target).unwrap();

    if target == target_lexicon::HOST {
        // For native builds, use the standard directories.
        println!("cargo::rustc-link-search=/lib");
        println!("cargo::rustc-link-search=/usr/lib");
        println!("cargo::rustc-link-search=/usr/local/lib");
    } else {
        // For cross builds on platforms supported by cross-llvm, use the
        // expected sysroot directories. Otherwise, fall back to the standard
        // ones.
        match (
            target.architecture,
            target.operating_system,
            target.environment,
        ) {
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Gnu) => {
                println!("cargo::rustc-link-search=/usr/aarch64-linux-gnu/lib");
                println!("cargo::rustc-link-search=/usr/lib/aarch64-linux-gnu");
            }
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Musl) => {
                println!("cargo::rustc-link-arg=--sysroot=/usr/aarch64-unknown-linux-musl");
                println!("cargo::rustc-link-search=/usr/aarch64-unknown-linux-musl/lib");
                println!("cargo::rustc-link-search=/usr/aarch64-unknown-linux-musl/usr/lib");
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Gnu) => {
                println!("cargo::rustc-link-search=/usr/riscv64-linux-gnu/lib");
                println!("cargo::rustc-link-search=/usr/lib/riscv64-linux-gnu");
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Musl) => {
                println!("cargo::rustc-link-arg=--sysroot=/usr/aarch64-unknown-linux-musl");
                println!("cargo::rustc-link-search=/usr/riscv64-unknown-linux-musl/lib");
                println!("cargo::rustc-link-search=/usr/riscv64-unknown-linux-musl/usr/lib");
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Gnu) => {
                println!("cargo::rustc-link-search=/usr/x86_64-linux-gnu/lib");
                println!("cargo::rustc-link-search=/usr/lib/x86_64-linux-gnu");
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Musl) => {
                println!("cargo::rustc-link-arg=--sysroot=/usr/x86_64-unknown-linux-musl");
                println!("cargo::rustc-link-search=/usr/x86_64-unknown-linux-musl/lib");
                println!("cargo::rustc-link-search=/usr/x86_64-unknown-linux-musl/usr/lib");
            }
            (_, _, _) => {
                println!("cargo::rustc-link-search=/lib");
                println!("cargo::rustc-link-search=/usr/lib");
                println!("cargo::rustc-link-search=/usr/local/lib");
            }
        }
    }

    let link_type = if target.environment == Environment::Gnu {
        // On GNU/Linux:
        // - Use libstdc++.
        // - Link system libraries dynamically. The reason being - Debian
        // doesn't ship static zlib and zstd.
        println!("cargo::rustc-link-lib=dylib=stdc++");
        "dylib"
    } else {
        // LLVM libc++ and static linking works fine on other systems (BSDs,
        // macOS, musl/Linux).
        println!("cargo::rustc-link-lib=static=c++_static");
        println!("cargo::rustc-link-lib=static=c++abi");
        "static"
    };

    println!("cargo::rustc-link-lib={link_type}=rt");
    println!("cargo::rustc-link-lib={link_type}=dl");
    println!("cargo::rustc-link-lib={link_type}=m");
    println!("cargo::rustc-link-lib={link_type}=z");
    println!("cargo::rustc-link-lib={link_type}=zstd");

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
            println!("cargo::rustc-link-lib={link_type}={libname}")
        }
    }
}
