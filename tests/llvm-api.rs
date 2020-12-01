extern crate llvm_sys;
extern crate rustc_llvm_proxy;

use llvm_sys::core::*;
use std::ffi::{CStr, CString};

#[test]
fn module_creation() {
    unsafe {
        let module_name = CString::new("test module").unwrap();
        let module = LLVMModuleCreateWithName(module_name.as_ptr());

        let data_layout = CString::new("e-i64:64-v16:16-v32:32-n16:32:64").unwrap();
        LLVMSetDataLayout(module, data_layout.as_ptr());

        let module_contents_raw = LLVMPrintModuleToString(module);
        let module_contents = CStr::from_ptr(module_contents_raw);

        assert_eq!(
            module_contents.to_str().unwrap(),
            r#"; ModuleID = 'test module'
source_filename = "test module"
target datalayout = "e-i64:64-v16:16-v32:32-n16:32:64"
"#
        );

        LLVMDisposeMessage(module_contents_raw);
        LLVMDisposeModule(module);
    }
}
