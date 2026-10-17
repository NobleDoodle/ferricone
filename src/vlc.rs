//! The only module that touches libVLC. The rest of the app uses these safe types,
//! so libVLC 4 API changes before 4.0 final stay in this file.

use crate::{media::Source, presets::StereoMode, video::Bridge};
use libvlc_sys as sys;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{Arc, Mutex},
    time::Duration,
};

pub struct Vlc(NonNull<sys::libvlc_instance_t>);

impl Vlc {
    /// `args` are VLC command-line options, such as `--verbose=2`.
    pub fn new(args: &[String]) -> Result<Self, String> {
        let args = args
            .iter()
            .map(|a| CString::new(a.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "a VLC option contains a NUL byte")?;
        let argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
        NonNull::new(unsafe { sys::libvlc_new(argv.len() as i32, argv.as_ptr()) })
            .map(Self)
            .ok_or_else(|| "libVLC failed to start".into())
    }

    /// A player with no media yet, drawing video through `video`. `events` is
    /// called on VLC's threads.
    pub fn player(
        &self,
        video: Arc<Bridge>,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Player, String> {
        unsafe {
            let callbacks = Box::new(Callbacks {
                events: Box::new(events),
                clock: Arc::default(),
            });
            let opaque = (&*callbacks as *const Callbacks).cast_mut().cast();
            let mp = sys::libvlc_media_player_new(self.0.as_ptr(), &PLAYER_CBS, opaque);
            let player = Player {
                mp: NonNull::new(mp).ok_or("libVLC could not create a player")?,
                video,
                callbacks,
            };
            let mp = player.mp.as_ptr();
            // Output-synced time, at most every 100 ms; the UI interpolates between.
            sys::libvlc_media_player_watch_time(mp, 100_000, &TIME_CBS, opaque);

            sys::libvlc_media_player_set_video_title_display(mp, sys::libvlc_position_disable, 0);
            let selected = sys::libvlc_video_set_output_callbacks(
                mp,
                sys::libvlc_video_engine_opengl,
                Some(setup),
                Some(cleanup),
                None,
                Some(update_output),
                Some(swap),
                Some(make_current),
                Some(get_proc_address),
                None,
                None,
                Arc::as_ptr(&player.video).cast_mut().cast(),
            );
            if !selected {
                return Err("libVLC has no OpenGL output engine".into());
            }
            Ok(player)
        }
    }
}

impl Vlc {
    /// Decodes single frames (the seek bar's preview, the library's
    /// thumbnails) on libVLC's parser threads, giving up on one after
    /// `timeout` if set. `done` gets each result (`None` on failure), on
    /// those threads.
    pub fn thumbnailer(
        &self,
        timeout: Option<Duration>,
        done: impl Fn(Option<Picture>) + Send + Sync + 'static,
    ) -> Result<Thumbnailer, String> {
        let cfg = sys::libvlc_parser_cfg {
            version: 0,
            max_parser_threads: 0,
            max_thumbnailer_threads: 1,
            timeout: timeout.map_or(0, |t| t.as_micros() as i64),
        };
        let parser = unsafe { sys::libvlc_parser_new(self.0.as_ptr(), &cfg) };
        Ok(Thumbnailer {
            parser: NonNull::new(parser).ok_or("libVLC could not create a thumbnailer")?,
            done: Box::new(Box::new(done)),
        })
    }

    /// Transcodes the audio of `path` to 4 kHz mono 16-bit WAV at `wav`, as
    /// fast as it decodes (no clock), for the waveform. `done` gets whether it
    /// reached the end, on VLC's thread; drop the result to stop early.
    pub fn extract_audio(
        &self,
        path: &Path,
        wav: &Path,
        done: impl Fn(bool) + Send + Sync + 'static,
    ) -> Result<AudioExtract, String> {
        let media = new_media(&Source::File(path.into()))?;
        let chain = format!(
            ":sout=#transcode{{acodec=s16l,channels=1,samplerate={WAVE_RATE}}}:std{{access=file,mux=wav,dst=\"{}\"}}",
            wav.display()
        );
        let options = [chain.as_str(), ":no-video", ":no-spu", ":no-sout-video"];
        unsafe {
            for option in options {
                sys::libvlc_media_add_option(media, c_string(option)?.as_ptr());
            }
            let done: Box<ExtractDone> = Box::new(Box::new(done));
            let opaque = (&*done as *const ExtractDone).cast_mut().cast();
            let mp = sys::libvlc_media_player_new_from_media(
                self.0.as_ptr(),
                media,
                &EXTRACT_CBS,
                opaque,
            );
            sys::libvlc_media_release(media);
            let extract = AudioExtract {
                mp: NonNull::new(mp).ok_or("libVLC could not create a player")?,
                _done: done,
            };
            check(sys::libvlc_media_player_play(extract.mp.as_ptr()))?;
            Ok(extract)
        }
    }
}

impl Vlc {
    /// Reads files' durations on libVLC's parser threads, for the queue.
    /// `done` gets each file and its duration (`None` if unknown), on those
    /// threads.
    pub fn prober(
        &self,
        done: impl Fn(PathBuf, Option<Duration>) + Send + Sync + 'static,
    ) -> Result<Prober, String> {
        let cfg = sys::libvlc_parser_cfg {
            version: 0,
            max_parser_threads: 2,
            max_thumbnailer_threads: 0,
            timeout: -1,
        };
        let parser = unsafe { sys::libvlc_parser_new(self.0.as_ptr(), &cfg) };
        Ok(Prober {
            parser: NonNull::new(parser).ok_or("libVLC could not create a parser")?,
            done: Box::new(Box::new(done)),
        })
    }
}

type ProbeDone = Box<dyn Fn(PathBuf, Option<Duration>) + Send + Sync>;

pub struct Prober {
    parser: NonNull<sys::libvlc_parser_t>,
    /// Boxed twice so the requests' pointer to it stays put.
    done: Box<ProbeDone>,
}

/// One request in flight, owned by libVLC until its callback.
struct Probe {
    done: *const ProbeDone,
    path: PathBuf,
}

impl Prober {
    pub fn probe(&self, path: &Path) -> Result<(), String> {
        let media = new_media(&Source::File(path.into()))?;
        let probe = Box::into_raw(Box::new(Probe {
            done: &*self.done,
            path: path.into(),
        }));
        unsafe {
            let req = sys::libvlc_parser_request_t {
                version: 0,
                media,
                parse_flags: sys::libvlc_media_parse,
            };
            let task = sys::libvlc_parser_task_new_parse(
                self.parser.as_ptr(),
                &req,
                &PROBE_CBS,
                probe.cast(),
            );
            sys::libvlc_media_release(media);
            if task.is_null() || sys::libvlc_parser_submit(self.parser.as_ptr(), task) != 0 {
                if !task.is_null() {
                    sys::libvlc_parser_task_release(task);
                }
                drop(Box::from_raw(probe));
                return Err("libVLC refused to read the file".into());
            }
        }
        Ok(())
    }
}

impl Drop for Prober {
    fn drop(&mut self) {
        // Reports what is pending as cancelled, freeing each request, and
        // joins the threads before `done` goes.
        unsafe { sys::libvlc_parser_destroy(self.parser.as_ptr()) };
    }
}

static PROBE_CBS: sys::libvlc_parser_cbs = sys::libvlc_parser_cbs {
    version: 0,
    on_parsed: Some(on_probed),
    on_attachments_added: None,
};

unsafe extern "C" fn on_probed(
    opaque: *mut c_void,
    task: *mut sys::libvlc_parser_task,
    status: sys::libvlc_parser_status_t,
) {
    let probe = unsafe { Box::from_raw(opaque.cast::<Probe>()) };
    let duration = unsafe {
        let media = sys::libvlc_parser_task_get_media(task);
        let us = if media.is_null() || status != sys::libvlc_parser_status_done {
            -1
        } else {
            sys::libvlc_media_get_duration(media)
        };
        sys::libvlc_parser_task_release(task);
        (us > 0).then(|| micros(us))
    };
    (unsafe { &*probe.done })(probe.path, duration);
}

/// libVLC's version ("4.0.0-dev Otto Chriek"), as it reports it.
pub fn version() -> String {
    unsafe { text(sys::libvlc_get_version().cast_mut()) }.unwrap_or_default()
}

/// A device to play on (Chromecast and similar), found by a discoverer.
/// Holds a reference to libVLC's item.
pub struct Renderer {
    item: NonNull<sys::libvlc_renderer_item_t>,
    pub name: String,
}

// libVLC's renderer items are reference counted and thread-safe.
unsafe impl Send for Renderer {}

impl Renderer {
    /// Takes a reference to `item`.
    unsafe fn retain(item: *mut sys::libvlc_renderer_item_t) -> Option<Self> {
        let item = NonNull::new(unsafe { sys::libvlc_renderer_item_retain(item) })?;
        let name = unsafe { text(sys::libvlc_renderer_item_name(item.as_ptr())) }.unwrap_or_default();
        Some(Self { item, name })
    }

    fn is(&self, item: *mut sys::libvlc_renderer_item_t) -> bool {
        self.item.as_ptr() == item
    }
}

impl Clone for Renderer {
    fn clone(&self) -> Self {
        unsafe { Self::retain(self.item.as_ptr()) }.expect("retain returns its item")
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { sys::libvlc_renderer_item_release(self.item.as_ptr()) };
    }
}

type RenderersChanged = Box<dyn Fn() + Send + Sync>;

/// The devices found by every renderer discoverer libVLC has (mDNS for
/// Chromecast, UPnP, …), kept up to date by libVLC's threads.
pub struct Discovery {
    discoverers: Vec<NonNull<sys::libvlc_renderer_discoverer_t>>,
    shared: Box<DiscoveryShared>,
}

struct DiscoveryShared {
    found: Mutex<Vec<Renderer>>,
    changed: RenderersChanged,
}

static RENDERER_CBS: sys::libvlc_renderer_discoverer_cbs = sys::libvlc_renderer_discoverer_cbs {
    version: 0,
    on_item_added: Some(on_renderer_added),
    on_item_removed: Some(on_renderer_removed),
};

unsafe extern "C" fn on_renderer_added(opaque: *mut c_void, item: *mut sys::libvlc_renderer_item_t) {
    let shared = unsafe { &*opaque.cast::<DiscoveryShared>() };
    if let Some(r) = unsafe { Renderer::retain(item) } {
        shared.found.lock().unwrap().push(r);
        (shared.changed)();
    }
}

unsafe extern "C" fn on_renderer_removed(opaque: *mut c_void, item: *mut sys::libvlc_renderer_item_t) {
    let shared = unsafe { &*opaque.cast::<DiscoveryShared>() };
    shared.found.lock().unwrap().retain(|r| !r.is(item));
    (shared.changed)();
}

impl Vlc {
    /// Starts looking for devices; `changed` runs on libVLC's threads when
    /// one comes or goes.
    pub fn discover_renderers(&self, changed: impl Fn() + Send + Sync + 'static) -> Discovery {
        let shared = Box::new(DiscoveryShared {
            found: Mutex::default(),
            changed: Box::new(changed),
        });
        let mut discoverers = Vec::new();
        unsafe {
            let mut services = std::ptr::null_mut();
            let count = sys::libvlc_renderer_discoverer_list_get(self.0.as_ptr(), &mut services);
            for i in 0..count {
                let service = *services.add(i);
                let rd = sys::libvlc_renderer_discoverer_new(
                    self.0.as_ptr(),
                    (*service).psz_name,
                    &RENDERER_CBS,
                    (&*shared as *const DiscoveryShared).cast_mut().cast(),
                );
                if let Some(rd) = NonNull::new(rd) {
                    if sys::libvlc_renderer_discoverer_start(rd.as_ptr()) == 0 {
                        discoverers.push(rd);
                    } else {
                        sys::libvlc_renderer_discoverer_destroy(rd.as_ptr());
                    }
                }
            }
            if count > 0 {
                sys::libvlc_renderer_discoverer_list_release(services, count);
            }
        }
        Discovery { discoverers, shared }
    }
}

impl Discovery {
    pub fn found(&self) -> Vec<Renderer> {
        self.shared.found.lock().unwrap().clone()
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        // Stops their threads before `shared` goes.
        for rd in &self.discoverers {
            unsafe { sys::libvlc_renderer_discoverer_destroy(rd.as_ptr()) };
        }
    }
}

/// A file's tags, as the library's Music page sorts by them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
}

type TagDone = Box<dyn Fn(PathBuf, Option<Tags>) + Send + Sync>;

/// Reads files' tags on libVLC's parser threads.
pub struct Tagger {
    parser: NonNull<sys::libvlc_parser_t>,
    done: Box<TagDone>,
}

struct Tagging {
    done: *const TagDone,
    path: PathBuf,
}

impl Vlc {
    /// `done` gets each file and its tags (`None` on failure), on the
    /// parser's threads.
    pub fn tagger(
        &self,
        done: impl Fn(PathBuf, Option<Tags>) + Send + Sync + 'static,
    ) -> Result<Tagger, String> {
        let cfg = sys::libvlc_parser_cfg {
            version: 0,
            max_parser_threads: 2,
            max_thumbnailer_threads: 0,
            timeout: Duration::from_secs(10).as_micros() as i64,
        };
        let parser = unsafe { sys::libvlc_parser_new(self.0.as_ptr(), &cfg) };
        Ok(Tagger {
            parser: NonNull::new(parser).ok_or("libVLC could not create a parser")?,
            done: Box::new(Box::new(done)),
        })
    }
}

impl Tagger {
    pub fn read(&self, path: &Path) -> Result<(), String> {
        let media = new_media(&Source::File(path.into()))?;
        let request = Box::into_raw(Box::new(Tagging {
            done: &*self.done,
            path: path.into(),
        }));
        unsafe {
            let req = sys::libvlc_parser_request_t {
                version: 0,
                media,
                parse_flags: sys::libvlc_media_parse,
            };
            let task = sys::libvlc_parser_task_new_parse(
                self.parser.as_ptr(),
                &req,
                &TAG_CBS,
                request.cast(),
            );
            sys::libvlc_media_release(media);
            if task.is_null() || sys::libvlc_parser_submit(self.parser.as_ptr(), task) != 0 {
                if !task.is_null() {
                    sys::libvlc_parser_task_release(task);
                }
                drop(Box::from_raw(request));
                return Err("libVLC refused to read the file".into());
            }
        }
        Ok(())
    }
}

impl Drop for Tagger {
    fn drop(&mut self) {
        unsafe { sys::libvlc_parser_destroy(self.parser.as_ptr()) };
    }
}

static TAG_CBS: sys::libvlc_parser_cbs = sys::libvlc_parser_cbs {
    version: 0,
    on_parsed: Some(on_tagged),
    on_attachments_added: None,
};

unsafe extern "C" fn on_tagged(
    opaque: *mut c_void,
    task: *mut sys::libvlc_parser_task,
    status: sys::libvlc_parser_status_t,
) {
    let request = unsafe { Box::from_raw(opaque.cast::<Tagging>()) };
    let tags = unsafe {
        let media = sys::libvlc_parser_task_get_media(task);
        let tags = (!media.is_null() && status == sys::libvlc_parser_status_done).then(|| {
            let get = |kind| owned_text(sys::libvlc_media_get_meta(media, kind));
            let number = |kind| get(kind).and_then(|t: String| t.split('/').next()?.trim().parse().ok());
            Tags {
                title: get(sys::libvlc_meta_Title),
                artist: get(sys::libvlc_meta_AlbumArtist).or_else(|| get(sys::libvlc_meta_Artist)),
                album: get(sys::libvlc_meta_Album),
                genre: get(sys::libvlc_meta_Genre),
                track: number(sys::libvlc_meta_TrackNumber),
                disc: number(sys::libvlc_meta_DiscNumber),
            }
        });
        sys::libvlc_parser_task_release(task);
        tags
    };
    (unsafe { &*request.done })(request.path, tags);
}

/// An entry of a remote playlist: its location and VLC's title for it.
pub struct Entry {
    pub mrl: String,
    pub title: Option<String>,
}

type ExpandDone = Box<dyn Fn(String, Option<Vec<Entry>>) + Send + Sync>;

/// Reads remote playlists (an M3U link) into their entries, on libVLC's
/// parser threads; for the library's browsable streams.
pub struct Expander {
    parser: NonNull<sys::libvlc_parser_t>,
    done: Box<ExpandDone>,
}

struct Expansion {
    done: *const ExpandDone,
    url: String,
}

impl Vlc {
    /// `done` gets each URL and its entries (`None` on failure), on the
    /// parser's threads; a list taking longer than `timeout` fails.
    pub fn expander(
        &self,
        timeout: Duration,
        done: impl Fn(String, Option<Vec<Entry>>) + Send + Sync + 'static,
    ) -> Result<Expander, String> {
        let cfg = sys::libvlc_parser_cfg {
            version: 0,
            max_parser_threads: 1,
            max_thumbnailer_threads: 0,
            timeout: timeout.as_micros() as i64,
        };
        let parser = unsafe { sys::libvlc_parser_new(self.0.as_ptr(), &cfg) };
        Ok(Expander {
            parser: NonNull::new(parser).ok_or("libVLC could not create a parser")?,
            done: Box::new(Box::new(done)),
        })
    }
}

impl Expander {
    pub fn expand(&self, url: &str) -> Result<(), String> {
        let media = new_media(&Source::Location(url.into()))?;
        let request = Box::into_raw(Box::new(Expansion {
            done: &*self.done,
            url: url.into(),
        }));
        unsafe {
            let req = sys::libvlc_parser_request_t {
                version: 0,
                media,
                parse_flags: sys::libvlc_media_parse | sys::libvlc_media_fetch_network,
            };
            let task = sys::libvlc_parser_task_new_parse(
                self.parser.as_ptr(),
                &req,
                &EXPAND_CBS,
                request.cast(),
            );
            sys::libvlc_media_release(media);
            if task.is_null() || sys::libvlc_parser_submit(self.parser.as_ptr(), task) != 0 {
                if !task.is_null() {
                    sys::libvlc_parser_task_release(task);
                }
                drop(Box::from_raw(request));
                return Err("libVLC refused to read the playlist".into());
            }
        }
        Ok(())
    }
}

impl Drop for Expander {
    fn drop(&mut self) {
        // As for the prober: pending requests are reported and freed first.
        unsafe { sys::libvlc_parser_destroy(self.parser.as_ptr()) };
    }
}

static EXPAND_CBS: sys::libvlc_parser_cbs = sys::libvlc_parser_cbs {
    version: 0,
    on_parsed: Some(on_expanded),
    on_attachments_added: None,
};

unsafe extern "C" fn on_expanded(
    opaque: *mut c_void,
    task: *mut sys::libvlc_parser_task,
    status: sys::libvlc_parser_status_t,
) {
    let request = unsafe { Box::from_raw(opaque.cast::<Expansion>()) };
    let entries = unsafe {
        let media = sys::libvlc_parser_task_get_media(task);
        let entries = if media.is_null() || status != sys::libvlc_parser_status_done {
            None
        } else {
            subitems(media)
        };
        sys::libvlc_parser_task_release(task);
        entries
    };
    (unsafe { &*request.done })(request.url, entries);
}

/// A parsed media's subitems; None when it has none (not a playlist).
unsafe fn subitems(media: *mut sys::libvlc_media_t) -> Option<Vec<Entry>> {
    unsafe {
        let list = sys::libvlc_media_subitems(media);
        if list.is_null() {
            return None;
        }
        sys::libvlc_media_list_lock(list);
        let count = sys::libvlc_media_list_count(list).max(0);
        let mut entries = Vec::with_capacity(count as usize);
        for i in 0..count {
            let item = sys::libvlc_media_list_item_at_index(list, i);
            if item.is_null() {
                continue;
            }
            if let Some(mrl) = owned_text(sys::libvlc_media_get_mrl(item)) {
                let title = owned_text(sys::libvlc_media_get_meta(item, sys::libvlc_meta_Title));
                entries.push(Entry { mrl, title });
            }
            sys::libvlc_media_release(item);
        }
        sys::libvlc_media_list_unlock(list);
        sys::libvlc_media_list_release(list);
        (!entries.is_empty()).then_some(entries)
    }
}

/// One of VLC's equalizer presets.
pub struct EqualizerPreset {
    pub name: String,
    pub preamp: f32,
    pub bands: Vec<f32>,
}

/// VLC's equalizer presets ("Flat", "Classical", …).
pub fn equalizer_presets() -> Vec<EqualizerPreset> {
    unsafe {
        let bands = sys::libvlc_audio_equalizer_get_band_count();
        (0..sys::libvlc_audio_equalizer_get_preset_count())
            .filter_map(|i| {
                let name = text(sys::libvlc_audio_equalizer_get_preset_name(i))?;
                let eq = sys::libvlc_audio_equalizer_new_from_preset(i);
                if eq.is_null() {
                    return None;
                }
                let preset = EqualizerPreset {
                    name,
                    preamp: sys::libvlc_audio_equalizer_get_preamp(eq),
                    bands: (0..bands)
                        .map(|b| sys::libvlc_audio_equalizer_get_amp_at_index(eq, b))
                        .collect(),
                };
                sys::libvlc_audio_equalizer_release(eq);
                Some(preset)
            })
            .collect()
    }
}

/// The equalizer's band frequencies in Hz.
pub fn equalizer_bands() -> Vec<f32> {
    unsafe {
        (0..sys::libvlc_audio_equalizer_get_band_count())
            .map(|b| sys::libvlc_audio_equalizer_get_band_frequency(b))
            .collect()
    }
}

/// The sample rate of extracted audio: enough for peaks, small to read.
pub const WAVE_RATE: u32 = 4000;

/// A media for `source`; the caller releases it.
fn new_media(source: &Source) -> Result<*mut sys::libvlc_media_t, String> {
    let media = unsafe {
        match source {
            Source::File(path) => {
                let path = CString::new(path.as_os_str().as_encoded_bytes())
                    .map_err(|_| "the path contains a NUL byte")?;
                sys::libvlc_media_new_path(path.as_ptr())
            }
            Source::Location(url) => sys::libvlc_media_new_location(c_string(url)?.as_ptr()),
        }
    };
    if media.is_null() {
        return Err(format!("cannot open {}", source.title()));
    }
    Ok(media)
}

/// A decoded frame, tightly packed RGBA.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

type ThumbnailDone = Box<dyn Fn(Option<Picture>) + Send + Sync>;

/// Where in the file a thumbnail is taken.
#[derive(Clone, Copy, Debug)]
pub enum Seek {
    Time(Duration),
    /// 0–1 of the length.
    Position(f64),
}

pub struct Thumbnailer {
    parser: NonNull<sys::libvlc_parser_t>,
    /// Boxed twice so the callbacks' pointer stays put.
    done: Box<ThumbnailDone>,
}

impl Thumbnailer {
    /// Asks for the frame near `at` of the file at `path`, `width` pixels
    /// wide; a keyframe seek, so it is quick rather than exact.
    pub fn request(&self, path: &Path, at: Seek, width: u32) -> Result<(), String> {
        let media = new_media(&Source::File(path.into()))?;
        unsafe {
            let mut req: sys::libvlc_thumbnailer_request_t = std::mem::zeroed();
            req.version = 0;
            req.media = media;
            req.width = width;
            req.type_ = sys::libvlc_picture_Rgba;
            match at {
                Seek::Time(time) => {
                    req.seek.type_ = sys::libvlc_thumbnailer_seek_time;
                    req.seek.value.time = time.as_micros() as i64;
                }
                Seek::Position(pos) => {
                    req.seek.type_ = sys::libvlc_thumbnailer_seek_pos;
                    req.seek.value.pos = pos;
                }
            }
            req.seek.speed = sys::libvlc_media_thumbnail_seek_fast;
            let opaque = (&*self.done as *const ThumbnailDone).cast_mut().cast();
            let task = sys::libvlc_parser_task_new_thumbnail(
                self.parser.as_ptr(),
                &req,
                &THUMBNAIL_CBS,
                opaque,
            );
            sys::libvlc_media_release(media);
            if task.is_null() {
                return Err("libVLC refused the thumbnail".into());
            }
            if sys::libvlc_parser_submit(self.parser.as_ptr(), task) != 0 {
                sys::libvlc_parser_task_release(task);
                return Err("libVLC refused the thumbnail".into());
            }
        }
        Ok(())
    }
}

impl Drop for Thumbnailer {
    fn drop(&mut self) {
        // Cancels what is pending, reports it, and joins the threads, so no
        // callback can reach `done` afterwards.
        unsafe { sys::libvlc_parser_destroy(self.parser.as_ptr()) };
    }
}

static THUMBNAIL_CBS: sys::libvlc_thumbnailer_cbs = sys::libvlc_thumbnailer_cbs {
    version: 0,
    on_ended: Some(on_thumbnail),
};

unsafe extern "C" fn on_thumbnail(
    opaque: *mut c_void,
    task: *mut sys::libvlc_parser_task,
    picture: *mut sys::libvlc_picture_t,
) {
    let done = unsafe { &*opaque.cast::<ThumbnailDone>() };
    done(unsafe { picture.as_ref() }.and_then(|p| unsafe { packed_rgba(p) }));
    unsafe { sys::libvlc_parser_task_release(task) };
}

/// Copies an RGBA picture without its row padding.
unsafe fn packed_rgba(picture: &sys::libvlc_picture_t) -> Option<Picture> {
    unsafe {
        if sys::libvlc_picture_type(picture) != sys::libvlc_picture_Rgba {
            return None;
        }
        let (width, height) = (
            sys::libvlc_picture_get_width(picture),
            sys::libvlc_picture_get_height(picture),
        );
        let stride = sys::libvlc_picture_get_stride(picture) as usize;
        let mut size = 0;
        let buffer = sys::libvlc_picture_get_buffer(picture, &mut size);
        let row = width as usize * 4;
        if buffer.is_null() || stride < row || size < stride * height as usize {
            return None;
        }
        let buffer = std::slice::from_raw_parts(buffer, size);
        let rgba = buffer
            .chunks(stride)
            .take(height as usize)
            .flat_map(|line| &line[..row])
            .copied()
            .collect();
        Some(Picture {
            width,
            height,
            rgba,
        })
    }
}

type ExtractDone = Box<dyn Fn(bool) + Send + Sync>;

/// A hidden player transcoding audio to a file (`Vlc::extract_audio`).
pub struct AudioExtract {
    mp: NonNull<sys::libvlc_media_player_t>,
    _done: Box<ExtractDone>,
}

impl Drop for AudioExtract {
    fn drop(&mut self) {
        // Stops and joins VLC's threads before `_done` goes.
        unsafe { sys::libvlc_media_player_release(self.mp.as_ptr()) };
    }
}

static EXTRACT_CBS: sys::libvlc_media_player_cbs = {
    let mut cbs: sys::libvlc_media_player_cbs = unsafe { std::mem::zeroed() };
    cbs.version = 0;
    cbs.on_media_stopping = Some(on_extract_stopping);
    cbs
};

unsafe extern "C" fn on_extract_stopping(
    opaque: *mut c_void,
    _media: *mut sys::libvlc_media_t,
    reason: sys::libvlc_stopping_reason_t,
) {
    let done = unsafe { &*opaque.cast::<ExtractDone>() };
    done(reason == sys::libvlc_stopping_reason_eos);
}

impl Drop for Vlc {
    fn drop(&mut self) {
        // Players keep their own reference to the instance.
        unsafe { sys::libvlc_release(self.0.as_ptr()) }
    }
}

pub struct Player {
    mp: NonNull<sys::libvlc_media_player_t>,
    video: Arc<Bridge>,
    callbacks: Box<Callbacks>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Opening,
    Playing,
    Paused,
    Stopping,
    Stopped,
    Error,
}

/// What the player reports, on VLC's threads.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    State(State),
    /// Read the new length from `Player::clock`.
    LengthChanged,
    Seekable(bool),
    /// Tracks were added, removed or selected: read them from `Player::tracks`.
    TracksChanged,
    /// The media played to its end or failed; a stop or a new media doesn't count.
    Ended,
    Recording(bool),
    /// A seek started, to this time.
    Seeked(Duration),
    /// VLC read new tags: read them from `Player::meta`.
    MetaChanged,
    /// The chapters, or the one playing, changed: read `Player::chapters`.
    ChaptersChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

impl TrackKind {
    fn raw(self) -> sys::libvlc_track_type_t {
        match self {
            TrackKind::Video => sys::libvlc_track_video,
            TrackKind::Audio => sys::libvlc_track_audio,
            TrackKind::Subtitle => sys::libvlc_track_text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

/// Playback time from VLC's output clock, interpolated to now.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub time: Duration,
    pub length: Duration,
    pub position: f64,
}

impl Player {
    pub fn video(&self) -> &Bridge {
        &self.video
    }

    /// Replaces the media and starts playing it, with `options` (":name=value")
    /// for it alone.
    pub fn open(&self, source: &Source, options: &[String]) -> Result<(), String> {
        let media = new_media(source)?;
        for option in options {
            let option = c_string(option)?;
            unsafe { sys::libvlc_media_add_option(media, option.as_ptr()) };
        }
        *self.callbacks.clock.lock().unwrap() = ClockPoint::default();
        unsafe {
            sys::libvlc_media_player_set_media(self.mp.as_ptr(), media);
            sys::libvlc_media_release(media);
        }
        check(unsafe { sys::libvlc_media_player_play(self.mp.as_ptr()) })
    }

    /// Stops playback and forgets the time; `restart` plays the media again
    /// from the start.
    pub fn stop(&self) {
        unsafe { sys::libvlc_media_player_stop_async(self.mp.as_ptr()) };
        *self.callbacks.clock.lock().unwrap() = ClockPoint::default();
    }

    /// Records the playing stream into VLC's record folder (the user's videos
    /// folder by default); VLC reports `Event::Recording` when it starts or ends.
    pub fn set_recording(&self, on: bool, dir: Option<&Path>) {
        let dir = dir.and_then(|d| CString::new(d.as_os_str().as_encoded_bytes()).ok());
        let dir = dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr());
        unsafe { sys::libvlc_media_player_record(self.mp.as_ptr(), on, dir) };
    }

    pub fn state(&self) -> State {
        state(unsafe { sys::libvlc_media_player_get_state(self.mp.as_ptr()) })
    }

    pub fn set_paused(&self, paused: bool) {
        unsafe { sys::libvlc_media_player_set_pause(self.mp.as_ptr(), c_int::from(paused)) }
    }

    /// Plays again from the start after the media ended or stopped.
    pub fn restart(&self) {
        unsafe { sys::libvlc_media_player_play(self.mp.as_ptr()) };
    }

    /// `position` is 0–1. `fast` trades precision for speed, for dragging.
    pub fn seek(&self, position: f64, fast: bool) {
        unsafe {
            sys::libvlc_media_player_set_position(self.mp.as_ptr(), position.clamp(0.0, 1.0), fast)
        };
    }

    /// 1 is normal speed.
    pub fn set_rate(&self, rate: f32) {
        unsafe { sys::libvlc_media_player_set_rate(self.mp.as_ptr(), rate) };
    }

    /// Seconds; positive plays the audio later.
    pub fn set_audio_delay(&self, seconds: f64) {
        unsafe { sys::libvlc_audio_set_delay(self.mp.as_ptr(), (seconds * 1e6) as i64) };
    }

    /// Seconds; positive shows the subtitles later.
    pub fn set_subtitle_delay(&self, seconds: f64) {
        unsafe { sys::libvlc_video_set_spu_delay(self.mp.as_ptr(), (seconds * 1e6) as i64) };
    }

    pub fn set_stereo(&self, stereo: StereoMode) {
        let mode = match stereo {
            StereoMode::Original => sys::libvlc_AudioStereoMode_Unset,
            StereoMode::Stereo => sys::libvlc_AudioStereoMode_Stereo,
            StereoMode::Reverse => sys::libvlc_AudioStereoMode_RStereo,
            StereoMode::Left => sys::libvlc_AudioStereoMode_Left,
            StereoMode::Right => sys::libvlc_AudioStereoMode_Right,
            StereoMode::Dolby => sys::libvlc_AudioStereoMode_Dolbys,
            StereoMode::Mono => sys::libvlc_AudioStereoMode_Mono,
        };
        unsafe { sys::libvlc_audio_set_stereomode(self.mp.as_ptr(), mode) };
    }

    /// Sets the equalizer (`preamp` and `bands` in dB), or turns it off.
    pub fn set_equalizer(&self, settings: Option<(f32, &[f32])>) {
        unsafe {
            let Some((preamp, bands)) = settings else {
                sys::libvlc_media_player_set_equalizer(self.mp.as_ptr(), std::ptr::null_mut());
                return;
            };
            let eq = sys::libvlc_audio_equalizer_new();
            if eq.is_null() {
                return;
            }
            sys::libvlc_audio_equalizer_set_preamp(eq, preamp);
            for (i, &amp) in bands.iter().enumerate() {
                sys::libvlc_audio_equalizer_set_amp_at_index(eq, amp, i as std::ffi::c_uint);
            }
            sys::libvlc_media_player_set_equalizer(self.mp.as_ptr(), eq);
            sys::libvlc_audio_equalizer_release(eq);
        }
    }

    /// Subtitle text size, 100 being normal.
    /// Plays on `renderer` (None: this device) from the next item opened.
    pub fn set_renderer(&self, renderer: Option<&Renderer>) {
        let item = renderer.map_or(std::ptr::null_mut(), |r| r.item.as_ptr());
        unsafe { sys::libvlc_media_player_set_renderer(self.mp.as_ptr(), item) };
    }

    /// Adds a subtitle file to the item playing and shows it.
    pub fn add_subtitle(&self, path: &Path) -> Result<(), String> {
        let uri = c_string(&crate::media::file_uri(path))?;
        let added = unsafe {
            sys::libvlc_media_player_add_slave(
                self.mp.as_ptr(),
                sys::libvlc_media_slave_type_subtitle,
                uri.as_ptr(),
                true,
            )
        };
        if added == 0 {
            Ok(())
        } else {
            Err(format!("libVLC refused {}", path.display()))
        }
    }

    pub fn set_subtitle_scale(&self, percent: u32) {
        unsafe { sys::libvlc_video_set_spu_text_scale(self.mp.as_ptr(), percent as f32 / 100.0) };
    }

    /// The first video track's size in pixels.
    pub fn video_size(&self) -> Option<(u32, u32)> {
        let (mut w, mut h) = (0, 0);
        let ok = unsafe { sys::libvlc_video_get_size(self.mp.as_ptr(), 0, &mut w, &mut h) };
        (ok == 0 && w > 0).then_some((w, h))
    }

    /// Goes to `time` precisely.
    pub fn set_time(&self, time: Duration) {
        unsafe {
            sys::libvlc_media_player_set_time(self.mp.as_ptr(), time.as_micros() as i64, false)
        };
    }

    /// Jumps by `seconds`, back if negative.
    pub fn jump(&self, seconds: f64) {
        unsafe { sys::libvlc_media_player_jump_time(self.mp.as_ptr(), (seconds * 1e6) as i64) };
    }

    /// `percent` is 0–200; 100 is the source level.
    pub fn set_volume(&self, percent: u32) {
        unsafe { sys::libvlc_audio_set_volume(self.mp.as_ptr(), percent.min(200) as c_int) };
    }

    pub fn set_muted(&self, muted: bool) {
        unsafe { sys::libvlc_audio_set_mute(self.mp.as_ptr(), c_int::from(muted)) }
    }

    /// The current playback time, or `None` before the first frame.
    pub fn clock(&self) -> Option<Clock> {
        read_clock(&self.callbacks.clock)
    }

    /// The same clock, readable from any thread.
    pub fn clock_source(&self) -> ClockSource {
        ClockSource(self.callbacks.clock.clone())
    }

    /// The current title's chapters, and the one playing.
    pub fn chapters(&self) -> (Vec<Chapter>, Option<usize>) {
        unsafe {
            let mp = self.mp.as_ptr();
            let mut list = std::ptr::null_mut();
            let count = sys::libvlc_media_player_get_full_chapter_descriptions(mp, -1, &mut list);
            if count <= 0 || list.is_null() {
                return (Vec::new(), None);
            }
            let chapters = std::slice::from_raw_parts(list, count as usize)
                .iter()
                .filter_map(|c| c.as_ref())
                .map(|c| Chapter {
                    name: text(c.psz_name),
                    start: micros(c.i_time_offset),
                })
                .collect();
            sys::libvlc_chapter_descriptions_release(list, count as std::ffi::c_uint);
            let current = sys::libvlc_media_player_get_chapter(mp);
            (chapters, usize::try_from(current).ok())
        }
    }

    pub fn set_chapter(&self, index: usize) {
        unsafe { sys::libvlc_media_player_set_chapter(self.mp.as_ptr(), index as c_int) };
    }

    /// The current media's location and tags, as far as VLC has read them.
    pub fn meta(&self) -> Option<Meta> {
        unsafe {
            let media = sys::libvlc_media_player_get_media(self.mp.as_ptr());
            if media.is_null() {
                return None;
            }
            let get = |kind| owned_text(sys::libvlc_media_get_meta(media, kind));
            let meta = Meta {
                url: owned_text(sys::libvlc_media_get_mrl(media)),
                title: get(sys::libvlc_meta_Title),
                artist: get(sys::libvlc_meta_Artist),
                album: get(sys::libvlc_meta_Album),
                art_url: get(sys::libvlc_meta_ArtworkURL),
            };
            sys::libvlc_media_release(media);
            Some(meta)
        }
    }

    /// Enables or disables a VLC filter module (video, subpicture or audio)
    /// while playing; other filters stay enabled (patches/0001).
    pub fn set_filter(&self, name: &str, enable: bool) -> Result<(), String> {
        let name = c_string(name)?;
        check(unsafe {
            sys::libvlc_media_player_set_filter(self.mp.as_ptr(), name.as_ptr(), enable)
        })
    }

    /// Sets a filter option such as `sepia-intensity`, parsed by its VLC type.
    /// Running filters pick it up; filters enabled later start with it.
    pub fn set_filter_option(&self, option: &str, value: &str) -> Result<(), String> {
        let (option, value) = (c_string(option)?, c_string(value)?);
        check(unsafe {
            sys::libvlc_media_player_set_filter_option(
                self.mp.as_ptr(),
                option.as_ptr(),
                value.as_ptr(),
            )
        })
    }

    pub fn tracks(&self, kind: TrackKind) -> Vec<Track> {
        unsafe {
            let list = sys::libvlc_media_player_get_tracklist(self.mp.as_ptr(), kind.raw(), false);
            if list.is_null() {
                return Vec::new();
            }
            let tracks = (0..sys::libvlc_media_tracklist_count(list))
                .filter_map(|i| sys::libvlc_media_tracklist_at(list, i).as_ref())
                .map(|t| Track {
                    id: text(t.psz_id).unwrap_or_default(),
                    // VLC's display name, e.g. "Track 1 - [English]".
                    name: text(t.psz_name)
                        .or_else(|| text(t.psz_description))
                        .or_else(|| text(t.psz_language))
                        .unwrap_or_else(|| text(t.psz_id).unwrap_or_default()),
                    selected: t.selected,
                })
                .collect();
            sys::libvlc_media_tracklist_delete(list);
            tracks
        }
    }

    /// Selects the track with `id`, or no track of `kind` for `None`.
    pub fn select_track(&self, kind: TrackKind, id: Option<&str>) -> Result<(), String> {
        let mp = self.mp.as_ptr();
        match id {
            None => unsafe { sys::libvlc_media_player_unselect_track_type(mp, kind.raw()) },
            Some(id) => {
                let id = c_string(id)?;
                unsafe {
                    sys::libvlc_media_player_select_tracks_by_ids(mp, kind.raw(), id.as_ptr())
                }
            }
        }
        Ok(())
    }

    /// The audio output's devices and the current one's id.
    pub fn audio_devices(&self) -> (Vec<AudioDevice>, Option<String>) {
        unsafe {
            let mp = self.mp.as_ptr();
            let list = sys::libvlc_audio_output_device_enum(mp);
            let mut devices = Vec::new();
            let mut node = list;
            while let Some(d) = node.as_ref() {
                devices.push(AudioDevice {
                    id: text(d.psz_device).unwrap_or_default(),
                    name: text(d.psz_description)
                        .unwrap_or_else(|| text(d.psz_device).unwrap_or_default()),
                });
                node = d.p_next;
            }
            if !list.is_null() {
                sys::libvlc_audio_output_device_list_release(list);
            }
            let current = sys::libvlc_audio_output_device_get(mp);
            let current_id = text(current);
            if !current.is_null() {
                sys::libvlc_free(current.cast());
            }
            (devices, current_id)
        }
    }

    pub fn set_audio_device(&self, id: &str) -> Result<(), String> {
        let id = c_string(id)?;
        check(unsafe { sys::libvlc_audio_output_device_set(self.mp.as_ptr(), id.as_ptr()) })
    }

    /// Decoding and display counters for the current media.
    pub fn stats(&self) -> Option<Stats> {
        unsafe {
            let media = sys::libvlc_media_player_get_media(self.mp.as_ptr());
            if media.is_null() {
                return None;
            }
            let mut s = std::mem::zeroed::<sys::libvlc_media_stats_t>();
            let ok = sys::libvlc_media_get_stats(media, &mut s);
            sys::libvlc_media_release(media);
            ok.then_some(Stats {
                decoded: s.i_decoded_video,
                displayed: s.i_displayed_pictures,
                late: s.i_late_pictures,
                lost: s.i_lost_pictures,
            })
        }
    }
}

pub struct Stats {
    pub decoded: u64,
    pub displayed: u64,
    pub late: u64,
    pub lost: u64,
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let Self {
            decoded,
            displayed,
            late,
            lost,
        } = self;
        write!(
            f,
            "decoded {decoded} · displayed {displayed} · late {late} · lost {lost}"
        )
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // The last release stops playback and joins VLC's threads, so no
        // callback can reach `video` or `callbacks` after this.
        unsafe {
            sys::libvlc_media_player_unwatch_time(self.mp.as_ptr());
            sys::libvlc_media_player_release(self.mp.as_ptr());
        }
    }
}

/// Playback time from the last clock point, interpolated to now.
fn read_clock(clock: &Mutex<ClockPoint>) -> Option<Clock> {
    let (point, hold) = {
        let clock = clock.lock().unwrap();
        (clock.point?, clock.hold)
    };
    // VLC asserts that the date it interpolates to is > 0.
    let now = match hold {
        Hold::Running => unsafe { sys::libvlc_clock() },
        Hold::PausedAt(date) => date,
    };
    let (mut time, mut position) = (0, 0.0);
    let ok = unsafe {
        sys::libvlc_media_player_time_point_interpolate(&point, now, &mut time, &mut position)
    };
    // Interpolation runs on past the end until VLC reports it stopped.
    let length = micros(point.length_us);
    (ok == 0).then(|| Clock {
        time: if length.is_zero() {
            micros(time)
        } else {
            micros(time).min(length)
        },
        length,
        position: position.clamp(0.0, 1.0),
    })
}

/// A player's clock for other threads (the desktop's media controls).
#[derive(Clone)]
pub struct ClockSource(Arc<Mutex<ClockPoint>>);

impl ClockSource {
    pub fn read(&self) -> Option<Clock> {
        read_clock(&self.0)
    }
}

#[derive(Clone, Debug)]
pub struct Chapter {
    /// `None` when the file doesn't name it.
    pub name: Option<String>,
    pub start: Duration,
}

/// What VLC knows about the current media; tags are `None` until it has read them.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub url: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
}

