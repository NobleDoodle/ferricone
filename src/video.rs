//! Hands frames from libVLC's OpenGL output (VLC's thread) to Slint (UI thread).
//!
//! VLC draws into three textures in its own GL context, which shares objects
//! with Slint's. `render` is being drawn, `ready` is the newest finished frame
//! and `shown` is what Slint samples. A finished frame swaps `render` and
//! `ready`; the UI swaps `ready` and `shown`. A fence per texture makes the UI
//! wait on the GPU, not the CPU, until VLC's drawing is done. Same scheme as
//! VLC's `doc/libvlc/sdl_opengl_player.cpp`.
//!
//! Framebuffers belong to VLC's context and are deleted there. Textures are
//! shared, so when VLC resizes or releases its output it only retires them;
//! the UI deletes them once a newer frame replaces them. The last frame stays
//! up meanwhile, so a reload (plan §9) freezes the picture instead of blanking it.

mod backdrop;

#[cfg_attr(target_os = "linux", path = "video/egl.rs")]
#[cfg_attr(not(target_os = "linux"), path = "video/unsupported.rs")]
mod context;

use backdrop::Backdrop;
use context::SharedContext;
use glow::HasContext;
use std::{
    ffi::{CStr, c_void},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Bridge {
    context: SharedContext,
    ui_gl: glow::Context,
    frames: Mutex<Frames>,
    redraw: Box<dyn Fn() + Send + Sync>,
    redraw_pending: AtomicBool,
    /// UI thread only.
    backdrop: Mutex<Backdrop>,
}

#[derive(Default)]
struct Frames {
    gl: Option<glow::Context>,
    textures: Vec<glow::NativeTexture>,
    fbos: Vec<glow::NativeFramebuffer>,
    fences: [Option<Fence>; 3],
    size: (u32, u32),
    render: usize,
    ready: usize,
    shown: usize,
    fresh: bool,
    retired: Vec<glow::NativeTexture>,
    /// The texture the UI shows, counted per frame for the backdrop.
    on_screen: Option<(glow::NativeTexture, (u32, u32))>,
    serial: u64,
}

struct Fence(glow::NativeFence);
// A GLsync belongs to the share group, not to a thread.
unsafe impl Send for Fence {}

impl Bridge {
    /// Call on the UI thread while Slint's GL context is current.
    /// `redraw` must ask the UI thread for a new frame; it may be called from any thread.
    pub fn new(
        get_proc_address: &dyn Fn(&CStr) -> *const c_void,
        redraw: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Ok(Self {
            context: SharedContext::from_current()?,
            ui_gl: unsafe {
                glow::Context::from_loader_function_cstr(|name| get_proc_address(name))
            },
            frames: Mutex::new(Frames {
                ready: 1,
                shown: 2,
                ..Default::default()
            }),
            redraw: Box::new(redraw),
            redraw_pending: AtomicBool::new(false),
            backdrop: Mutex::default(),
        })
    }

    // VLC's thread ----------------------------------------------------------

    pub fn proc_address(&self, name: &CStr) -> *const c_void {
        self.context.proc_address(name)
    }

    /// (Re)creates the three render targets. VLC may call this with or without
    /// its context current.
    pub fn resize(&self, width: u32, height: u32) -> bool {
        let entered = !self.context.is_current() && self.context.make_current();
        let mut f = self.frames.lock().unwrap();
        let context = &self.context;
        let gl = f.gl.get_or_insert_with(|| unsafe {
            glow::Context::from_loader_function_cstr(|name| context.proc_address(name))
        });
        let ok = match unsafe { create_targets(gl, width, height) } {
            Ok((textures, fbos)) => {
                f.retire_targets();
                (f.textures, f.fbos, f.size) = (textures, fbos, (width, height));
                (f.render, f.ready, f.shown, f.fresh) = (0, 1, 2, false);
                true
            }
            Err(e) => {
                eprintln!("ferricone: video targets: {e}");
                false
            }
        };
        drop(f);
        if entered {
            self.context.release();
        }
        ok
    }

    pub fn make_current(&self, enter: bool) -> bool {
        let mut f = self.frames.lock().unwrap();
        if enter {
            if !self.context.make_current() {
                return false;
            }
            if let (Some(gl), Some(&fbo)) = (&f.gl, f.fbos.get(f.render)) {
                unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo)) };
            }
            true
        } else {
            let f = &mut *f;
            if let Some(gl) = &f.gl
                && let Ok(fence) = unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0) }
            {
                unsafe { gl.flush() };
                if let Some(Fence(old)) = f.fences[f.render].replace(Fence(fence)) {
                    unsafe { gl.delete_sync(old) };
                }
            }
            self.context.release()
        }
    }

    /// VLC finished a frame (its context is no longer current).
    pub fn frame_done(&self) {
        {
            let mut f = self.frames.lock().unwrap();
            (f.render, f.ready) = (f.ready, f.render);
            f.fresh = true;
        }
        self.request_redraw();
    }

    /// VLC is done with its output (context not current).
    pub fn release_gl(&self) {
        let entered = !self.context.is_current() && self.context.make_current();
        {
            let mut f = self.frames.lock().unwrap();
            f.retire_targets();
            f.fresh = false;
        }
        if entered {
            self.context.release();
        }
    }

    fn request_redraw(&self) {
        if !self.redraw_pending.swap(true, Ordering::AcqRel) {
            (self.redraw)();
        }
    }

    // UI thread -------------------------------------------------------------

    /// VLC's newest finished frame, or `None` if there is none since the last
    /// call. The caller must show it in this frame. Call while Slint's context
    /// is current (rendering notifier).
    pub fn take_frame(&self) -> Option<slint::Image> {
        self.redraw_pending.store(false, Ordering::Release);
        let mut f = self.frames.lock().unwrap();
        if !f.fresh {
            return None;
        }
        (f.shown, f.ready, f.fresh) = (f.ready, f.shown, false);
        let shown = f.shown;
        if let Some(Fence(fence)) = f.fences[shown].take() {
            unsafe {
                self.ui_gl.wait_sync(fence, 0, glow::TIMEOUT_IGNORED);
                self.ui_gl.delete_sync(fence);
            }
        }
        let texture = *f.textures.get(shown)?;
        (f.on_screen, f.serial) = (Some((texture, f.size)), f.serial + 1);
        let texture = texture.0;
        // SAFETY: VLC only retires this texture; it stays alive until the UI deletes it below.
        let image = unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(texture, f.size.into())
                .origin(slint::BorrowedOpenGLTextureOrigin::BottomLeft)
                .build()
        };
        // The caller replaces the old image, so retired textures are no longer sampled.
        for texture in f.retired.drain(..) {
            unsafe { self.ui_gl.delete_texture(texture) };
        }
        Some(image)
    }

    /// The frame on screen blurred and letterboxed to the video box's
    /// `aspect`, for frosted overlays; `None` while unchanged since the last
    /// call. Call while Slint's context is current.
    pub fn backdrop(&self, aspect: f32) -> Option<slint::Image> {
        let (texture, size, serial) = {
            let f = self.frames.lock().unwrap();
            let (texture, size) = f.on_screen?;
            (texture, size, f.serial)
        };
        unsafe {
            self.backdrop
                .lock()
                .unwrap()
                .update(&self.ui_gl, texture, size, aspect, serial)
        }
    }

    /// Frees the UI-side GL objects; call while Slint's context is current.
    pub fn release_ui_gl(&self) {
        unsafe { self.backdrop.lock().unwrap().release(&self.ui_gl) }
    }
}

impl Frames {
    /// Deletes VLC's framebuffers and fences and hands the textures to the UI.
    /// Needs VLC's context current.
    fn retire_targets(&mut self) {
        let Some(gl) = &self.gl else { return };
        unsafe {
            for fbo in self.fbos.drain(..) {
                gl.delete_framebuffer(fbo);
            }
            for Fence(fence) in self.fences.iter_mut().filter_map(Option::take) {
                gl.delete_sync(fence);
            }
        }
        self.retired.append(&mut self.textures);
    }
}

unsafe fn create_targets(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<(Vec<glow::NativeTexture>, Vec<glow::NativeFramebuffer>), String> {
    let (mut textures, mut fbos) = (Vec::with_capacity(3), Vec::with_capacity(3));
    unsafe {
        for _ in 0..3 {
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            textures.push(texture);
            fbos.push(fbo);
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                for fbo in fbos {
                    gl.delete_framebuffer(fbo);
                }
                for texture in textures {
                    gl.delete_texture(texture);
                }
                return Err(format!("incomplete framebuffer at {width}×{height}"));
            }
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    Ok((textures, fbos))
}
