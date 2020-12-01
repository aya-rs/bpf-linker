use super::SHARED_LIB;
use llvm_sys::prelude::LLVMBool;

use std::io::{BufRead, BufReader, Result};
use std::process::Command;

const POSSIBLE_BACKENDS: &[&str] = &[
    "AArch64", "AMDGPU", "ARM", "BPF", "Hexagon", "Lanai", "Mips", "MSP430", "NVPTX", "PowerPC",
    "Sparc", "SystemZ", "X86", "XCore",
];

fn get_native_arch() -> Result<String> {
    let output = Command::new("rustc").args(&["--print", "cfg"]).output()?;
    let buf = BufReader::new(output.stdout.as_slice());
    for line in buf.lines() {
        let line = line?;
        if !line.starts_with("target_arch") {
            continue;
        }
        // line should be like: target_arch="x86_64"
        return Ok(line.split('"').nth(1).unwrap().into());
    }
    unreachable!("`rustc --print cfg` result is wrong");
}

fn arch2backend(arch: &str) -> String {
    match arch {
        "aarch64" => "AArch64".into(),
        "arm" => "ARM".into(),
        "mips" | "mips64" => "Mips".into(),
        "powerpc" | "powerpc64" => "PowerPC".into(),
        "sparc" | "sparc64" => "Sparc".into(),
        "x86" | "x86_64" => "X86".into(),
        _ => panic!("Unknown backend: {}", arch),
    }
}

fn get_native_backend() -> String {
    let arch = get_native_arch().expect("Fail to get native arch");
    arch2backend(&arch)
}

unsafe fn init_all(postfix: &str) {
    for backend in POSSIBLE_BACKENDS {
        let name = format!("LLVMInitialize{}{}", backend, postfix);
        if let Ok(entrypoint) = SHARED_LIB.get::<unsafe extern "C" fn()>(name.as_bytes()) {
            entrypoint();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeAllTargetInfos() {
    init_all("TargetInfo");
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeAllTargets() {
    init_all("Target");
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeAllTargetMCs() {
    init_all("TargetMC");
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeAllAsmParsers() {
    init_all("AsmParser");
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeAllAsmPrinters() {
    init_all("AsmPrinter");
}

unsafe fn init_native(postfix: &str) -> LLVMBool {
    let backend = get_native_backend();
    let name = format!("LLVMInitialize{}{}", backend, postfix);
    if let Ok(entrypoint) = SHARED_LIB.get::<unsafe extern "C" fn()>(name.as_bytes()) {
        entrypoint();
        0
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeNativeTarget() -> LLVMBool {
    init_native("Target")
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeNativeAsmParser() -> LLVMBool {
    init_native("AsmParser")
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeNativeAsmPrinter() -> LLVMBool {
    init_native("AsmPrinter")
}
#[no_mangle]
pub unsafe extern "C" fn LLVM_InitializeNativeDisassembler() -> LLVMBool {
    init_native("Disassembler")
}
