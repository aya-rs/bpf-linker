mod di;
mod iter;
mod types;

use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{CStr, CString},
    os::raw::c_char,
    ptr, slice, str,
};

pub(crate) use di::DISanitizer;
use iter::{IterModuleFunctions as _, IterModuleGlobalAliases as _, IterModuleGlobals as _};
use llvm_sys::{
    LLVMAttributeFunctionIndex, LLVMLinkage, LLVMVisibility,
    bit_reader::LLVMParseBitcodeInContext2,
    core::{
        LLVMCountBasicBlocks, LLVMCreateMemoryBufferWithMemoryRange, LLVMDisposeMemoryBuffer,
        LLVMDisposeMessage, LLVMGetEnumAttributeKindForName, LLVMGetMDString,
        LLVMGetModuleInlineAsm, LLVMGetTarget, LLVMGetValueName2, LLVMIsAFunction,
        LLVMIsAGlobalVariable, LLVMIsDeclaration, LLVMRemoveEnumAttributeAtIndex, LLVMSetLinkage,
        LLVMSetModuleInlineAsm2, LLVMSetSection, LLVMSetVisibility,
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
};
use log::info;
use tracing::{debug, error};
pub(crate) use types::{
    context::{InstalledDiagnosticHandler, LLVMContext},
    memory_buffer::MemoryBuffer,
    module::LLVMModule,
    target_machine::LLVMTargetMachine,
};

use crate::OptLevel;

pub(crate) fn init(args: &[Cow<'_, CStr>], overview: &CStr) {
    unsafe {
        LLVMInitializeBPFTarget();
        LLVMInitializeBPFTargetMC();
        LLVMInitializeBPFTargetInfo();
        LLVMInitializeBPFAsmPrinter();
        LLVMInitializeBPFAsmParser();
        LLVMInitializeBPFDisassembler();
    }

    let c_ptrs = args.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
    unsafe {
        LLVMParseCommandLineOptions(
            c_ptrs.len().try_into().unwrap(),
            c_ptrs.as_ptr(),
            overview.as_ptr(),
        )
    };
}

pub(crate) fn find_embedded_bitcode(
    context: &LLVMContext,
    data: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let buffer_name = c"mem_buffer";
    let buffer = unsafe {
        LLVMCreateMemoryBufferWithMemoryRange(
            data.as_ptr().cast(),
            data.len(),
            buffer_name.as_ptr(),
            0,
        )
    };

    let (bin, message) =
        Message::with(|message| unsafe { LLVMCreateBinary(buffer, context.as_mut_ptr(), message) });
    if bin.is_null() {
        return Err(message.as_string_lossy().to_string());
    }

    let mut ret = None;
    let iter = unsafe { LLVMObjectFileCopySectionIterator(bin) };
    while unsafe { LLVMObjectFileIsSectionIteratorAtEnd(bin, iter) } == 0 {
        let name = unsafe { LLVMGetSectionName(iter) };
        if !name.is_null() {
            let name = unsafe { CStr::from_ptr(name) };
            if name == c".llvmbc" {
                let buf = unsafe { LLVMGetSectionContents(iter) };
                let size = unsafe { LLVMGetSectionSize(iter) }.try_into().unwrap();
                ret = Some(unsafe { slice::from_raw_parts(buf.cast(), size).to_vec() });
                break;
            }
        }
        unsafe { LLVMMoveToNextSection(iter) };
    }
    unsafe { LLVMDisposeSectionIterator(iter) };
    unsafe { LLVMDisposeBinary(bin) };
    unsafe { LLVMDisposeMemoryBuffer(buffer) };

    Ok(ret)
}

#[must_use]
pub(crate) fn link_bitcode_buffer<'ctx>(
    context: &'ctx LLVMContext,
    module: &mut LLVMModule<'ctx>,
    buffer: &[u8],
) -> bool {
    let mut linked = false;
    let buffer_name = c"mem_buffer";
    let buffer = unsafe {
        LLVMCreateMemoryBufferWithMemoryRange(
            buffer.as_ptr().cast(),
            buffer.len(),
            buffer_name.as_ptr(),
            0,
        )
    };

    let mut temp_module = ptr::null_mut();

    if unsafe { LLVMParseBitcodeInContext2(context.as_mut_ptr(), buffer, &mut temp_module) } == 0 {
        linked = unsafe { LLVMLinkModules2(module.as_mut_ptr(), temp_module) } == 0;
    }

    unsafe { LLVMDisposeMemoryBuffer(buffer) };

    linked
}

pub(crate) fn target_from_triple(triple: &CStr) -> Result<LLVMTargetRef, String> {
    let mut target = ptr::null_mut();
    let (ret, message) = Message::with(|message| unsafe {
        LLVMGetTargetFromTriple(triple.as_ptr(), &mut target, message)
    });
    if ret == 0 {
        Ok(target)
    } else {
        Err(message.as_string_lossy().to_string())
    }
}

pub(crate) fn target_from_module(module: &LLVMModule<'_>) -> Result<LLVMTargetRef, String> {
    let triple = unsafe { LLVMGetTarget(module.as_mut_ptr()) };
    unsafe { target_from_triple(CStr::from_ptr(triple)) }
}

pub(crate) fn optimize(
    tm: &LLVMTargetMachine,
    module: &mut LLVMModule<'_>,
    opt_level: OptLevel,
    ignore_inline_never: bool,
    export_symbols: &HashSet<Cow<'_, [u8]>>,
) -> Result<(), String> {
    if module_asm_is_probestack(module.as_mut_ptr()) {
        unsafe { LLVMSetModuleInlineAsm2(module.as_mut_ptr(), ptr::null_mut(), 0) };
    }

    for sym in module.as_mut_ptr().globals_iter() {
        internalize(sym, symbol_name(sym), export_symbols);
    }
    for sym in module.as_mut_ptr().global_aliases_iter() {
        internalize(sym, symbol_name(sym), export_symbols);
    }

    for function in module.as_mut_ptr().functions_iter() {
        let name = symbol_name(function);
        if !name.starts_with(b"llvm.") {
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
    let options = unsafe { LLVMCreatePassBuilderOptions() };
    let error = unsafe {
        LLVMRunPasses(
            module.as_mut_ptr(),
            passes.as_ptr(),
            tm.as_mut_ptr(),
            options,
        )
    };
    unsafe { LLVMDisposePassBuilderOptions(options) };
    // Handle the error and print it to stderr.
    if !error.is_null() {
        let error_type_id = unsafe { LLVMGetErrorTypeId(error) };
        // This is the only error type that exists currently, but there might be more in the future.
        assert_eq!(error_type_id, unsafe { LLVMGetStringErrorTypeId() });
        let error_message = unsafe { LLVMGetErrorMessage(error) };
        let error_string = unsafe { CStr::from_ptr(error_message) }
            .to_string_lossy()
            .to_string();
        unsafe { LLVMDisposeErrorMessage(error_message) };
        return Err(error_string);
    }

    Ok(())
}

pub(crate) fn module_asm_is_probestack(module: LLVMModuleRef) -> bool {
    let mut len = 0;
    let ptr = unsafe { LLVMGetModuleInlineAsm(module, &mut len) };
    if ptr.is_null() {
        return false;
    }

    let needle = b"__rust_probestack";
    let haystack: &[u8] = unsafe { slice::from_raw_parts(ptr.cast(), len) };
    haystack.windows(needle.len()).any(|w| w == needle)
}

pub(crate) fn symbol_name<'a>(value: *mut llvm_sys::LLVMValue) -> &'a [u8] {
    let mut name_len = 0;
    let ptr = unsafe { LLVMGetValueName2(value, &mut name_len) };
    unsafe { slice::from_raw_parts(ptr.cast(), name_len) }
}

pub(crate) fn remove_attribute(function: *mut llvm_sys::LLVMValue, name: &str) {
    let attr_kind = unsafe { LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len()) };
    unsafe { LLVMRemoveEnumAttributeAtIndex(function, LLVMAttributeFunctionIndex, attr_kind) };
}

pub(crate) fn internalize(
    value: LLVMValueRef,
    name: &[u8],
    export_symbols: &HashSet<Cow<'_, [u8]>>,
) {
    if !name.starts_with(b"llvm.") && !export_symbols.contains(name) {
        if unsafe { !LLVMIsAFunction(value).is_null() } {
            let num_blocks = unsafe { LLVMCountBasicBlocks(value) };
            if num_blocks == 0 {
                unsafe { LLVMSetSection(value, c".ksyms".as_ptr()) };
                info!(
                    "not internalizing undefined function {}",
                    str::from_utf8(name).unwrap_or("<invalid utf8>")
                );
                return;
            }
        }
        if unsafe { !LLVMIsAGlobalVariable(value).is_null() } {
            if unsafe { LLVMIsDeclaration(value) != 0 } {
                unsafe { LLVMSetSection(value, c".ksyms".as_ptr()) };
                info!(
                    "not internalizing undefined global variable {}",
                    str::from_utf8(name).unwrap_or("<invalid utf8>")
                );
                return;
            }
        }

        unsafe { LLVMSetLinkage(value, LLVMLinkage::LLVMInternalLinkage) };
        unsafe { LLVMSetVisibility(value, LLVMVisibility::LLVMDefaultVisibility) };
    }
}

pub(crate) trait LLVMDiagnosticHandler {
    fn handle_diagnostic(
        &mut self,
        severity: llvm_sys::LLVMDiagnosticSeverity,
        message: Cow<'_, str>,
    );
}

pub(crate) extern "C" fn fatal_error(reason: *const c_char) {
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

    fn as_string_lossy(&self) -> Cow<'_, str> {
        self.as_c_str()
            .map(CStr::to_bytes)
            .map(String::from_utf8_lossy)
            .unwrap_or("<null>".into())
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
