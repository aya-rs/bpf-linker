mod iter;

use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{c_void, CStr, CString},
    os::raw::c_char,
    ptr, slice,
};

use libc::c_char as libc_char;
use llvm_sys::bit_reader::*;
use llvm_sys::core::*;
use llvm_sys::error::*;
use llvm_sys::linker::LLVMLinkModules2;
use llvm_sys::object::*;
use llvm_sys::prelude::*;
use llvm_sys::support::LLVMParseCommandLineOptions;
use llvm_sys::target::*;
use llvm_sys::target_machine::*;
use llvm_sys::transforms::pass_builder::*;
use llvm_sys::LLVMAttributeFunctionIndex;
use llvm_sys::{LLVMLinkage, LLVMVisibility};
use log::*;

use crate::OptLevel;
use iter::{IterModuleFunctions, IterModuleGlobalAliases, IterModuleGlobals};

pub unsafe fn init<T: AsRef<str>>(args: &[T], overview: &str) {
    LLVMInitializeBPFTarget();
    LLVMInitializeBPFTargetMC();
    LLVMInitializeBPFTargetInfo();
    LLVMInitializeBPFAsmPrinter();
    LLVMInitializeBPFAsmParser();
    LLVMInitializeBPFDisassembler();

    parse_command_line_options(args, overview);
}

unsafe fn parse_command_line_options<T: AsRef<str>>(args: &[T], overview: &str) {
    let c_args = args
        .iter()
        .map(|s| CString::new(s.as_ref()).unwrap())
        .collect::<Vec<_>>();
    let c_ptrs = c_args.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
    let overview = CString::new(overview).unwrap();
    LLVMParseCommandLineOptions(c_ptrs.len() as i32, c_ptrs.as_ptr(), overview.as_ptr());
}

pub unsafe fn create_module(name: &str, context: LLVMContextRef) -> Option<LLVMModuleRef> {
    let c_name = CString::new(name).unwrap();
    let module = LLVMModuleCreateWithNameInContext(c_name.as_ptr(), context);

    if module.is_null() {
        return None;
    }

    Some(module)
}

pub unsafe fn find_embedded_bitcode(
    context: LLVMContextRef,
    data: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let buffer_name = CString::new("mem_buffer").unwrap();
    let buffer = LLVMCreateMemoryBufferWithMemoryRange(
        data.as_ptr() as *const libc_char,
        data.len(),
        buffer_name.as_ptr(),
        0,
    );

    let (bin, message) = Message::with(|message| LLVMCreateBinary(buffer, context, message));
    if bin.is_null() {
        return Err(message.as_c_str().unwrap().to_str().unwrap().to_string());
    }

    let mut ret = None;
    let iter = LLVMObjectFileCopySectionIterator(bin);
    while LLVMObjectFileIsSectionIteratorAtEnd(bin, iter) == 0 {
        let name = LLVMGetSectionName(iter);
        if !name.is_null() {
            let name = CStr::from_ptr(name);
            if name.to_str().unwrap() == ".llvmbc" {
                let buf = LLVMGetSectionContents(iter) as *const u8;
                let size = LLVMGetSectionSize(iter) as usize;
                ret = Some(slice::from_raw_parts(buf, size).to_vec());
                break;
            }
        }
        LLVMMoveToNextSection(iter);
    }
    LLVMDisposeSectionIterator(iter);
    LLVMDisposeBinary(bin);
    LLVMDisposeMemoryBuffer(buffer);

    Ok(ret)
}

#[must_use]
pub unsafe fn link_bitcode_buffer(
    context: LLVMContextRef,
    module: LLVMModuleRef,
    buffer: &[u8],
) -> bool {
    let mut linked = false;
    let buffer_name = CString::new("mem_buffer").unwrap();
    let buffer = LLVMCreateMemoryBufferWithMemoryRange(
        buffer.as_ptr() as *const libc_char,
        buffer.len(),
        buffer_name.as_ptr(),
        0,
    );

    let mut temp_module = ptr::null_mut();

    if LLVMParseBitcodeInContext2(context, buffer, &mut temp_module) == 0 {
        linked = LLVMLinkModules2(module, temp_module) == 0;
    }

    LLVMDisposeMemoryBuffer(buffer);

    linked
}

