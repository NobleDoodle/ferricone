//! Connects the player to the window: VLC's events and clock become window
//! properties, and the bars', menus' and keys' actions become player commands.

use crate::{
    AppWindow, TrackItem, TracksTab,
    diag::trace,
    media::{self, Queue, Source},
    queue::{self, Lengths},
    remote::{self, Command},
    seekbar::SeekExtras,
    state::{Bookmarks, Positions},
    vlc::{Clock, Event, Player, State, Track, TrackKind, Vlc},
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

/// While playing, the place is also kept this often, in case the player
/// never gets to close normally (a crash, or the session ending).
const KEEP_EVERY: Duration = Duration::from_secs(30);

/// What playback shares on the UI thread.
pub struct Session {
    /// Made once Slint's GL context exists (main.rs).
    pub player: RefCell<Option<Player>>,
    /// The seek preview and the waveform.
    pub extras: SeekExtras,
    pub(crate) queue: RefCell<Queue>,
    pub(crate) lengths: RefCell<Lengths>,
    pub(crate) positions: RefCell<Positions>,
    /// The library dock's place, rows and thumbnails.
    pub(crate) library: crate::library::Library,
    /// What was played when and how far, for the library.
    pub(crate) history: RefCell<crate::state::History>,
    pub(crate) bookmarks: RefCell<Bookmarks>,
    /// The item opened in the player, whose place `remember` keeps.
    pub(crate) loaded: RefCell<Option<Source>>,
    /// The file after the loaded one in its folder, for when nothing is
    /// queued after it: Next and the end of the item go on there.
    pub(crate) folder_next: RefCell<Option<std::path::PathBuf>>,
    /// ⋯ › Open Recent's items as the menu lists them.
    pub(crate) recent: RefCell<Vec<Source>>,
    /// The saved playlists as the menu lists them.
    pub(crate) playlists: RefCell<Vec<crate::playlists::Playlist>>,
    /// The New Playlist dialog's files and cover.
    pub(crate) draft: RefCell<Vec<Source>>,
    pub(crate) draft_cover: RefCell<Option<std::path::PathBuf>>,
    /// The settings sidebar's presets, whether the loaded item has had its
    /// preset applied yet, and the sidebar's timers.
    pub(crate) presets: RefCell<crate::presets::Presets>,
    pub(crate) preset_applied: Cell<bool>,
    pub(crate) running: RefCell<crate::settings::Running>,
    pub(crate) timeline: crate::subtimeline::Timeline,
    /// Where the next item opened starts (casting reopens the item where
    /// it was).
    pub(crate) start_at: Cell<Option<Duration>>,
    /// Where the resume pill continues from.
    resume_at: Cell<Option<Duration>>,
    /// The length chip shows the time left.
    remaining: Cell<bool>,
    kept: Cell<Instant>,
}

pub type Shared = Rc<Session>;

impl Session {
    pub fn new(vlc: Rc<Vlc>) -> Shared {
        Rc::new(Self {
            player: RefCell::default(),
            extras: SeekExtras::new(vlc.clone()),
            library: crate::library::Library::new(vlc),
            queue: RefCell::default(),
            lengths: RefCell::default(),
            positions: RefCell::new(Positions::load()),
            history: RefCell::new(crate::state::History::load()),
            bookmarks: RefCell::new(Bookmarks::load()),
            loaded: RefCell::default(),
            folder_next: RefCell::default(),
            playlists: RefCell::default(),
            recent: RefCell::default(),
            draft: RefCell::default(),
            draft_cover: RefCell::default(),
            presets: RefCell::new(crate::presets::Presets::load()),
            preset_applied: Cell::default(),
            running: RefCell::default(),
            timeline: crate::subtimeline::Timeline::default(),
            resume_at: Cell::default(),
            start_at: Cell::default(),
            remaining: Cell::default(),
            kept: Cell::new(Instant::now()),
        })
    }

    /// Keeps the loaded item's place for the resume pill, or forgets it when
    /// it was watched to the end. Call before the player moves on or stops.
    pub fn remember(&self) {
        self.kept.set(Instant::now());
        let clock = self.player.borrow().as_ref().and_then(Player::clock);
        if let (Some(source), Some(clock)) = (&*self.loaded.borrow(), clock) {
            // Settings › Privacy can turn either off, or skip some items.
            let prefs = crate::prefs::get();
            if prefs.excluded(source) {
                return;
            }
            // Back on an item that started from the top with its resume offer
            // not taken: an earlier time must not replace the place it was
            // left (switching away a minute in would lose the rest).
            if self.resume_at.get().is_some_and(|at| clock.time < at) {
                return;
            }
            if prefs.remember_position {
                self.positions
                    .borrow_mut()
                    .remember(source, clock.time, clock.length);
            }
            if prefs.save_history {
                self.history
                    .borrow_mut()
                    .note(source, clock.time, clock.length);
            }
        }
    }

    pub(crate) fn with_player(&self, f: impl FnOnce(&Player)) {
        if let Some(p) = &*self.player.borrow() {
            f(p);
        }
    }
}

/// Plays `sources` (folders expanded) in place of what was open.
pub fn open(app: &AppWindow, s: &Session, sources: Vec<Source>) {
    let sources = media::expand(sources);
    if sources.is_empty() {
        eprintln!("ferricone: nothing to play there");
        return;
    }
    trace!("open {} item(s)", sources.len());
    s.queue.borrow_mut().replace(sources);
    play_current(app, s);
}

/// Adds `sources` (folders expanded) after what is queued; plays them when
/// nothing was.
pub fn append(app: &AppWindow, s: &Session, sources: Vec<Source>) {
    let sources = media::expand(sources);
    let was_empty = s.queue.borrow().items().is_empty();
    s.queue.borrow_mut().append(sources);
    if was_empty {
        play_current(app, s);
    } else {
        queue::refresh(app, s);
    }
}

/// Opens the queue's current item, or waits for the player if it isn't
/// made yet (main.rs calls this again once it is).
pub fn play_current(app: &AppWindow, s: &Session) {
    s.remember();
    let queue = s.queue.borrow();
    let Some(source) = queue.current() else {
        return;
    };
    app.set_media_title(source.title().into());
    app.set_folder_name(source.folder().into());
    app.set_has_media(true);
    app.set_position(0.0);
    app.set_time_text(hms(Duration::ZERO).into());
    app.set_length_text(hms(Duration::ZERO).into());
    let track = remote::Track {
        title: source.title(),
        ..Default::default()
    };
    remote::new_track(track, queue.has_previous(), queue.has_next());

    // Where it was left: offered by the resume pill (playback starts from
    // the top), continued at once, or ignored (Settings › Playback).
    let prefs = crate::prefs::get();
    let resume_at = match prefs.resume {
        crate::prefs::Resume::Never => None,
        _ if !prefs.remember_position => None,
        _ => s.positions.borrow().get(source),
    };
    let resume_now = prefs.resume == crate::prefs::Resume::Always;
    s.resume_at.set(resume_at.filter(|_| !resume_now));
    if let Some(at) = resume_at {
        app.set_resume_time(hms(at).into());
    }
    app.set_resume_shown(resume_at.is_some() && !resume_now);

    // Audio shows its cover; assume video until the tracks say otherwise.
    app.set_has_video(true);
    app.set_cover(slint::Image::default());
    app.set_cover_blur(slint::Image::default());
    app.set_music_caption(source.title().into());
    let file = match source {
        Source::File(path) => Some(path.clone()),
        Source::Location(_) => None,
    };
    s.extras.reset(app, file);
    if app.get_waveform_on() {
        s.extras.want_waveform(app);
    }

    *s.loaded.borrow_mut() = Some(source.clone());
    s.preset_applied.set(false);
    *s.folder_next.borrow_mut() = match source {
        Source::File(path) => media::next_in_folder(path),
        Source::Location(_) => None,
    };
    app.set_up_next_shown(false);
    // The preset's preferred tracks apply as the item opens, and the
    // Playback settings' resume and start paused.
    let mut options = crate::settings::open_options(s);
    if let Some(at) = s.start_at.take() {
        options.push(format!(":start-time={:.3}", at.as_secs_f64()));
        app.set_resume_shown(false);
    } else if let (true, Some(at)) = (resume_now, resume_at) {
        options.push(format!(":start-time={:.3}", at.as_secs_f64()));
    }
    if !prefs.autoplay {
        options.push(":start-paused".into());
    }
    let mut opened = false;
    s.with_player(|p| {
        opened = true;
        if let Err(e) = p.open(source, &options) {
            eprintln!("ferricone: {e}");
        }
    });
    // Subtitles chosen in Add Subtitles.
    if let (true, Source::File(path)) = (opened, source) {
        crate::addsubs::load_into(s, path);
    }
    if opened && prefs.save_history && !prefs.excluded(source) {
        s.history.borrow_mut().started(source);
    }
    crate::settings::apply_on_open(s);
    drop(queue);
    crate::library::refresh(app, s);
    queue::refresh(app, s);
    queue::refresh_marks(app, s);
    crate::settings::refresh(app, s);
    app.set_chapters(ModelRc::default());
}

/// Queues the next file in the loaded one's folder and plays it; false
/// when there is none.
pub fn play_folder_next(app: &AppWindow, s: &Session) -> bool {
    let Some(next) = s.folder_next.borrow_mut().take() else {
        return false;
    };
    trace!("next in folder: {}", next.display());
    {
        let mut queue = s.queue.borrow_mut();
        queue.append(vec![Source::File(next)]);
        let last = queue.items().len() - 1;
        queue.play_at(last);
    }
    play_current(app, s);
    true
}

/// Forwards VLC's events to the window and the desktop's media controls.
/// Called on VLC's threads.
pub fn event_sink(app: &AppWindow) -> impl Fn(Event) + Send + Sync + 'static {
    let ui = app.as_weak();
    move |event| {
        remote::observe(&event);
        let ui = ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = ui.upgrade() {
                apply(&app, event);
            }
        });
    }
}

