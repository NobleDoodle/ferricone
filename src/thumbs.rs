//! The library's thumbnails: a frame a third of the way into each video,
//! made by libVLC's thumbnailer one at a time and kept as PNGs in the cache
//! folder (`$XDG_CACHE_HOME/ferricone/thumbs`), named by the file's path,
//! size and modification time so an edited file gets a new one.

use crate::vlc::{Picture, Seek, Thumbnailer, Vlc};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Wide enough for the full library's 316 px cards on a 1.5× screen.
const WIDTH: u32 = 480;
const WHERE: Seek = Seek::Position(0.33);
/// A file that takes longer than this gets no thumbnail.
const TIMEOUT: Duration = Duration::from_secs(8);

/// What the thumbnailer's thread hands back: the file and its frame.
type Arrived = Arc<Mutex<VecDeque<(PathBuf, Option<Picture>)>>>;

pub struct Thumbs {
    vlc: Rc<Vlc>,
    thumbnailer: RefCell<Option<Thumbnailer>>,
    dir: Option<PathBuf>,
    shown: RefCell<HashMap<PathBuf, slint::Image>>,
    failed: RefCell<HashSet<PathBuf>>,
    waiting: RefCell<VecDeque<PathBuf>>,
    /// The file being made, shared with the thumbnailer's callback.
    busy: Arc<Mutex<Option<PathBuf>>>,
    arrived: Arrived,
}

impl Thumbs {
    pub fn new(vlc: Rc<Vlc>) -> Self {
        let dir = directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
            .map(|d| d.cache_dir().join("thumbs"));
        Self {
            vlc,
            thumbnailer: RefCell::default(),
            dir,
            shown: RefCell::default(),
            failed: RefCell::default(),
            waiting: RefCell::default(),
            busy: Arc::default(),
            arrived: Arc::default(),
        }
    }

    /// Starts the thumbnailer; `ready` runs on its thread after each one,
    /// and should get the UI thread to call `collect`.
    pub fn start(&self, ready: impl Fn() + Send + Sync + 'static) {
        let (busy, arrived, dir) = (self.busy.clone(), self.arrived.clone(), self.dir.clone());
        let made = self.vlc.thumbnailer(Some(TIMEOUT), move |picture| {
            let Some(path) = busy.lock().unwrap().take() else {
                return;
            };
            // Encoding here keeps it off the UI thread.
            if let (Some(dir), Some(p)) = (&dir, &picture)
                && let Some(file) = cache_file(dir, &path)
                && let Err(e) = save_png(&file, p)
            {
                eprintln!("ferricone: thumbnail cache: {e}");
            }
            arrived.lock().unwrap().push_back((path, picture));
            ready();
        });
        match made {
            Ok(t) => *self.thumbnailer.borrow_mut() = Some(t),
            Err(e) => eprintln!("ferricone: thumbnails: {e}"),
        }
    }

    /// `path`'s thumbnail if there is one yet; otherwise it is made (or
    /// read from the cache) and `collect` reports it.
    pub fn get(&self, path: &Path) -> Option<slint::Image> {
        if let Some(image) = self.shown.borrow().get(path) {
            return Some(image.clone());
        }
        if self.failed.borrow().contains(path) {
            return None;
        }
        if let Some(file) = self.dir.as_deref().and_then(|d| cache_file(d, path))
            && file.exists()
        {
            match slint::Image::load_from_path(&file) {
                Ok(image) => {
                    self.shown.borrow_mut().insert(path.into(), image.clone());
                    return Some(image);
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&file);
                }
            }
        }
        let mut waiting = self.waiting.borrow_mut();
        if !waiting.iter().any(|p| p == path) && self.busy.lock().unwrap().as_deref() != Some(path) {
            waiting.push_back(path.into());
        }
        drop(waiting);
        self.next();
        None
    }

    /// Takes in what the thumbnailer made; true when anything new shows.
    pub fn collect(&self) -> bool {
        let arrived: Vec<_> = self.arrived.lock().unwrap().drain(..).collect();
        let mut any = false;
        for (path, picture) in arrived {
            match picture {
                Some(p) => {
                    self.shown.borrow_mut().insert(path, image(p));
                    any = true;
                }
                None => {
                    self.failed.borrow_mut().insert(path);
                }
            }
        }
        self.next();
        any
    }

    /// Deletes the cached thumbnails; they are made again as needed.
    pub fn clear_cache(&self) {
        self.shown.borrow_mut().clear();
        self.failed.borrow_mut().clear();
        if let Some(dir) = &self.dir
            && let Err(e) = std::fs::remove_dir_all(dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("ferricone: clearing thumbnails: {e}");
        }
    }

    /// Forgets files no longer wanted (the library moved on).
    pub fn cancel_waiting(&self) {
        self.waiting.borrow_mut().clear();
    }

    fn next(&self) {
        if self.busy.lock().unwrap().is_some() {
            return;
        }
        let thumbnailer = self.thumbnailer.borrow();
        let Some(t) = thumbnailer.as_ref() else {
            return;
        };
        loop {
            let Some(path) = self.waiting.borrow_mut().pop_front() else {
                return;
            };
            *self.busy.lock().unwrap() = Some(path.clone());
            match t.request(&path, WHERE, WIDTH) {
                Ok(()) => return,
                Err(_) => {
                    *self.busy.lock().unwrap() = None;
                    self.failed.borrow_mut().insert(path);
                }
            }
        }
    }
}

fn image(p: Picture) -> slint::Image {
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &p.rgba, p.width, p.height,
    );
    slint::Image::from_rgba8(buffer)
}

/// The cache's name for `path` as it is now; None when it can't be read.
fn cache_file(dir: &Path, path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok().hash(&mut h);
    Some(dir.join(format!("{:016x}.png", h.finish())))
}

fn save_png(file: &Path, p: &Picture) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let temporary = file.with_extension("png.tmp");
    let out = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(out), p.width, p.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&p.rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, file).map_err(|e| e.to_string())
}
