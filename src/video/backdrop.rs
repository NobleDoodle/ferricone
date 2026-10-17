//! A blurred copy of the frame on screen, for the frosted panels and bars over
//! the video (the design's "bg blur", radius 50). Linear-filtered blits halve
//! the frame down to 1/64 of its size and step it back up to 1/8, smoothing at
//! every step (a shader-free "dual filter" blur); scaled up bilinearly from
//! there it shows no blocky creases. The frame is first placed on a black
//! canvas shaped like the video box, as it is displayed, so the edges of a
//! letterboxed picture blur too. GL 3.0 blits only.
//! Everything here runs on the UI thread with Slint's context current.

use glow::HasContext;
use std::num::NonZeroU32;

/// Halvings down; the backdrop is the level at 1/2^UP_TO after stepping back up.
const LEVELS: usize = 6;
const UP_TO: usize = 3;

type Level = (glow::NativeTexture, u32, u32);

#[derive(Default)]
pub struct Backdrop {
    read: Option<glow::NativeFramebuffer>,
    draw: Option<glow::NativeFramebuffer>,
    /// Texture and size per halving, from 1/2 down to 1/2^LEVELS.
    levels: Vec<Level>,
    /// The frame (serial) and canvas blurred last.
    key: Option<(u64, (u32, u32))>,
}

impl Backdrop {
    /// The blurred frame on a canvas of the video box's `aspect` (width /
    /// height), or `None` if that was blurred already.
    pub unsafe fn update(
        &mut self,
        gl: &glow::Context,
        frame: glow::NativeTexture,
        size: (u32, u32),
        aspect: f32,
        serial: u64,
    ) -> Option<slint::Image> {
        let canvas = canvas(size, aspect);
        if self.key == Some((serial, canvas)) {
            return None;
        }
        unsafe { self.render(gl, frame, size, canvas) }.ok()?;
        self.key = Some((serial, canvas));
        let &(texture, width, height) = self.levels.get(UP_TO - 1)?;
        // SAFETY: the texture lives until `release`, called before the context goes.
        Some(unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                texture.0,
                (width, height).into(),
            )
            .origin(slint::BorrowedOpenGLTextureOrigin::BottomLeft)
            .build()
        })
    }

    unsafe fn render(
        &mut self,
        gl: &glow::Context,
        frame: glow::NativeTexture,
        (width, height): (u32, u32),
        canvas: (u32, u32),
    ) -> Result<(), String> {
        unsafe {
            // Leave Slint's renderer the state it had.
            let prev_read = gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING);
            let prev_draw = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            let prev_texture = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);
            let scissor = gl.is_enabled(glow::SCISSOR_TEST);

            self.size_levels(gl, canvas.0, canvas.1)?;
            let read = match self.read {
                Some(fbo) => fbo,
                None => *self.read.insert(gl.create_framebuffer()?),
            };
            let draw = match self.draw {
                Some(fbo) => fbo,
                None => *self.draw.insert(gl.create_framebuffer()?),
            };
            // Copies `src` into the rectangle `(x, y, w, h)` of `dst`.
            let blit = |(src, sw, sh): Level,
                        dst: glow::NativeTexture,
                        (x, y, dw, dh): (u32, u32, u32, u32)| {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
                gl.framebuffer_texture_2d(
                    glow::READ_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(src),
                    0,
                );
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(draw));
                gl.framebuffer_texture_2d(
                    glow::DRAW_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(dst),
                    0,
                );
                gl.blit_framebuffer(
                    0,
                    0,
                    sw as i32,
                    sh as i32,
                    x as i32,
                    y as i32,
                    (x + dw) as i32,
                    (y + dh) as i32,
                    glow::COLOR_BUFFER_BIT,
                    glow::LINEAR,
                );
            };
            let whole = |(_, w, h): Level| (0, 0, w, h);

            gl.disable(glow::SCISSOR_TEST);
            // The frame centered on the black canvas, at half size.
            let first = self.levels[0];
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(draw));
            gl.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(first.0),
                0,
            );
            gl.clear_buffer_f32_slice(glow::COLOR, 0, &[0.0, 0.0, 0.0, 1.0]);
            let (x, y) = ((canvas.0 - width) / 4, (canvas.1 - height) / 4);
            blit(
                (frame, width, height),
                first.0,
                (x, y, width / 2, height / 2),
            );
            // Down to 1/2^LEVELS, then back up to 1/2^UP_TO.
            for i in 1..LEVELS {
                blit(self.levels[i - 1], self.levels[i].0, whole(self.levels[i]));
            }
            for i in (UP_TO..LEVELS).rev() {
                blit(
                    self.levels[i],
                    self.levels[i - 1].0,
                    whole(self.levels[i - 1]),
                );
            }

            let fbo = |id: i32| NonZeroU32::new(id as u32).map(glow::NativeFramebuffer);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, fbo(prev_read));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, fbo(prev_draw));
            gl.bind_texture(
                glow::TEXTURE_2D,
                NonZeroU32::new(prev_texture as u32).map(glow::NativeTexture),
            );
            if scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
        }
        Ok(())
    }

    /// (Re)creates the level textures when the frame size changes.
    unsafe fn size_levels(
        &mut self,
        gl: &glow::Context,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let sizes: Vec<(u32, u32)> = (1..=LEVELS)
            .map(|i| ((width >> i).max(1), (height >> i).max(1)))
            .collect();
        if self
            .levels
            .iter()
            .map(|&(_, w, h)| (w, h))
            .eq(sizes.iter().copied())
        {
            return Ok(());
        }
        unsafe {
            self.delete_levels(gl);
            for (w, h) in sizes {
                let texture = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    w as i32,
                    h as i32,
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
                self.levels.push((texture, w, h));
            }
        }
        Ok(())
    }

    unsafe fn delete_levels(&mut self, gl: &glow::Context) {
        for (texture, ..) in self.levels.drain(..) {
            unsafe { gl.delete_texture(texture) };
        }
        self.key = None;
    }

    /// Frees the GL objects; call while Slint's context is current.
    pub unsafe fn release(&mut self, gl: &glow::Context) {
        unsafe {
            self.delete_levels(gl);
            for fbo in [self.read.take(), self.draw.take()].into_iter().flatten() {
                gl.delete_framebuffer(fbo);
            }
        }
    }
}

/// The frame letterboxed to `aspect`, as the video box shows it.
fn canvas((width, height): (u32, u32), aspect: f32) -> (u32, u32) {
    let (w, h) = (width as f32, height as f32);
    if !(aspect.is_finite() && aspect > 0.0) {
        (width, height)
    } else if w / h < aspect {
        ((h * aspect).round() as u32, height)
    } else {
        (width, (w / aspect).round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::canvas;

    #[test]
    fn letterboxes_to_the_box_aspect() {
        assert_eq!(canvas((1920, 1080), 2.0), (2160, 1080));
        assert_eq!(canvas((1920, 1080), 1.0), (1920, 1920));
        assert_eq!(canvas((1920, 1080), 16.0 / 9.0), (1920, 1080));
        assert_eq!(canvas((1920, 1080), f32::NAN), (1920, 1080));
    }
}
