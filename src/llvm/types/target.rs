use std::{
    ffi::{CStr, CString},
    ptr::{self, NonNull},
};

use llvm_sys::target_machine::{
    LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetMachine, LLVMDisposeTargetMachine,
    LLVMGetTargetFromTriple, LLVMRelocMode, LLVMTargetMachineRef, LLVMTargetRef,
};

use crate::llvm::{types::LLVMTypeWrapper, LLVMError, Message};

pub struct Target {
    target_ref: LLVMTargetRef,
}

impl LLVMTypeWrapper for Target {
    type Target = LLVMTargetRef;

    unsafe fn from_ptr(target_ref: Self::Target) -> Self {
        Self { target_ref }
    }

    fn as_ptr(&self) -> Self::Target {
        self.target_ref
    }
}

impl Target {
    pub fn from_triple(triple: &CStr) -> Result<Self, LLVMError> {
        let mut target_ref = ptr::null_mut();
        let (ret, message) = unsafe {
            Message::with(|message| {
                LLVMGetTargetFromTriple(triple.as_ptr(), &mut target_ref, message)
            })
        };
        if ret == 0 {
            Ok(Self { target_ref })
        } else {
            Err(LLVMError::FailedToResolveTarget(
                triple.to_str().unwrap().to_string(),
                message.as_c_str().unwrap().to_str().unwrap().to_string(),
            ))
        }
    }

    pub fn create_target_machine(
        &self,
        triple: &str,
        cpu: &str,
        features: &str,
    ) -> Option<TargetMachine> {
        let triple = CString::new(triple).unwrap();
        let cpu = CString::new(cpu).unwrap();
        let features = CString::new(features).unwrap();
        let tm = unsafe {
            LLVMCreateTargetMachine(
                self.as_ptr(),
                triple.as_ptr(),
                cpu.as_ptr(),
                features.as_ptr(),
                LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
                LLVMRelocMode::LLVMRelocDefault,
                LLVMCodeModel::LLVMCodeModelDefault,
            )
        };
        NonNull::new(tm).map(|tm| TargetMachine::from_target_machine_ref(tm.as_ptr()))
    }
}

pub struct TargetMachine {
    target_machine_ref: LLVMTargetMachineRef,
}

impl LLVMTypeWrapper for TargetMachine {
    type Target = LLVMTargetMachineRef;

    unsafe fn from_ptr(target_machine_ref: Self::Target) -> Self {
        Self { target_machine_ref }
    }

    fn as_ptr(&self) -> Self::Target {
        self.target_machine_ref
    }
}

impl Drop for TargetMachine {
    fn drop(&mut self) {
        unsafe { LLVMDisposeTargetMachine(self.target_machine_ref) }
    }
}

impl TargetMachine {
    pub fn from_target_machine_ref(target_machine_ref: LLVMTargetMachineRef) -> Self {
        Self { target_machine_ref }
    }
}