/// libvlc_time_t is microseconds in libVLC 4.
fn micros(us: i64) -> Duration {
    Duration::from_micros(us.max(0) as u64)
}

// Player and time-watcher callbacks. `opaque` is the player's `Callbacks`,
// which outlive the player.

struct Callbacks {
    events: Box<dyn Fn(Event) + Send + Sync>,
    clock: Arc<Mutex<ClockPoint>>,
}

#[derive(Default)]
struct ClockPoint {
    point: Option<sys::libvlc_media_player_time_point_t>,
    hold: Hold,
}

/// How to read the last clock point.
#[derive(Clone, Copy, Default)]
enum Hold {
    /// Advance it to now.
    #[default]
    Running,
    /// Paused or stopped: advance it to this system date (µs, > 0).
    PausedAt(i64),
}

// The time point holds only numbers.
unsafe impl Send for ClockPoint {}

unsafe fn callbacks<'a>(opaque: *mut c_void) -> &'a Callbacks {
    unsafe { &*opaque.cast::<Callbacks>() }
}

static PLAYER_CBS: sys::libvlc_media_player_cbs = {
    let mut cbs: sys::libvlc_media_player_cbs = unsafe { std::mem::zeroed() };
    cbs.version = 0;
    cbs.on_media_stopping = Some(on_media_stopping);
    cbs.on_state_changed = Some(on_state_changed);
    cbs.on_length_changed = Some(on_length_changed);
    cbs.on_capabilities_changed = Some(on_capabilities_changed);
    cbs.on_track_list_changed = Some(on_track_list_changed);
    cbs.on_track_selection_changed = Some(on_track_selection_changed);
    cbs.on_recording_changed = Some(on_recording_changed);
    cbs.on_media_meta_changed = Some(on_media_meta_changed);
    cbs.on_titles_changed = Some(on_titles_changed);
    cbs.on_title_selection_changed = Some(on_title_selection_changed);
    cbs.on_chapter_selection_changed = Some(on_chapter_selection_changed);
    cbs
};

