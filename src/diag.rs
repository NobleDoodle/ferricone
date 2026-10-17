//! Development probes, off unless their environment variable is set.
//!
//! - `FERRICONE_STATS=1` prints VLC's frame counters every two seconds.
//! - `FERRICONE_FILTER_EVERY=<seconds>` steps through live filter changes
//!   (patches/0001): sepia on, its intensity changed, sepia off, then the audio
//!   compressor on with a new ratio. After each step it prints the longest gap
//!   between shown frames in the next two seconds, the freeze the change costs.
//! - `FERRICONE_VLC_ARGS="--verbose=2 …"` passes options to libVLC.
//! - `FERRICONE_TRACE=1` logs player events and the commands the UI sends.
//! - `FERRICONE_OPEN_PANEL=video|audio|subtitles` opens the tracks panel at
//!   start, `menu` the right-click menu, `open-media` / `playlists` that
//!   with its Open Media / Playlist submenu, `save-queue` New Playlist with
//!   the queue in it, `save-streams` the network dialog saving to the
//!   library, `library-path:<page>:<row>:…` the dock opening those rows,
//!   `library-menu:<page>:<row>[:watch]` a row's right-click menu,
//!   `settings:<page>[:<search>]` the Settings window, `shortcuts` the
//!   Keyboard Shortcuts dialog, `addsubs:<video>[:folder]` Add Subtitles,
//!   `about` About Ferricone, `recent-menu` ⋯ › Open Recent,
//!   `timeline-adjust` the subtitle timeline adjusting, `intro` /
//!   `intro-theme` / `intro-keys` the introduction at that step,
//!   `appearance` its theme step alone, `playback` / `video-settings` / `audio-settings` /
//!   `subtitle-settings` the settings sidebar on that tab (`color-picker`,
//!   `color-grid` and `font-picker` with the text color's picker on the
//!   Spectrum or Grid tab, or the font list, open),
//!   `library` / `library-videos` / `library-recent` / `library-folder` the
//!   library dock at Home, Videos, Recent or the loaded item's folder
//!   (`library-full` / `library-full-videos` the full library at Home or
//!   Videos),
//!   `folder-preset`
//!   its footer's "This Folder",
//!   `more` the ⋯ menu, `network` the network dialog, `waveform` the
//!   waveform, `preview` the seek preview at the middle of the track, and
//!   `queue`, `bookmarks` or `chapters` the queue panel on that tab, for
//!   screenshots without a pointer.

use crate::vlc::Player;
use slint::ComponentHandle;
use std::{
    cell::Cell,
    rc::Rc,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub fn tracing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FERRICONE_TRACE").is_some())
}

/// `eprintln!` when `FERRICONE_TRACE` is set.
macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::diag::tracing() {
            eprintln!("ferricone: {}", format_args!($($arg)*));
        }
    };
}
pub(crate) use trace;

const WINDOW: Duration = Duration::from_secs(2);

