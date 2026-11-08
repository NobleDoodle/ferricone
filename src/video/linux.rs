//! Linux: VLC's GL context, created next to Slint's and sharing its objects.
//! Slint draws with EGL on Wayland and with GLX on X11 (glutin prefers the
//! platform's own API), so both are handled.

#[path = "egl.rs"]
mod egl;
#[path = "glx.rs"]
mod glx;

use std::ffi::{CStr, c_void};

pub enum SharedContext {
    Egl(egl::SharedContext),
    Glx(glx::SharedContext),
}

impl SharedContext {
    /// Call while Slint's context is current.
    pub fn from_current() -> Result<Self, String> {
        egl::SharedContext::from_current()
            .map(Self::Egl)
            .or_else(|egl| {
                glx::SharedContext::from_current()
                    .map(Self::Glx)
                    .map_err(|glx| format!("{egl}; {glx}"))
            })
    }

    pub fn make_current(&self) -> bool {
        match self {
            Self::Egl(c) => c.make_current(),
            Self::Glx(c) => c.make_current(),
        }
    }

    pub fn release(&self) -> bool {
        match self {
            Self::Egl(c) => c.release(),
            Self::Glx(c) => c.release(),
        }
    }

    pub fn is_current(&self) -> bool {
        match self {
            Self::Egl(c) => c.is_current(),
            Self::Glx(c) => c.is_current(),
        }
    }

    pub fn proc_address(&self, name: &CStr) -> *const c_void {
        match self {
            Self::Egl(c) => c.proc_address(name),
            Self::Glx(c) => c.proc_address(name),
        }
    }
}