static TIME_CBS: sys::libvlc_media_player_watch_time_cbs = {
    let mut cbs: sys::libvlc_media_player_watch_time_cbs = unsafe { std::mem::zeroed() };
    cbs.version = 0;
    cbs.on_update = Some(on_time_update);
    cbs.on_paused = Some(on_time_paused);
    cbs.on_seek = Some(on_time_seek);
    cbs
};

fn state(state: sys::libvlc_state_t) -> State {
    match state {
        sys::libvlc_Opening => State::Opening,
        sys::libvlc_Playing => State::Playing,
        sys::libvlc_Paused => State::Paused,
        sys::libvlc_Stopping => State::Stopping,
        sys::libvlc_Stopped => State::Stopped,
        sys::libvlc_Error => State::Error,
        _ => State::Idle,
    }
}

unsafe extern "C" fn on_media_stopping(
    opaque: *mut c_void,
    _media: *mut sys::libvlc_media_t,
    reason: sys::libvlc_stopping_reason_t,
) {
    if reason != sys::libvlc_stopping_reason_user {
        (unsafe { callbacks(opaque) }.events)(Event::Ended);
    }
}

unsafe extern "C" fn on_recording_changed(
    opaque: *mut c_void,
    recording: bool,
    _file_path: *const c_char,
) {
    (unsafe { callbacks(opaque) }.events)(Event::Recording(recording));
}