fn apply(app: &AppWindow, event: Event) {
    trace!("event {event:?}");
    match event {
        Event::State(state) => {
            app.set_playing(matches!(state, State::Opening | State::Playing));
            if state == State::Playing {
                app.invoke_media_started();
                // The audio output lists its devices once it has started.
                if app.get_tracks_open() && app.get_tracks_tab() == crate::TracksTab::Audio {
                    app.invoke_refresh_devices();
                }
            }
            app.invoke_tick(); // the clock timer only runs while playing
        }
        Event::LengthChanged => app.invoke_tick(),
        Event::Seekable(seekable) => app.set_seekable(seekable),
        Event::TracksChanged => app.invoke_refresh_tracks(),
        Event::Ended => app.invoke_media_ended(),
        Event::Recording(on) => app.set_recording(on),
        Event::Seeked(_) => {}
        Event::MetaChanged => app.invoke_refresh_meta(),
        Event::ChaptersChanged => app.invoke_refresh_chapters(),
    }
}

/// Carries out what the desktop's media controls ask.
pub fn remote_command(app: &AppWindow, command: Command) {
    trace!("media controls: {command:?}");
    match command {
        Command::Play if !app.get_playing() => app.invoke_toggle_play(),
        Command::Pause if app.get_playing() => app.invoke_toggle_play(),
        Command::Play | Command::Pause => {}
        Command::PlayPause => app.invoke_toggle_play(),
        Command::Stop => app.invoke_stop(),
        Command::Next => app.invoke_next(),
        Command::Previous => app.invoke_previous(),
        Command::Seek(seconds) => app.invoke_jump(seconds as f32),
        Command::SetPosition(position) => app.invoke_seek(position as f32, false),
        Command::SetVolume(volume) => app.invoke_set_volume(volume as f32),
        Command::Open(uri) => app.invoke_open_locations(uri.into()),
        Command::Raise => {
            use slint::winit_030::WinitWindowAccessor;
            app.window().with_winit_window(|w| w.focus_window());
        }
        Command::Quit => {
            let _ = slint::quit_event_loop();
        }
    }
}

