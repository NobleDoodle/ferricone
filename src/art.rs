//! The music player's backdrop: the cover, blurred. Shrinking it to a few
//! dozen pixels and box-blurring that is cheap, and drawn stretched over
//! the window it looks like Figma's 20 px layer blur.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// The shrunk square's side.
const SIDE: u32 = 48;
const RADIUS: usize = 3;

/// `cover` blurred, or None when its pixels can't be read.
pub fn blurred(cover: &Image) -> Option<Image> {
    let buffer = cover.to_rgba8()?;
    let (w, h) = (buffer.width(), buffer.height());
    if w == 0 || h == 0 {
        return None;
    }
    let pixels = buffer.as_slice();
    // The centered square, averaged down to SIDE × SIDE.
    let square = w.min(h);
    let (left, top) = ((w - square) / 2, (h - square) / 2);
    let mut small = vec![[0f32; 4]; (SIDE * SIDE) as usize];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (x0, x1) = (left + x * square / SIDE, left + ((x + 1) * square / SIDE).max(x * square / SIDE + 1));
            let (y0, y1) = (top + y * square / SIDE, top + ((y + 1) * square / SIDE).max(y * square / SIDE + 1));
            let mut sum = [0f32; 4];
            let mut n = 0f32;
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    let p = pixels[(sy * w + sx) as usize];
                    sum[0] += f32::from(p.r);
                    sum[1] += f32::from(p.g);
                    sum[2] += f32::from(p.b);
                    sum[3] += f32::from(p.a);
                    n += 1.0;
                }
            }
            small[(y * SIDE + x) as usize] = sum.map(|c| c / n.max(1.0));
        }
    }
    // Three box passes each way approximate a Gaussian.
    for _ in 0..3 {
        small = box_pass(&small, true);
        small = box_pass(&small, false);
    }
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(SIDE, SIDE);
    for (dst, src) in out.make_mut_slice().iter_mut().zip(&small) {
        *dst = Rgba8Pixel::new(src[0] as u8, src[1] as u8, src[2] as u8, 255);
    }
    Some(Image::from_rgba8(out))
}

fn box_pass(src: &[[f32; 4]], horizontal: bool) -> Vec<[f32; 4]> {
    let side = SIDE as usize;
    let mut out = vec![[0f32; 4]; src.len()];
    for a in 0..side {
        for b in 0..side {
            let mut sum = [0f32; 4];
            let mut n = 0f32;
            for d in b.saturating_sub(RADIUS)..(b + RADIUS + 1).min(side) {
                let i = if horizontal { a * side + d } else { d * side + a };
                for c in 0..4 {
                    sum[c] += src[i][c];
                }
                n += 1.0;
            }
            let i = if horizontal { a * side + b } else { b * side + a };
            out[i] = sum.map(|c| c / n);
        }
    }
    out
}