unsafe extern "C" fn on_titles_changed(opaque: *mut c_void) {
    (unsafe { callbacks(opaque) }.events)(Event::ChaptersChanged);
}

unsafe extern "C" fn on_title_selection_changed(
    opaque: *mut c_void,
    _title: *const sys::libvlc_title_description_t,
    _idx: std::ffi::c_uint,
) {
    (unsafe { callbacks(opaque) }.events)(Event::ChaptersChanged);
}

unsafe extern "C" fn on_chapter_selection_changed(
    opaque: *mut c_void,
    _title: *const sys::libvlc_title_description_t,
    _title_idx: std::ffi::c_uint,
    _chapter: *const sys::libvlc_chapter_description_t,
    _chapter_idx: std::ffi::c_uint,
) {
    (unsafe { callbacks(opaque) }.events)(Event::ChaptersChanged);
}

unsafe extern "C" fn on_media_meta_changed(opaque: *mut c_void, _media: *mut sys::libvlc_media_t) {
    (unsafe { callbacks(opaque) }.events)(Event::MetaChanged);
}

unsafe extern "C" fn on_state_changed(opaque: *mut c_void, new: sys::libvlc_state_t) {
    (unsafe { callbacks(opaque) }.events)(Event::State(state(new)));
}

unsafe extern "C" fn on_length_changed(opaque: *mut c_void, _length: sys::libvlc_time_t) {
    (unsafe { callbacks(opaque) }.events)(Event::LengthChanged);
}

