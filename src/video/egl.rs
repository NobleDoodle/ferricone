//! Linux: VLC's GL context, created next to Slint's EGL context and sharing its objects.

use khronos_egl as egl;
use std::{
    ffi::{CStr, c_void},
    ptr,
};

pub struct SharedContext {
    display: egl::Display,
    context: egl::Context,
}

// EGL handles are valid on any thread; the context is current on one thread at a time.
unsafe impl Send for SharedContext {}
unsafe impl Sync for SharedContext {}

impl SharedContext {
    /// Call while Slint's context is current.
    pub fn from_current() -> Result<Self, String> {
        let egl = egl();
        let display = egl
            .get_current_display()
            .ok_or("Slint has no current EGL display")?;
        let ui = egl
            .get_current_context()
            .ok_or("Slint has no current EGL context")?;
        let client = egl
            .query_context(display, ui, egl::CONTEXT_CLIENT_TYPE)
            .map_err(error)?;
        if client != egl::OPENGL_API as egl::Int {
            return Err("Slint's context is not desktop OpenGL".into());
        }
        let id = egl
            .query_context(display, ui, egl::CONFIG_ID)
            .map_err(error)?;
        let config = egl
            .choose_first_config(display, &[egl::CONFIG_ID, id, egl::NONE])
            .map_err(error)?
            .ok_or("Slint's EGL config is gone")?;
        egl.bind_api(egl::OPENGL_API).map_err(error)?;
        let context = egl
            .create_context(display, config, Some(ui), &[egl::NONE])
            .map_err(error)?;
        Ok(Self { display, context })
    }

    /// Surfaceless: VLC only draws into our framebuffers.
    pub fn make_current(&self) -> bool {
        let egl = egl();
        egl.bind_api(egl::OPENGL_API).is_ok()
            && egl
                .make_current(self.display, None, None, Some(self.context))
                .is_ok()
    }

    pub fn release(&self) -> bool {
        egl().make_current(self.display, None, None, None).is_ok()
    }

    pub fn is_current(&self) -> bool {
        let egl = egl();
        egl.bind_api(egl::OPENGL_API).is_ok() && egl.get_current_context() == Some(self.context)
    }

    pub fn proc_address(&self, name: &CStr) -> *const c_void {
        name.to_str()
            .ok()
            .and_then(|name| egl().get_proc_address(name))
            .map_or(ptr::null(), |f| f as *const c_void)
    }
}

impl Drop for SharedContext {
    fn drop(&mut self) {
        let _ = egl().destroy_context(self.display, self.context);
    }
}

fn egl() -> egl::Instance<egl::Static> {
    egl::Instance::new(egl::Static)
}

fn error(e: egl::Error) -> String {
    format!("EGL: {e}")
}