pub fn connect(app: &AppWindow, session: &Shared) {
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_tick(move || {
        let clock = s.player.borrow().as_ref().and_then(Player::clock);
        if let (Some(app), Some(clock)) = (ui.upgrade(), clock) {
            show_clock(&app, clock, s.remaining.get());
            s.extras.set_length(clock.length);
            crate::subtimeline::tick(&app, &s, &clock);
        }
        // Ticks run only while playing.
        if s.kept.get().elapsed() >= KEEP_EVERY {
            s.remember();
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_toggle_play(move || {
        // With nothing open, Play asks for something to open, as VLC does.
        if s.queue.borrow().current().is_none() {
            if let Some(app) = ui.upgrade() {
                app.invoke_open_files();
            }
            return;
        }
        let state = s.player.borrow().as_ref().map(Player::state);
        trace!("toggle-play in {state:?}");
        if matches!(state, Some(State::Playing | State::Opening)) {
            s.remember();
        }
        s.with_player(|p| match p.state() {
            State::Playing | State::Opening => p.set_paused(true),
            State::Paused => p.set_paused(false),
            _ => p.restart(),
        });
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_stop(move || {
        s.remember();
        s.with_player(Player::stop);
        if let Some(app) = ui.upgrade() {
            app.set_position(0.0);
            app.set_time_text(hms(Duration::ZERO).into());
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_next(move || {
        let advanced = s.queue.borrow_mut().next().is_some();
        let Some(app) = ui.upgrade() else { return };
        if advanced {
            play_current(&app, &s);
        } else if !play_folder_next(&app, &s) {
            s.remember();
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_play_up_next(move || {
        if let Some(app) = ui.upgrade() {
            play_folder_next(&app, &s);
        }
    });

    // The previous item, or the start of the first one.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_previous(move || {
        let moved = s.queue.borrow_mut().previous().is_some();
        if moved {
            if let Some(app) = ui.upgrade() {
                play_current(&app, &s);
            }
        } else {
            // A stopped player ignores seeks: play it again instead.
            s.with_player(|p| match p.state() {
                State::Stopped | State::Stopping | State::Error => p.restart(),
                _ => p.seek(0.0, false),
            });
        }
    });

    // The resume pill's Continue Watching.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_resume(move || {
        if let Some(at) = s.resume_at.take() {
            trace!("resume at {at:?}");
            s.with_player(|p| p.set_time(at));
        }
        if let Some(app) = ui.upgrade() {
            app.set_resume_shown(false);
            app.invoke_tick();
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_toggle_record(move || {
        if let Some(app) = ui.upgrade() {
            let dir = crate::prefs::get().record_dir;
            let dir = (!dir.is_empty()).then(|| std::path::PathBuf::from(dir));
            s.with_player(|p| p.set_recording(!app.get_recording(), dir.as_deref()));
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_seek(move |position, fast| {
        // Seek precisely (Settings › Playback) off: always the quick way.
        let fast = fast || !crate::prefs::get().precise_seek;
        s.with_player(|p| p.seek(f64::from(position), fast));
        if let Some(app) = ui.upgrade() {
            app.invoke_tick();
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_jump(move |seconds| {
        s.with_player(|p| p.jump(f64::from(seconds)));
        if let Some(app) = ui.upgrade() {
            app.invoke_tick();
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_volume(move |volume| {
        let volume = volume.clamp(0.0, 2.0);
        s.with_player(|p| p.set_volume((volume * 100.0).round() as u32));
        if let Some(app) = ui.upgrade() {
            app.set_volume(volume);
        }
        remote::set_volume(f64::from(volume));
    });

    // VLC read the item's tags: the desktop shows them where there are any.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_refresh_meta(move || {
        let Some(source) = s.loaded.borrow().clone() else {
            return;
        };
        let meta = s.player.borrow().as_ref().and_then(Player::meta);
        let meta = meta.unwrap_or_default();
        // VLC keeps covers it found (embedded or beside the file) as files.
        let cover = meta
            .art_url
            .as_deref()
            .and_then(|url| match Source::parse(url)? {
                Source::File(path) => slint::Image::load_from_path(&path).ok(),
                Source::Location(_) => None,
            });
        let Some(app) = ui.upgrade() else { return };
        if let Some(cover) = cover {
            app.set_cover_blur(crate::art::blurred(&cover).unwrap_or_default());
            app.set_cover(cover);
        }
        let title = meta.title.clone().unwrap_or_else(|| source.title());
        app.set_music_caption(
            match &meta.artist {
                Some(artist) if !artist.is_empty() => format!("{title} - {artist}"),
                _ => title,
            }
            .into(),
        );
        remote::update_track(remote::Track {
            title: meta.title.unwrap_or_else(|| source.title()),
            artist: meta.artist,
            album: meta.album,
            art_url: meta.art_url,
            url: meta.url,
        });
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_toggle_mute(move || {
        let Some(app) = ui.upgrade() else { return };
        let muted = !app.get_muted();
        s.with_player(|p| p.set_muted(muted));
        app.set_muted(muted);
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_refresh_tracks(move || {
        let (Some(app), Some(p)) = (ui.upgrade(), &*s.player.borrow()) else {
            return;
        };
        let (video, audio) = (p.tracks(TrackKind::Video), p.tracks(TrackKind::Audio));
        // Audio-only means audio tracks and no video; with no tracks at all
        // (an item ending or stopping) the layout stays as it was.
        if !video.is_empty() || !audio.is_empty() {
            app.set_has_video(!video.is_empty());
        }
        app.set_video_tracks(track_items(video));
        app.set_audio_tracks(track_items(audio));
        app.set_subtitle_tracks(track_items(p.tracks(TrackKind::Subtitle)));
        crate::subtimeline::reload(&app, &s);
    });

    let s = session.clone();
    app.on_select_track(move |tab, id| {
        let kind = track_kind(tab);
        trace!("select {kind:?} track {id:?}");
        // VLC reports the new selection, which refreshes the lists.
        s.with_player(|p| {
            if let Err(e) = p.select_track(kind, (!id.is_empty()).then_some(id.as_str())) {
                eprintln!("ferricone: {e}");
            }
        });
    });

    let s = session.clone();
    app.on_cycle_track(move |tab| {
        let Some(p) = &*s.player.borrow() else {
            return "".into();
        };
        let kind = track_kind(tab);
        let tracks = p.tracks(kind);
        // The list as the panel shows it: None, then the tracks.
        let choices: Vec<Option<&Track>> = std::iter::once(None)
            .chain(tracks.iter().map(Some))
            .collect();
        let current = tracks.iter().position(|t| t.selected).map_or(0, |i| i + 1);
        let next = choices[(current + 1) % choices.len()];
        trace!("cycle {kind:?} track to {:?}", next.map(|t| &t.id));
        if let Err(e) = p.select_track(kind, next.map(|t| t.id.as_str())) {
            eprintln!("ferricone: {e}");
        }
        next.map_or("None", |t| t.name.as_str()).into()
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_refresh_devices(move || {
        let (Some(app), Some(p)) = (ui.upgrade(), &*s.player.borrow()) else {
            return;
        };
        let (devices, current) = p.audio_devices();
        let items: Vec<TrackItem> = devices
            .into_iter()
            .map(|d| TrackItem {
                selected: current.as_deref() == Some(d.id.as_str()),
                id: d.id.into(),
                label: d.name.into(),
            })
            .collect();
        app.set_audio_devices(ModelRc::new(VecModel::from(items)));
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_select_device(move |id| {
        s.with_player(|p| {
            if let Err(e) = p.set_audio_device(&id) {
                eprintln!("ferricone: audio device: {e}");
            }
        });
        if let Some(app) = ui.upgrade() {
            app.invoke_refresh_devices();
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_preview(move |position| {
        if let Some(app) = ui.upgrade() {
            s.extras.hover(&app, position);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_preview_arrived(move || {
        if let Some(app) = ui.upgrade() {
            s.extras.frame_arrived(&app);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_want_waveform(move || {
        if let Some(app) = ui.upgrade() {
            s.extras.want_waveform(&app);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_waveform_arrived(move || {
        if let Some(app) = ui.upgrade() {
            s.extras.levels_arrived(&app);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_toggle_remaining(move || {
        s.remaining.set(!s.remaining.get());
        if let Some(app) = ui.upgrade() {
            app.invoke_tick();
        }
    });
}

fn track_kind(tab: TracksTab) -> TrackKind {
    match tab {
        TracksTab::Video => TrackKind::Video,
        TracksTab::Audio => TrackKind::Audio,
        TracksTab::Subtitles => TrackKind::Subtitle,
    }
}

/// "None" first, selected when no track is, then VLC's tracks.
fn track_items(tracks: Vec<Track>) -> ModelRc<TrackItem> {
    let none = TrackItem {
        id: "".into(),
        label: "None".into(),
        selected: !tracks.iter().any(|t| t.selected),
    };
    let items: Vec<TrackItem> = std::iter::once(none)
        .chain(tracks.into_iter().map(|t| TrackItem {
            id: t.id.into(),
            label: t.name.into(),
            selected: t.selected,
        }))
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn show_clock(app: &AppWindow, clock: Clock, remaining: bool) {
    app.set_position(clock.position as f32);
    app.set_time_text(hms(clock.time).into());
    let length = if remaining {
        format!("-{}", hms(clock.length.saturating_sub(clock.time)))
    } else {
        hms(clock.length)
    };
    app.set_length_text(length.into());
}

/// `00:02:10`, as in the design.
pub fn hms(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::hms;
    use std::time::Duration;

    #[test]
    fn formats_like_the_design() {
        assert_eq!(hms(Duration::from_secs(130)), "00:02:10");
        assert_eq!(hms(Duration::from_secs(25 * 60 + 40)), "00:25:40");
        assert_eq!(hms(Duration::from_secs(3 * 3600 + 5)), "03:00:05");
    }
}