unsafe extern "C" fn on_capabilities_changed(
    opaque: *mut c_void,
    _old: sys::libvlc_capability_t,
    new: sys::libvlc_capability_t,
) {
    let seekable = new & sys::libvlc_capability_seek != 0;
    (unsafe { callbacks(opaque) }.events)(Event::Seekable(seekable));
}

/// Only added and removed tracks change the lists; VLC also reports every
/// update of a track's details, many times a second.
unsafe extern "C" fn on_track_list_changed(
    opaque: *mut c_void,
    action: sys::libvlc_list_action_t,
    _kind: sys::libvlc_track_type_t,
    _id: *const c_char,
) {
    if action != sys::libvlc_list_action_updated {
        (unsafe { callbacks(opaque) }.events)(Event::TracksChanged);
    }
}

unsafe extern "C" fn on_track_selection_changed(
    opaque: *mut c_void,
    _kind: sys::libvlc_track_type_t,
    _unselected: *const c_char,
    _selected: *const c_char,
) {
    (unsafe { callbacks(opaque) }.events)(Event::TracksChanged);
}

unsafe extern "C" fn on_time_update(
    opaque: *mut c_void,
    value: *const sys::libvlc_media_player_time_point_t,
) {
    let mut clock = unsafe { callbacks(opaque) }.clock.lock().unwrap();
    (clock.point, clock.hold) = (Some(unsafe { *value }), Hold::Running);
}

