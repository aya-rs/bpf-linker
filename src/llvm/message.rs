use std::{ffi::CStr, ptr};

use libc::c_char;
use llvm_sys::core::LLVMDisposeMessage;

/// Convenient LLVM Message pointer wrapper.
pub struct Message {
    ptr: *mut c_char,
}

impl Message {
    pub fn new() -> Self {
        Self {
            ptr: ptr::null_mut(),
        }
    }

    pub fn from_ptr(ptr: *mut c_char) -> Self {
        Self { ptr }
    }

    pub fn as_c_str(&self) -> Option<&CStr> {
        let Self { ptr } = self;
        if !ptr.is_null() {
            unsafe { Some(CStr::from_ptr(*ptr)) }
        } else {
            None
        }
    }
}

impl std::ops::Deref for Message {
    type Target = *mut c_char;

    fn deref(&self) -> &Self::Target {
        let Self { ptr } = self;
        ptr
    }
}

impl std::ops::DerefMut for Message {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let Self { ptr } = self;
        ptr
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        let Self { ptr } = self;
        if !ptr.is_null() {
            unsafe {
                LLVMDisposeMessage(*ptr);
            }
        }
    }
}
