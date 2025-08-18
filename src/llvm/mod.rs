mod di;
mod iter;

mod types;

use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{c_uchar, CStr, CString},
    os::raw::c_char,
    ptr, slice, str,
};

pub use di::DISanitizer;
use iter::{IterModuleFunctions, IterModuleGlobalAliases, IterModuleGlobals};
use libc::c_char as libc_char;
use llvm_sys::{
    bit_reader::LLVMParseBitcodeInContext2,
    core::{
        LLVMCreateMemoryBufferWithMemoryRange, LLVMDisposeMemoryBuffer, LLVMDisposeMessage,
        LLVMGetEnumAttributeKindForName, LLVMGetMDString, LLVMGetModuleInlineAsm, LLVMGetTarget,
        LLVMGetValueName2, LLVMRemoveEnumAttributeAtIndex, LLVMSetLinkage, LLVMSetModuleInlineAsm2,
        LLVMSetVisibility,
    },
    error::{
        LLVMDisposeErrorMessage, LLVMGetErrorMessage, LLVMGetErrorTypeId, LLVMGetStringErrorTypeId,
    },
    linker::LLVMLinkModules2,
    object::{
        LLVMCreateBinary, LLVMDisposeBinary, LLVMDisposeSectionIterator, LLVMGetSectionContents,
        LLVMGetSectionName, LLVMGetSectionSize, LLVMMoveToNextSection,
        LLVMObjectFileCopySectionIterator, LLVMObjectFileIsSectionIteratorAtEnd,
    },
    prelude::{LLVMModuleRef, LLVMValueRef},
    support::LLVMParseCommandLineOptions,
    target::{
        LLVMInitializeBPFAsmParser, LLVMInitializeBPFAsmPrinter, LLVMInitializeBPFDisassembler,
        LLVMInitializeBPFTarget, LLVMInitializeBPFTargetInfo, LLVMInitializeBPFTargetMC,
    },
    target_machine::{LLVMGetTargetFromTriple, LLVMTargetRef},
    transforms::pass_builder::{
        LLVMCreatePassBuilderOptions, LLVMDisposePassBuilderOptions, LLVMRunPasses,
    },
    LLVMAttributeFunctionIndex, LLVMLinkage, LLVMVisibility,
};
use tracing::{debug, error};
pub use types::{
    context::LLVMContext, memory_buffer::MemoryBuffer, module::LLVMModule,
    target_machine::LLVMTargetMachine,
};

use crate::OptLevel;

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

pub unsafe fn find_embedded_bitcode(
    context: &LLVMContext,
    data: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let buffer_name = CString::new("mem_buffer").unwrap();
    let buffer = LLVMCreateMemoryBufferWithMemoryRange(
        data.as_ptr() as *const libc_char,
        data.len(),
        buffer_name.as_ptr(),
        0,
    );

    let (bin, message) =
        Message::with(|message| LLVMCreateBinary(buffer, unsafe { context.as_raw() }, message));
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
                let buf = LLVMGetSectionContents(iter);
                let size = LLVMGetSectionSize(iter) as usize;
                ret = Some(slice::from_raw_parts(buf as *const c_uchar, size).to_vec());
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
pub unsafe fn link_bitcode_buffer<'ctx>(
    context: &'ctx LLVMContext,
    module: &mut LLVMModule<'ctx>,
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

    if LLVMParseBitcodeInContext2(unsafe { context.as_raw() }, buffer, &mut temp_module) == 0 {
        linked = LLVMLinkModules2(unsafe { module.as_raw() }, temp_module) == 0;
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

pub unsafe fn target_from_module(module: &LLVMModule) -> Result<LLVMTargetRef, String> {
    let triple = LLVMGetTarget(unsafe { module.as_raw() });
    target_from_triple(CStr::from_ptr(triple))
}

pub unsafe fn optimize(
    tm: &LLVMTargetMachine,
    module: &mut LLVMModule,
    opt_level: OptLevel,
    ignore_inline_never: bool,
    export_symbols: &HashSet<Cow<'static, str>>,
) -> Result<(), String> {
    if module_asm_is_probestack(unsafe { module.as_raw() }) {
        LLVMSetModuleInlineAsm2(unsafe { module.as_raw() }, ptr::null_mut(), 0);
    }

    for sym in unsafe { module.as_raw() }.globals_iter() {
        internalize(sym, symbol_name(sym), export_symbols);
    }
    for sym in unsafe { module.as_raw() }.global_aliases_iter() {
        internalize(sym, symbol_name(sym), export_symbols);
    }

    for function in unsafe { module.as_raw() }.functions_iter() {
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
    let error = LLVMRunPasses(
        unsafe { module.as_raw() },
        passes.as_ptr(),
        unsafe { tm.as_raw() },
        options,
    );
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

    let asm = String::from_utf8_lossy(slice::from_raw_parts(ptr as *const c_uchar, len));
    asm.contains("__rust_probestack")
}

fn symbol_name<'a>(value: *mut llvm_sys::LLVMValue) -> &'a str {
    let mut name_len = 0;
    let ptr = unsafe { LLVMGetValueName2(value, &mut name_len) };
    unsafe { str::from_utf8(slice::from_raw_parts(ptr as *const c_uchar, name_len)).unwrap() }
}

unsafe fn remove_attribute(function: *mut llvm_sys::LLVMValue, name: &str) {
    let attr_kind = LLVMGetEnumAttributeKindForName(name.as_ptr() as *const c_char, name.len());
    LLVMRemoveEnumAttributeAtIndex(function, LLVMAttributeFunctionIndex, attr_kind);
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

fn mdstring_to_str<'a>(mdstring: LLVMValueRef) -> &'a str {
    let mut len = 0;
    let ptr = unsafe { LLVMGetMDString(mdstring, &mut len) };
    unsafe { str::from_utf8(slice::from_raw_parts(ptr as *const c_uchar, len as usize)).unwrap() }
}
