//! Placeholder until the WGL (Windows) and CGL (macOS) branches land.

use std::{
    ffi::{CStr, c_void},
    ptr,
};

pub struct SharedContext;

impl SharedContext {
    pub fn from_current() -> Result<Self, String> {
        Err("video output is Linux-only so far".into())
    }

    pub fn make_current(&self) -> bool {
        false
    }

    pub fn release(&self) -> bool {
        false
    }

    pub fn is_current(&self) -> bool {
        false
    }

    pub fn proc_address(&self, _name: &CStr) -> *const c_void {
        ptr::null()
    }
}
