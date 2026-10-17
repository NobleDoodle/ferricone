//! The P1 snapshot test: the player rendered headlessly with Slint's
//! software renderer (no display, no compositor, the built-in Dark palette)
//! and compared with the design's `Player.png` export.
//!
//! The export is third-party design content, so it stays out of the repo:
//! the test reads it from `$FERRICONE_DESIGN_DIR`, else from the
//! `VLC Redesign (Community)` folder beside the repo, and only renders
//! (to `target/snapshots/`) when neither has it.
//!
//! Compared: the control bar, and the title bar's left group (⋯, dock,
//! divider, folder crumb). Not compared: the app glyph (the design's is
//! VideoLAN's cone), the title text and window buttons (Ferricone has only
//! fullscreen under a tiling compositor), and the video area (the design
//! shows the library dock over a still). Text is set in the system font, not
//! the design's Inter, so glyph edges differ; the limits allow for that.

use crate::AppWindow;
use slint::{
    ComponentHandle, PhysicalSize, Rgb8Pixel, SharedPixelBuffer,
    platform::{
        Platform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
/// The export is 4× the frame size.
const EXPORT_SCALE: u32 = 4;

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// x, y, width, height, and the limits: the mean channel difference, and
/// the share of pixels off by more than `FAR` in some channel.
struct Region {
    name: &'static str,
    /// In the design's frame.
    rect: (u32, u32, u32, u32),
    /// Where ours sits relative to the design's, in x.
    shift: i32,
    max_mean: f64,
    max_far: f64,
}

const FAR: u8 = 40;

/// Ferricone adds fullscreen at the control bar's far right (30 wide, 12
/// apart), which moves the groups right of the seek bar left by this.
const FULLSCREEN_SHIFT: i32 = -42;

const REGIONS: [Region; 3] = [
    Region {
        name: "control bar, transport and elapsed time",
        rect: (0, HEIGHT - 54, 250, 54),
        shift: 0,
        // Measured 5.03 / 5.49 %: the time's glyphs are a large share here.
        max_mean: 6.0,
        max_far: 0.065,
    },
    Region {
        name: "control bar, length to queue",
        rect: (1500, HEIGHT - 54, WIDTH - 1500, 54),
        shift: FULLSCREEN_SHIFT,
        // Measured 2.58 / 2.34 %: text.
        max_mean: 3.5,
        max_far: 0.03,
    },
    Region {
        name: "title bar, left group",
        rect: (44, 0, 200, 40),
        shift: 0,
        // Measured 5.03 / 4.94 %: text, and the design's dock button shows
        // the dock open.
        max_mean: 6.0,
        max_far: 0.06,
    },
];

/// The frame of Figma `Player` `247:445`: paused at 00:02:10 of 00:25:40,
/// volume at about half, folder "Videos".
fn render() -> SharedPixelBuffer<Rgb8Pixel> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
    let app = AppWindow::new().unwrap();
    app.set_media_title("Ferricone Player".into());
    app.set_folder_name("Videos".into());
    app.set_has_media(true);
    app.set_has_next(true);
    app.set_seekable(true);
    app.set_time_text("00:02:10".into());
    app.set_length_text("00:25:40".into());
    app.set_position(130.0 / 1540.0);
    app.set_volume(0.5);
    app.show().unwrap();
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    let mut frame = SharedPixelBuffer::<Rgb8Pixel>::new(WIDTH, HEIGHT);
    window.draw_if_needed(|renderer| {
        renderer.render(frame.make_mut_slice(), WIDTH as usize);
    });
    frame
}

fn design_dir() -> PathBuf {
    std::env::var_os("FERRICONE_DESIGN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../VLC Redesign (Community)")
        })
}

/// The export, box-averaged down to the frame size.
fn reference(path: &Path) -> Option<Vec<[u8; 3]>> {
    let decoder = png::Decoder::new(std::fs::File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).ok()?;
    let channels = info.color_type.samples();
    if info.width != WIDTH * EXPORT_SCALE || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let n = EXPORT_SCALE as usize;
    let mut out = vec![[0; 3]; (WIDTH * HEIGHT) as usize];
    for (i, px) in out.iter_mut().enumerate() {
        let (x, y) = (i % WIDTH as usize, i / WIDTH as usize);
        for c in 0..3 {
            let mut sum = 0u32;
            for dy in 0..n {
                let row = (y * n + dy) * info.line_size;
                for dx in 0..n {
                    sum += u32::from(bytes[row + (x * n + dx) * channels + c]);
                }
            }
            px[c] = (sum / (n * n) as u32) as u8;
        }
    }
    Some(out)
}

fn save(path: &Path, pixels: impl Iterator<Item = [u8; 3]>, width: u32, height: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgb);
    let data: Vec<u8> = pixels.flatten().collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&data)
        .unwrap();
}

#[test]
fn player_matches_the_design() {
    let frame = render();
    let ours: Vec<[u8; 3]> = frame.as_slice().iter().map(|p| [p.r, p.g, p.b]).collect();
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/snapshots");
    save(&out.join("player.png"), ours.iter().copied(), WIDTH, HEIGHT);

    let export = design_dir().join("Player.png");
    let Some(design) = reference(&export) else {
        eprintln!(
            "snapshot: no {} to compare with; rendered only",
            export.display()
        );
        return;
    };
    save(
        &out.join("player-design.png"),
        design.iter().copied(),
        WIDTH,
        HEIGHT,
    );

    let mut failures = Vec::new();
    for region in &REGIONS {
        let (x0, y0, w, h) = region.rect;
        let (mut total, mut far) = (0u64, 0u64);
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let i = (y * WIDTH + x) as usize;
                let j = (y * WIDTH) as usize + (x as i32 + region.shift) as usize;
                let diff = (0..3).map(|c| ours[j][c].abs_diff(design[i][c]));
                let worst = diff.clone().max().unwrap();
                total += diff.map(u64::from).sum::<u64>();
                far += u64::from(worst > FAR);
            }
        }
        let pixels = u64::from(w * h);
        let mean = total as f64 / (pixels * 3) as f64;
        let far = far as f64 / pixels as f64;
        eprintln!(
            "snapshot: {}: mean difference {mean:.2} (limit {}), {:.2} % far off (limit {:.0} %)",
            region.name,
            region.max_mean,
            far * 100.0,
            region.max_far * 100.0
        );
        if mean > region.max_mean || far > region.max_far {
            failures.push(region.name);
        }
    }
    assert!(
        failures.is_empty(),
        "{failures:?} differ from the design; compare target/snapshots/player.png with player-design.png"
    );
}
