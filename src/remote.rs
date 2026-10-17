//! The desktop's media controls: media keys, the bar's now-playing widget and
//! `playerctl`. On Linux this is an MPRIS server (remote/mpris.rs); other
//! systems get theirs with P7, until then these functions do nothing there.
//!
//! The player and the window report what is playing here, from any thread;
//! the server reads it when asked and announces the changes. Commands from
//! the desktop arrive through the closure given to `start`.

#[cfg(target_os = "linux")]
mod mpris;

use crate::vlc::{ClockSource, Event, State};
use std::{
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// What the desktop asks the player to do.
#[derive(Clone, Debug)]
pub enum Command {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Jump by this many seconds, back if negative.
    Seek(f64),
    /// Go to this point, 0–1 of the item.
    SetPosition(f64),
    /// 1.0 is the source level.
    SetVolume(f64),
    Open(String),
    Raise,
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

/// The playing item as the desktop shows it.
#[derive(Clone, Debug, Default)]
pub struct Track {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub url: Option<String>,
}

/// What changed since the server last announced; bits of `Report::changed`.
const STATUS: u8 = 1;
const METADATA: u8 = 2;
const VOLUME: u8 = 4;
const CONTROLS: u8 = 8;

#[derive(Default)]
struct Report {
    status: Status,
    /// Counts up per item, for MPRIS track ids; 0 is no track.
    track_id: u64,
    track: Track,
    clock: Option<ClockSource>,
    volume: f64,
    can_next: bool,
    can_previous: bool,
    can_seek: bool,
    changed: u8,
    seeked: Option<Duration>,
}

impl Report {
    /// Microseconds into the item, as MPRIS counts.
    fn position_us(&self) -> i64 {
        let clock = self.clock.as_ref().and_then(ClockSource::read);
        clock.map_or(0, |c| c.time.as_micros() as i64)
    }

    fn length_us(&self) -> Option<i64> {
        let clock = self.clock.as_ref().and_then(ClockSource::read)?;
        (!clock.length.is_zero()).then_some(clock.length.as_micros() as i64)
    }
}

struct Remote {
    report: Mutex<Report>,
    #[cfg(target_os = "linux")]
    waker: mpris::Waker,
}

static REMOTE: OnceLock<Remote> = OnceLock::new();

/// Offers the player to the desktop. `commands` runs on the server's thread.
pub fn start(commands: impl Fn(Command) + Send + 'static) {
    #[cfg(target_os = "linux")]
    match mpris::Waker::new() {
        Ok((waker, woken)) => {
            let remote = Remote {
                report: Mutex::new(Report {
                    volume: 1.0,
                    ..Default::default()
                }),
                waker,
            };
            if REMOTE.set(remote).is_ok() {
                mpris::serve(REMOTE.get().unwrap(), woken, Box::new(commands));
            }
        }
        Err(e) => eprintln!("ferricone: media controls: {e}"),
    }
    #[cfg(not(target_os = "linux"))]
    let _ = commands;
}

/// Changes the report and tells the server what changed.
fn update(f: impl FnOnce(&mut Report) -> u8) {
    let Some(remote) = REMOTE.get() else { return };
    let changed = {
        let mut report = remote.report.lock().unwrap();
        let changed = f(&mut report);
        report.changed |= changed;
        changed != 0 || report.seeked.is_some()
    };
    #[cfg(target_os = "linux")]
    if changed {
        remote.waker.wake();
    }
    #[cfg(not(target_os = "linux"))]
    let _ = changed;
}

/// Follows the player's own events; called on VLC's threads.
pub fn observe(event: &Event) {
    match *event {
        Event::State(state) => {
            let status = match state {
                State::Opening | State::Playing => Status::Playing,
                State::Paused => Status::Paused,
                _ => Status::Stopped,
            };
            update(|r| {
                let changed = r.status != status;
                r.status = status;
                if changed { STATUS } else { 0 }
            });
        }
        Event::LengthChanged => update(|_| METADATA),
        Event::Seekable(seekable) => update(|r| {
            r.can_seek = seekable;
            CONTROLS
        }),
        Event::Seeked(time) => update(|r| {
            r.seeked = Some(time);
            0
        }),
        _ => {}
    }
}

pub fn set_clock(clock: ClockSource) {
    update(|r| {
        r.clock = Some(clock);
        0
    });
}

/// A new item started.
pub fn new_track(track: Track, can_previous: bool, can_next: bool) {
    update(|r| {
        r.track_id += 1;
        r.track = track;
        (r.can_previous, r.can_next) = (can_previous, can_next);
        METADATA | CONTROLS
    });
}

/// The queue changed what Previous and Next can do.
pub fn set_queue(can_previous: bool, can_next: bool) {
    update(|r| {
        let changed = (r.can_previous, r.can_next) != (can_previous, can_next);
        (r.can_previous, r.can_next) = (can_previous, can_next);
        if changed { CONTROLS } else { 0 }
    });
}

/// The same item, with the tags VLC has read since.
pub fn update_track(track: Track) {
    update(|r| {
        r.track = track;
        METADATA
    });
}

pub fn set_volume(volume: f64) {
    update(|r| {
        let changed = (r.volume - volume).abs() > f64::EPSILON;
        r.volume = volume;
        if changed { VOLUME } else { 0 }
    });
}