/// (description, filter toggle, filter option)
type Step = (
    &'static str,
    Option<(&'static str, bool)>,
    Option<(&'static str, &'static str)>,
);

const FILTER_STEPS: [Step; 4] = [
    (
        "sepia on",
        Some(("sepia", true)),
        Some(("sepia-intensity", "120")),
    ),
    (
        "sepia intensity 255",
        None,
        Some(("sepia-intensity", "255")),
    ),
    ("sepia off", Some(("sepia", false)), None),
    (
        "compressor on, ratio 8",
        Some(("compressor", true)),
        Some(("compressor-ratio", "8")),
    ),
];

pub struct Diagnostics {
    _timers: Vec<slint::Timer>,
    freeze: Rc<FreezeProbe>,
}

#[derive(Default)]
struct FreezeProbe {
    step: Cell<Option<(&'static str, Instant)>>,
    last_frame: Cell<Option<Instant>>,
    longest_gap: Cell<Duration>,
}

impl Diagnostics {
    pub fn from_env(session: &crate::controls::Shared) -> Self {
        let mut timers = Vec::new();
        if std::env::var_os("FERRICONE_STATS").is_some() {
            let session = session.clone();
            timers.push(repeat(Duration::from_secs(2), move || {
                if let Some(stats) = session.player.borrow().as_ref().and_then(Player::stats) {
                    eprintln!("ferricone: {stats}");
                }
            }));
        }

        let freeze = Rc::new(FreezeProbe::default());
        let every = std::env::var("FERRICONE_FILTER_EVERY")
            .ok()
            .and_then(|s| s.parse().ok());
        if let Some(seconds) = every {
            let (session, probe, next) = (session.clone(), freeze.clone(), Cell::new(0));
            timers.push(repeat(Duration::from_secs_f32(seconds), move || {
                let Some(p) = &*session.player.borrow() else {
                    return;
                };
                let (name, toggle, option) = FILTER_STEPS[next.get() % FILTER_STEPS.len()];
                next.set(next.get() + 1);
                probe.step.set(Some((name, Instant::now())));
                probe.longest_gap.set(Duration::ZERO);
                let result = toggle
                    .map_or(Ok(()), |(filter, on)| p.set_filter(filter, on))
                    .and_then(|()| option.map_or(Ok(()), |(o, v)| p.set_filter_option(o, v)));
                if let Err(e) = result {
                    eprintln!("ferricone: {name}: {e}");
                }
            }));
        }
        Self {
            _timers: timers,
            freeze,
        }
    }

    /// Call whenever the window starts showing a new frame.
    pub fn frame_shown(&self) {
        let probe = &self.freeze;
        let now = Instant::now();
        let last = probe.last_frame.replace(Some(now));
        let Some((name, since)) = probe.step.get() else {
            return;
        };
        if let Some(last) = last {
            probe
                .longest_gap
                .set(probe.longest_gap.get().max(now - last));
        }
        if now - since >= WINDOW {
            probe.step.set(None);
            let ms = probe.longest_gap.get().as_millis();
            eprintln!("ferricone: {name}: longest frame gap {ms} ms");
        }
    }
}

pub fn open_panel(app: &crate::AppWindow) {
    use crate::TracksTab;
    let tab = match std::env::var("FERRICONE_OPEN_PANEL").as_deref() {
        Ok("video") => TracksTab::Video,
        Ok("audio") => TracksTab::Audio,
        Ok("subtitles") => TracksTab::Subtitles,
        Ok("menu") => return app.invoke_open_context_menu(60.0, 50.0),
        Ok("open-media") => {
            app.invoke_open_context_menu(60.0, 50.0);
            // The Open Media row: 12 rows and 3 dividers below the top padding.
            app.set_submenu_y(50.0 + 4.0 + 12.0 * 48.0 + 3.0);
            return app.set_submenu(crate::MenuAction::OpenMedia);
        }
        Ok("more") => return app.invoke_show_more_menu(),
        Ok("playlists") => {
            app.invoke_open_context_menu(60.0, 50.0);
            // The Playlist row: 11 rows and 3 dividers below the top padding.
            app.set_submenu_y(50.0 + 4.0 + 11.0 * 48.0 + 3.0);
            return app.set_submenu(crate::MenuAction::Playlist);
        }
        Ok("save-queue") => return app.invoke_new_playlist(true),
        Ok("save-streams") => return app.invoke_show_save_streams_dialog(),
        Ok("shortcuts") => return app.invoke_open_shortcuts(),
        Ok("about") => return app.invoke_show_about(),
        Ok("recent-menu") => {
            app.invoke_show_more_menu();
            // The Open Recent row: 6 rows below the top padding.
            app.set_submenu_y(40.0 + 4.0 + 6.0 * 48.0);
            return app.set_submenu(crate::MenuAction::OpenRecent);
        }
        Ok(spec) if spec.starts_with("addsubs:") => {
            // addsubs:<video>[:folder], Add Subtitles for it.
            let rest = &spec["addsubs:".len()..];
            let (video, folder) = match rest.strip_suffix(":folder") {
                Some(v) => (v, true),
                None => (rest, false),
            };
            let (video, online) = match video.strip_suffix(":online") {
                Some(v) => (v, true),
                None => (video, false),
            };
            crate::addsubs::open(app, std::path::Path::new(video), folder);
            if online {
                app.invoke_addsubs_set_online(true);
                app.invoke_addsubs_search(app.get_addsubs_query(), app.get_addsubs_language());
            }
            return;
        }
        Ok("intro") => return app.invoke_show_intro(0, false),
        Ok("intro-theme") => return app.invoke_show_intro(1, false),
        Ok("intro-keys") => return app.invoke_show_intro(2, false),
        Ok("appearance") => return app.invoke_show_intro(1, true),
        Ok("timeline-adjust") => return app.invoke_set_timeline_adjust(true),
        Ok(page) if page.starts_with("settings:") => {
            // settings:<page>[:advanced|:<search>], the Settings window.
            let mut parts = page.split(':').skip(1);
            app.set_prefs_page(parts.next().and_then(|p| p.parse().ok()).unwrap_or(0));
            app.invoke_open_preferences();
            if let Some(query) = parts.next() {
                app.invoke_prefs_search(query.into());
            }
            return;
        }
        Ok(path) if path.starts_with("library-menu:") => {
            // library-menu:<page>:<row>[:watch], the row's right-click menu
            // (and Mark as Watched) once the page is shown.
            let parts: Vec<&str> = path.split(':').skip(1).collect();
            let page = parts.first().and_then(|p| p.parse().ok()).unwrap_or(1);
            let row: i32 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
            let watch = parts.get(2) == Some(&"watch");
            app.set_dock_open(true);
            app.invoke_lib_go_page(page);
            let ui = app.as_weak();
            slint::Timer::single_shot(Duration::from_millis(1500), move || {
                if let Some(app) = ui.upgrade() {
                    app.invoke_open_library_row_menu(row, 120.0, 240.0);
                    if watch {
                        app.invoke_lib_menu_action(crate::MenuAction::MarkWatched, 0);
                    }
                }
            });
            return;
        }
        Ok(path) if path.starts_with("library-path:") => {
            // library-path:<page>:<row>:<row>…, a row every 1.5 s.
            let mut steps = path.split(':').skip(1).filter_map(|n| n.parse::<i32>().ok());
            app.set_dock_open(true);
            app.invoke_lib_go_page(steps.next().unwrap_or(0));
            for (i, row) in steps.enumerate() {
                let ui = app.as_weak();
                slint::Timer::single_shot(Duration::from_millis(1500 * (i as u64 + 1)), move || {
                    if let Some(app) = ui.upgrade() {
                        app.invoke_lib_open(row);
                    }
                });
            }
            return;
        }
        Ok(
            page @ ("library" | "library-videos" | "library-recent" | "library-network"
            | "library-full" | "library-full-videos"),
        ) => {
            if page.starts_with("library-full") {
                app.set_lib_full(true);
            } else {
                app.set_dock_open(true);
            }
            match page {
                "library-videos" | "library-full-videos" => app.invoke_lib_go_page(1),
                "library-recent" => app.invoke_lib_go_page(5),
                "library-network" => app.invoke_lib_go_page(3),
                _ => app.invoke_lib_go_page(0),
            }
            return;
        }
        Ok("library-folder") => {
            // Once the item is loaded.
            let ui = app.as_weak();
            slint::Timer::single_shot(Duration::from_secs(2), move || {
                if let Some(app) = ui.upgrade() {
                    app.invoke_lib_reveal();
                }
            });
            return;
        }
        Ok(picker @ ("color-picker" | "color-grid" | "font-picker")) => {
            app.set_settings_tab(crate::SettingsTab::Subtitles);
            if picker == "color-grid" {
                app.global::<crate::Palette>().set_tab(0);
            }
            let picker = picker.replace("color-grid", "color-picker");
            app.global::<crate::PickerDiag>().set_open(picker.into());
            return app.set_settings_open(true);
        }
        Ok(tab @ ("playback" | "video-settings" | "audio-settings" | "subtitle-settings")) => {
            app.set_settings_tab(match tab {
                "playback" => crate::SettingsTab::Playback,
                "video-settings" => crate::SettingsTab::Video,
                "audio-settings" => crate::SettingsTab::Audio,
                _ => crate::SettingsTab::Subtitles,
            });
            return app.set_settings_open(true);
        }
        Ok("folder-preset") => {
            // The footer's "This Folder", once the item is loaded.
            let ui = app.as_weak();
            slint::Timer::single_shot(Duration::from_secs(2), move || {
                if let Some(app) = ui.upgrade() {
                    app.invoke_choose_preset(1);
                }
            });
            return;
        }
        Ok("network") => return app.invoke_show_network_dialog(),
        Ok("waveform") => return app.set_waveform_on(true),
        Ok(tab @ ("queue" | "bookmarks" | "chapters")) => {
            app.set_queue_tab(match tab {
                "queue" => crate::QueueTab::Queue,
                "bookmarks" => crate::QueueTab::Bookmarks,
                _ => crate::QueueTab::Chapters,
            });
            return app.set_queue_open(true);
        }
        Ok("preview") => {
            // Once the length is known.
            let ui = app.as_weak();
            slint::Timer::single_shot(Duration::from_secs(2), move || {
                if let Some(app) = ui.upgrade() {
                    app.set_preview_at(0.5);
                    app.set_preview_pinned(true);
                    app.invoke_preview(0.5);
                }
            });
            return;
        }
        _ => return,
    };
    app.set_tracks_tab(tab);
    app.set_tracks_open(true);
}

/// libVLC options from `FERRICONE_VLC_ARGS`, split on whitespace.
/// The Settings window's libVLC options, then FERRICONE_VLC_ARGS (which
/// can override them).
pub fn vlc_args() -> Vec<String> {
    let mut args = crate::prefs::get().vlc_args();
    if let Ok(extra) = std::env::var("FERRICONE_VLC_ARGS") {
        args.extend(extra.split_whitespace().map(String::from));
    }
    args
}

fn repeat(interval: Duration, tick: impl FnMut() + 'static) -> slint::Timer {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, interval, tick);
    timer
}