pub unsafe fn target_from_triple(triple: &CStr) -> Result<LLVMTargetRef, String> {
    let mut target = ptr::null_mut();
    let (ret, message) =
        Message::with(|message| LLVMGetTargetFromTriple(triple.as_ptr(), &mut target, message));
    if ret == 0 {
        Ok(target)
    } else {
        Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
    }
}

pub unsafe fn target_from_module(module: LLVMModuleRef) -> Result<LLVMTargetRef, String> {
    let triple = LLVMGetTarget(module);
    target_from_triple(CStr::from_ptr(triple))
}

pub unsafe fn create_target_machine(
    target: LLVMTargetRef,
    triple: &str,
    cpu: &str,
    features: &str,
) -> Option<LLVMTargetMachineRef> {
    let triple = CString::new(triple).unwrap();
    let cpu = CString::new(cpu).unwrap();
    let features = CString::new(features).unwrap();
    let tm = LLVMCreateTargetMachine(
        target,
        triple.as_ptr(),
        cpu.as_ptr(),
        features.as_ptr(),
        LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
        LLVMRelocMode::LLVMRelocDefault,
        LLVMCodeModel::LLVMCodeModelDefault,
    );
    if tm.is_null() {
        None
    } else {
        Some(tm)
    }
}

pub unsafe fn optimize(
    tm: LLVMTargetMachineRef,
    module: LLVMModuleRef,
    opt_level: OptLevel,
    ignore_inline_never: bool,
    export_symbols: &HashSet<Cow<'static, str>>,
) -> Result<(), String> {
    if module_asm_is_probestack(module) {
        LLVMSetModuleInlineAsm2(module, ptr::null_mut(), 0);
    }

    for sym in module.globals_iter() {
        internalize(sym, symbol_name(sym), export_symbols);
    }
    for sym in module.global_aliases_iter() {
        internalize(sym, symbol_name(sym), export_symbols);
    }

    for function in module.functions_iter() {
        let name = symbol_name(function);
        if !name.starts_with("llvm.") {
            if ignore_inline_never {
                remove_attribute(function, "noinline");
            }
            internalize(function, name, export_symbols);
        }
    }

    let passes = [
        // NB: "default<_>" must be the first pass in the list, otherwise it will be ignored.
        match opt_level {
            // Pretty much nothing compiles with -O0 so make it an alias for -O1.
            OptLevel::No | OptLevel::Less => "default<O1>",
            OptLevel::Default => "default<O2>",
            OptLevel::Aggressive => "default<O3>",
            OptLevel::Size => "default<Os>",
            OptLevel::SizeMin => "default<Oz>",
        },
        // NB: This seems to be included in most default pipelines, but not obviously all of them.
        // See
        // https://github.com/llvm/llvm-project/blob/bbe2887f/llvm/lib/Passes/PassBuilderPipelines.cpp#L2011-L2012
        // for a case which includes DCE only conditionally. Better safe than sorry; include it always.
        "dce",
    ];

    let passes = passes.join(",");
    debug!("running passes: {passes}");
    let passes = CString::new(passes).unwrap();
    let options = LLVMCreatePassBuilderOptions();
    let error = LLVMRunPasses(module, passes.as_ptr(), tm, options);
    LLVMDisposePassBuilderOptions(options);
    // Handle the error and print it to stderr.
    if !error.is_null() {
        let error_type_id = LLVMGetErrorTypeId(error);
        // This is the only error type that exists currently, but there might be more in the future.
        assert_eq!(error_type_id, LLVMGetStringErrorTypeId());
        let error_message = LLVMGetErrorMessage(error);
        let error_string = CStr::from_ptr(error_message).to_str().unwrap().to_owned();
        LLVMDisposeErrorMessage(error_message);
        return Err(error_string);
    }

    Ok(())
}