/// `system_date_us` is only valid (> 0) when paused; at the end of media or a
/// stop VLC reports its clock stopping with an invalid date, so the time is
/// held where it stopped, now. (The last update can be seconds old by then.)
unsafe extern "C" fn on_time_paused(opaque: *mut c_void, system_date_us: sys::libvlc_time_t) {
    let date = if system_date_us > 0 {
        system_date_us
    } else {
        unsafe { sys::libvlc_clock() }
    };
    unsafe { callbacks(opaque) }.clock.lock().unwrap().hold = Hold::PausedAt(date);
}

/// `value` is the seek target while seeking, NULL once it settled.
unsafe extern "C" fn on_time_seek(
    opaque: *mut c_void,
    value: *const sys::libvlc_media_player_time_point_t,
) {
    let Some(&point) = (unsafe { value.as_ref() }) else {
        return;
    };
    let callbacks = unsafe { callbacks(opaque) };
    callbacks.clock.lock().unwrap().point = Some(point);
    (callbacks.events)(Event::Seeked(micros(point.ts_us)));
}

/// A string libVLC handed over (to be freed); `None` for NULL or empty.
unsafe fn owned_text(ptr: *mut c_char) -> Option<String> {
    let s = text(ptr);
    if !ptr.is_null() {
        unsafe { sys::libvlc_free(ptr.cast()) };
    }
    s
}

