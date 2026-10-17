//! What the seek bar shows besides the time: the preview frame under the
//! pointer and the waveform. Both are made off the UI thread by libVLC and
//! come back through the window's `preview-arrived` / `waveform-arrived`
//! callbacks. Results for an item that is no longer open are dropped.

use crate::{
    AppWindow,
    vlc::{AudioExtract, Picture, Thumbnailer, Vlc},
    waveform,
};
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Frames kept per item, most recent first (about 250 KB each).
const FRAMES_KEPT: usize = 48;
/// Preview frames are this far apart at most; long items use 1/300 of
/// their length, so hovering doesn't decode a frame per pixel.
const MIN_STEP: Duration = Duration::from_secs(1);
/// The preview frame's width in the design, before the window's scale.
const FRAME_WIDTH: f32 = 252.0;

type Slot<T> = Arc<Mutex<Option<T>>>;

pub struct SeekExtras {
    vlc: Rc<Vlc>,
    /// Counts items opened, to tell stale results apart.
    item: Cell<u64>,
    file: RefCell<Option<PathBuf>>,
    length: Cell<Duration>,

    thumbnailer: RefCell<Option<Thumbnailer>>,
    frame_arrived: Slot<Option<Picture>>,
    /// (item, step index) of the frame being decoded; one at a time.
    decoding: Cell<Option<(u64, u64)>>,
    /// The step index under the pointer.
    wanted: Cell<Option<u64>>,
    frames: RefCell<VecDeque<(u64, Image)>>,

    extract: RefCell<Option<AudioExtract>>,
    levels_arrived: Slot<(u64, Option<Vec<u8>>)>,
}

impl SeekExtras {
    pub fn new(vlc: Rc<Vlc>) -> Self {
        Self {
            vlc,
            item: Cell::default(),
            file: RefCell::default(),
            length: Cell::default(),
            thumbnailer: RefCell::default(),
            frame_arrived: Slot::default(),
            decoding: Cell::default(),
            wanted: Cell::default(),
            frames: RefCell::default(),
            extract: RefCell::default(),
            levels_arrived: Slot::default(),
        }
    }

    /// Starts the frame decoder; call once the window exists.
    pub fn start(&self, app: &AppWindow) {
        let (ui, slot) = (app.as_weak(), self.frame_arrived.clone());
        match self.vlc.thumbnailer(None, move |picture| {
            *slot.lock().unwrap() = Some(picture);
            let _ = ui.upgrade_in_event_loop(|app| app.invoke_preview_arrived());
        }) {
            Ok(t) => *self.thumbnailer.borrow_mut() = Some(t),
            Err(e) => eprintln!("ferricone: seek preview: {e}"),
        }
    }

    /// A new item opened: forget the last one's frames and waveform.
    pub fn reset(&self, app: &AppWindow, file: Option<PathBuf>) {
        self.item.set(self.item.get() + 1);
        self.frames.borrow_mut().clear();
        self.wanted.set(None);
        self.length.set(Duration::ZERO);
        // Dropping the extraction stops it; its callback removes the WAV.
        drop(self.extract.borrow_mut().take());
        app.set_preview_available(file.is_some() && self.thumbnailer.borrow().is_some());
        app.set_preview_ready(false);
        app.set_waveform_levels(ModelRc::default());
        *self.file.borrow_mut() = file;
    }

    pub fn set_length(&self, length: Duration) {
        self.length.set(length);
    }

    fn step(&self) -> Duration {
        MIN_STEP.max(self.length.get() / 300)
    }

    /// The pointer is at `position` (0–1) of the track.
    pub fn hover(&self, app: &AppWindow, position: f32) {
        let length = self.length.get();
        if length.is_zero() {
            return;
        }
        let time = length.mul_f32(position.clamp(0.0, 1.0));
        app.set_preview_time(crate::controls::hms(time).into());
        let index = (time.as_secs_f64() / self.step().as_secs_f64()).round() as u64;
        self.wanted.set(Some(index));
        if let Some(frame) = self.frame(index) {
            app.set_preview_frame(frame);
            app.set_preview_ready(true);
            return;
        }
        app.set_preview_ready(false);
        if self.decoding.get().is_none() {
            self.decode(app, index);
        }
    }

