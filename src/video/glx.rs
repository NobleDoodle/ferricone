//! GLX (X11, where glutin prefers it to EGL): VLC's GL context, created next
//! to Slint's with the same framebuffer config and sharing its objects.
//!
//! VLC only draws into our framebuffers, so the context needs no window. It is
//! made current without a drawable where the driver allows that (Mesa, and
//! GL 3.0+ contexts elsewhere), and with a 1×1 pbuffer otherwise.
//!
//! The display is winit's, which called XInitThreads and installed an error
//! handler that logs instead of exiting, so a refused call here returns false.

use std::{
    ffi::{CStr, c_int, c_ulong, c_void},
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use x11_dl::{
    glx::{self, GLXContext, GLXFBConfig, Glx},
    xlib::{self, Display, Xlib},
};

pub struct SharedContext {
    display: *mut Display,
    config: GLXFBConfig,
    context: GLXContext,
    /// 0 while making the context current without a drawable works.
    pbuffer: AtomicU64,
}

// Xlib is thread-safe after XInitThreads; the context is current on one
// thread at a time.
unsafe impl Send for SharedContext {}
unsafe impl Sync for SharedContext {}

struct Libs {
    glx: Glx,
    xlib: Xlib,
}

fn libs() -> Result<&'static Libs, String> {
    static LIBS: OnceLock<Result<Libs, String>> = OnceLock::new();
    LIBS.get_or_init(|| {
        Ok(Libs {
            glx: Glx::open().map_err(|e| format!("GLX: {e}"))?,
            xlib: Xlib::open().map_err(|e| format!("Xlib: {e}"))?,
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

impl SharedContext {
    /// Call while Slint's context is current.
    pub fn from_current() -> Result<Self, String> {
        let Libs { glx, xlib } = libs()?;
        unsafe {
            let display = (glx.glXGetCurrentDisplay)();
            let ui = (glx.glXGetCurrentContext)();
            if display.is_null() || ui.is_null() {
                return Err("Slint has no current GLX context".into());
            }
            let query = |attribute| {
                let mut value = 0;
                let status = (glx.glXQueryContext)(display, ui, attribute, &mut value);
                (status == xlib::Success as c_int).then_some(value)
            };
            let (Some(id), Some(screen)) = (query(glx::GLX_FBCONFIG_ID), query(glx::GLX_SCREEN))
            else {
                return Err("GLX: cannot read Slint's framebuffer config".into());
            };
            let wanted = [glx::GLX_FBCONFIG_ID, id, 0];
            let mut count = 0;
            let configs = (glx.glXChooseFBConfig)(display, screen, wanted.as_ptr(), &mut count);
            if configs.is_null() {
                return Err("GLX: Slint's framebuffer config is gone".into());
            }
            let config = (count > 0).then(|| *configs);
            (xlib.XFree)(configs.cast());
            let config = config.ok_or("GLX: Slint's framebuffer config is gone")?;
            let context =
                (glx.glXCreateNewContext)(display, config, glx::GLX_RGBA_TYPE, ui, xlib::True);
            if context.is_null() {
                return Err("GLX: cannot create a context that shares Slint's".into());
            }
            Ok(Self {
                display,
                config,
                context,
                pbuffer: AtomicU64::new(0),
            })
        }
    }

    pub fn make_current(&self) -> bool {
        let Ok(Libs { glx, .. }) = libs() else { return false };
        let current = |drawable: c_ulong| unsafe {
            (glx.glXMakeContextCurrent)(self.display, drawable, drawable, self.context) != 0
        };
        let pbuffer = self.pbuffer.load(Ordering::Acquire) as c_ulong;
        if pbuffer != 0 {
            return current(pbuffer);
        }
        if current(0) {
            return true;
        }
        let size = [glx::GLX_PBUFFER_WIDTH, 1, glx::GLX_PBUFFER_HEIGHT, 1, 0];
        let pbuffer = unsafe { (glx.glXCreatePbuffer)(self.display, self.config, size.as_ptr()) };
        if pbuffer == 0 {
            eprintln!("ferricone: GLX: no drawable for the video context");
            return false;
        }
        self.pbuffer.store(pbuffer, Ordering::Release);
        current(pbuffer)
    }

    pub fn release(&self) -> bool {
        let Ok(Libs { glx, .. }) = libs() else { return false };
        unsafe { (glx.glXMakeContextCurrent)(self.display, 0, 0, ptr::null_mut()) != 0 }
    }

    pub fn is_current(&self) -> bool {
        libs().is_ok_and(|l| unsafe { (l.glx.glXGetCurrentContext)() } == self.context)
    }

    pub fn proc_address(&self, name: &CStr) -> *const c_void {
        let Ok(Libs { glx, .. }) = libs() else { return ptr::null() };
        unsafe { (glx.glXGetProcAddressARB)(name.as_ptr().cast()) }
            .map_or(ptr::null(), |f| f as *const c_void)
    }
}

impl Drop for SharedContext {
    fn drop(&mut self) {
        let Ok(Libs { glx, .. }) = libs() else { return };
        unsafe {
            if self.is_current() {
                (glx.glXMakeContextCurrent)(self.display, 0, 0, ptr::null_mut());
            }
            (glx.glXDestroyContext)(self.display, self.context);
            let pbuffer = *self.pbuffer.get_mut() as c_ulong;
            if pbuffer != 0 {
                (glx.glXDestroyPbuffer)(self.display, pbuffer);
            }
        }
    }
}