/// A string libVLC owns; `None` for NULL or empty.
fn text(ptr: *const c_char) -> Option<String> {
    let s = unsafe { ptr.as_ref() }
        .map(|p| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())?;
    (!s.is_empty()).then_some(s)
}

fn c_string(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("{s:?} contains a NUL byte"))
}

/// libVLC's 0 / -1 convention, with its message for the calling thread.
fn check(ret: std::ffi::c_int) -> Result<(), String> {
    if ret == 0 {
        return Ok(());
    }
    let message = unsafe { sys::libvlc_errmsg() };
    Err(if message.is_null() {
        "libVLC error".into()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    })
}

// libVLC OpenGL output callbacks. `opaque` is the player's `Arc<Bridge>`,
// which outlives the player.

unsafe fn bridge<'a>(opaque: *mut c_void) -> &'a Bridge {
    unsafe { &*opaque.cast::<Bridge>() }
}

unsafe extern "C" fn setup(
    _opaque: *mut *mut c_void,
    _cfg: *const sys::libvlc_video_setup_device_cfg_t,
    _out: *mut sys::libvlc_video_setup_device_info_t,
) -> bool {
    true
}

unsafe extern "C" fn cleanup(opaque: *mut c_void) {
    unsafe { bridge(opaque) }.release_gl();
}