    fn frame(&self, index: u64) -> Option<Image> {
        let mut frames = self.frames.borrow_mut();
        let at = frames.iter().position(|(i, _)| *i == index)?;
        let entry = frames.remove(at)?;
        let image = entry.1.clone();
        frames.push_front(entry);
        Some(image)
    }

    fn decode(&self, app: &AppWindow, index: u64) {
        let (Some(file), Some(thumbnailer)) = (&*self.file.borrow(), &*self.thumbnailer.borrow())
        else {
            return;
        };
        let time = (self.step() * index as u32).min(self.length.get());
        let width = (FRAME_WIDTH * app.window().scale_factor()).round() as u32;
        match thumbnailer.request(file, crate::vlc::Seek::Time(time), width) {
            Ok(()) => self.decoding.set(Some((self.item.get(), index))),
            Err(e) => eprintln!("ferricone: seek preview: {e}"),
        }
    }

    /// A frame came back: keep it, show it if still wanted, then decode
    /// whatever the pointer moved to meanwhile.
    pub fn frame_arrived(&self, app: &AppWindow) {
        let picture = self.frame_arrived.lock().unwrap().take().flatten();
        let Some((item, index)) = self.decoding.take() else {
            return;
        };
        if item != self.item.get() {
            return;
        }
        if let Some(picture) = picture {
            let pixels = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                &picture.rgba,
                picture.width,
                picture.height,
            );
            let image = Image::from_rgba8(pixels);
            let mut frames = self.frames.borrow_mut();
            frames.push_front((index, image.clone()));
            frames.truncate(FRAMES_KEPT);
            if self.wanted.get() == Some(index) {
                app.set_preview_frame(image);
                app.set_preview_ready(true);
            }
        }
        if let Some(wanted) = self.wanted.get()
            && wanted != index
            && self.frame(wanted).is_none()
        {
            self.decode(app, wanted);
        }
    }

    /// The waveform is wanted: from the cache, or extracted once.
    pub fn want_waveform(&self, app: &AppWindow) {
        let Some(file) = self.file.borrow().clone() else {
            return;
        };
        if self.extract.borrow().is_some() {
            return;
        }
        let Some((cache, wav)) = waveform::cache_paths(&file) else {
            return;
        };
        if let Some(levels) = waveform::load(&cache) {
            show_levels(app, &levels);
            return;
        }
        if let Some(dir) = wav.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let (ui, slot, item) = (app.as_weak(), self.levels_arrived.clone(), self.item.get());
        let scratch = wav.clone();
        let extract = self.vlc.extract_audio(&file, &wav, move |finished| {
            // VLC's thread: reading and reducing the WAV goes on a thread of
            // its own; a stopped or failed extraction leaves nothing behind.
            let (ui, slot, wav, cache) = (ui.clone(), slot.clone(), scratch.clone(), cache.clone());
            std::thread::spawn(move || {
                let levels = if finished {
                    waveform::finish(&wav, &cache)
                } else {
                    let _ = std::fs::remove_file(&wav);
                    None
                };
                *slot.lock().unwrap() = Some((item, levels));
                let _ = ui.upgrade_in_event_loop(|app| app.invoke_waveform_arrived());
            });
        });
        match extract {
            Ok(extract) => *self.extract.borrow_mut() = Some(extract),
            Err(e) => eprintln!("ferricone: waveform: {e}"),
        }
    }

    pub fn levels_arrived(&self, app: &AppWindow) {
        let Some((item, levels)) = self.levels_arrived.lock().unwrap().take() else {
            return;
        };
        if item != self.item.get() {
            return;
        }
        drop(self.extract.borrow_mut().take());
        if let Some(levels) = levels {
            show_levels(app, &levels);
        }
    }
}

fn show_levels(app: &AppWindow, levels: &[u8]) {
    let levels: Vec<i32> = levels.iter().map(|&l| i32::from(l)).collect();
    app.set_waveform_levels(ModelRc::new(VecModel::from(levels)));
}