unsafe fn module_asm_is_probestack(module: LLVMModuleRef) -> bool {
    let mut len = 0;
    let ptr = LLVMGetModuleInlineAsm(module, &mut len);
    if ptr.is_null() {
        return false;
    }

    let asm = String::from_utf8_lossy(slice::from_raw_parts(ptr as *const u8, len));
    asm.contains("__rust_probestack")
}

fn symbol_name<'a>(value: *mut llvm_sys::LLVMValue) -> &'a str {
    let mut name_len = 0;
    let ptr = unsafe { LLVMGetValueName2(value, &mut name_len) };
    unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()
}

unsafe fn remove_attribute(function: *mut llvm_sys::LLVMValue, name: &str) {
    let attr = CString::new(name).unwrap();
    let attr_kind = LLVMGetEnumAttributeKindForName(attr.as_ptr(), name.len());
    LLVMRemoveEnumAttributeAtIndex(function, LLVMAttributeFunctionIndex, attr_kind);
}

pub unsafe fn write_ir(module: LLVMModuleRef, output: &CStr) -> Result<(), String> {
    let (ret, message) =
        Message::with(|message| LLVMPrintModuleToFile(module, output.as_ptr(), message));
    if ret == 0 {
        Ok(())
    } else {
        Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
    }
}

pub unsafe fn codegen(
    tm: LLVMTargetMachineRef,
    module: LLVMModuleRef,
    output: &CStr,
    output_type: LLVMCodeGenFileType,
) -> Result<(), String> {
    let (ret, message) = Message::with(|message| {
        LLVMTargetMachineEmitToFile(tm, module, output.as_ptr() as *mut _, output_type, message)
    });
    if ret == 0 {
        Ok(())
    } else {
        Err(message.as_c_str().unwrap().to_str().unwrap().to_string())
    }
}

pub unsafe fn internalize(
    value: LLVMValueRef,
    name: &str,
    export_symbols: &HashSet<Cow<'static, str>>,
) {
    if !name.starts_with("llvm.") && !export_symbols.contains(name) {
        LLVMSetLinkage(value, LLVMLinkage::LLVMInternalLinkage);
        LLVMSetVisibility(value, LLVMVisibility::LLVMDefaultVisibility);
    }
}

pub trait LLVMDiagnosticHandler {
    fn handle_diagnostic(&mut self, severity: llvm_sys::LLVMDiagnosticSeverity, message: &str);
}

pub extern "C" fn diagnostic_handler<T: LLVMDiagnosticHandler>(
    info: LLVMDiagnosticInfoRef,
    handler: *mut c_void,
) {
    let severity = unsafe { LLVMGetDiagInfoSeverity(info) };
    let message = Message {
        ptr: unsafe { LLVMGetDiagInfoDescription(info) },
    };
    let handler = handler as *mut T;
    unsafe { &mut *handler }
        .handle_diagnostic(severity, message.as_c_str().unwrap().to_str().unwrap());
}

pub extern "C" fn fatal_error(reason: *const c_char) {
    error!("fatal error: {:?}", unsafe { CStr::from_ptr(reason) })
}

struct Message {
    ptr: *mut c_char,
}

impl Message {
    fn with<T, F: FnOnce(*mut *mut c_char) -> T>(f: F) -> (T, Self) {
        let mut ptr = ptr::null_mut();
        let t = f(&mut ptr);
        (t, Self { ptr })
    }

    fn as_c_str(&self) -> Option<&CStr> {
        let Self { ptr } = self;
        let ptr = *ptr;
        (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) })
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        let Self { ptr } = self;
        let ptr = *ptr;
        if !ptr.is_null() {
            unsafe {
                LLVMDisposeMessage(ptr);
            }
        }
    }
}