unsafe extern "C" fn update_output(
    opaque: *mut c_void,
    cfg: *const sys::libvlc_video_render_cfg_t,
    out: *mut sys::libvlc_video_output_cfg_t,
) -> bool {
    let (cfg, out) = unsafe { (&*cfg, &mut *out) };
    if !unsafe { bridge(opaque) }.resize(cfg.width, cfg.height) {
        return false;
    }
    out.u.opengl_format = glow::RGBA as i32;
    out.full_range = true;
    out.colorspace = sys::libvlc_video_colorspace_BT709;
    out.primaries = sys::libvlc_video_primaries_BT709;
    out.transfer = sys::libvlc_video_transfer_func_SRGB;
    out.orientation = sys::libvlc_video_orient_top_left;
    true
}

unsafe extern "C" fn swap(opaque: *mut c_void) {
    unsafe { bridge(opaque) }.frame_done();
}

unsafe extern "C" fn make_current(opaque: *mut c_void, enter: bool) -> bool {
    unsafe { bridge(opaque) }.make_current(enter)
}

unsafe extern "C" fn get_proc_address(opaque: *mut c_void, name: *const c_char) -> *mut c_void {
    let name = unsafe { CStr::from_ptr(name) };
    unsafe { bridge(opaque) }.proc_address(name).cast_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_a_playlist_into_its_entries() {
        let dir = std::env::temp_dir().join(format!("ferricone-expand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let list = dir.join("channels.m3u");
        std::fs::write(
            &list,
            "#EXTM3U\n#EXTINF:-1,Channel One\nhttps://example.com/one.m3u8\n\
             #EXTINF:-1,Channel Two\nhttps://example.com/two.m3u8\n",
        )
        .unwrap();
        let vlc = Vlc::new(&["--quiet".into()]).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let expander = vlc
            .expander(Duration::from_secs(10), move |_, entries| {
                let _ = tx.lock().unwrap().send(entries);
            })
            .unwrap();
        let url = format!("file://{}", list.display());
        expander.expand(&url).unwrap();
        let entries = rx.recv_timeout(Duration::from_secs(15)).unwrap().expect("entries");
        drop(expander);
        let _ = std::fs::remove_dir_all(&dir);
        let got: Vec<_> = entries
            .iter()
            .map(|e| (e.mrl.as_str(), e.title.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("https://example.com/one.m3u8", Some("Channel One")),
                ("https://example.com/two.m3u8", Some("Channel Two")),
            ]
        );
    }
}
