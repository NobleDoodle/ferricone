//! The queue panel: the play order with its lengths, the item's bookmarks
//! and its chapters. Lengths are read off the UI thread by libVLC's parser
//! (`Prober`) and arrive through the window's `durations-arrived`.

use crate::{
    AppWindow, ChapterRow, MarkRow, QueueRow, RepeatMode,
    controls::{Session, Shared, hms, play_current},
    diag::trace,
    media::{Repeat, Source},
    remote,
    vlc::{Player, Prober, Vlc},
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Arrived = Arc<Mutex<Vec<(PathBuf, Option<Duration>)>>>;

/// The lengths read so far, and the reader.
#[derive(Default)]
pub struct Lengths {
    known: std::collections::HashMap<PathBuf, Option<Duration>>,
    prober: Option<Prober>,
    arrived: Arrived,
}

/// Starts the length reader; call once the window exists.
pub fn start(app: &AppWindow, s: &Session, vlc: &Vlc) {
    let (ui, arrived) = (app.as_weak(), s.lengths.borrow().arrived.clone());
    match vlc.prober(move |path, length| {
        arrived.lock().unwrap().push((path, length));
        let _ = ui.upgrade_in_event_loop(|app| app.invoke_durations_arrived());
    }) {
        Ok(prober) => s.lengths.borrow_mut().prober = Some(prober),
        Err(e) => eprintln!("ferricone: queue lengths: {e}"),
    }
}

/// Rebuilds the queue rows, the summary and what Previous/Next can do, and
/// asks for lengths not read yet.
pub fn refresh(app: &AppWindow, s: &Session) {
    let queue = s.queue.borrow();
    let mut lengths = s.lengths.borrow_mut();
    let current = queue.current_index();
    let mut total = Duration::ZERO;
    let rows: Vec<QueueRow> = queue
        .items()
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let length = match source {
                Source::File(path) => {
                    if !lengths.known.contains_key(path) {
                        // Asked once; `None` until (and unless) it is known.
                        lengths.known.insert(path.clone(), None);
                        if let Some(prober) = &lengths.prober
                            && let Err(e) = prober.probe(path)
                        {
                            trace!("length of {}: {e}", path.display());
                        }
                    }
                    lengths.known[path]
                }
                Source::Location(_) => None,
            };
            total += length.unwrap_or_default();
            QueueRow {
                title: source.title().into(),
                duration: length.map(clock_text).unwrap_or_default().into(),
                playing: Some(i) == current,
            }
        })
        .collect();
    let count = rows.len();
    let items = if count == 1 {
        "1 item".to_string()
    } else {
        format!("{count} items")
    };
    let summary = if total.is_zero() {
        items
    } else {
        format!("{items} - {}", span_text(total))
    };
    app.set_queue_items(ModelRc::new(VecModel::from(rows)));
    app.set_queue_summary(summary.into());
    app.set_repeat(match queue.repeat {
        Repeat::Off => RepeatMode::Off,
        Repeat::All => RepeatMode::All,
        Repeat::One => RepeatMode::One,
    });
    let has_next = queue.has_next() || s.folder_next.borrow().is_some();
    app.set_has_previous(queue.has_previous());
    app.set_has_next(has_next);
    remote::set_queue(queue.has_previous(), has_next);
}

/// The loaded item's bookmarks.
pub fn refresh_marks(app: &AppWindow, s: &Session) {
    let rows: Vec<MarkRow> = match &*s.loaded.borrow() {
        Some(source) => s
            .bookmarks
            .borrow()
            .of(source)
            .iter()
            .map(|b| MarkRow {
                name: b.name.clone().into(),
                time: precise_text(Duration::from_secs_f64(b.time)).into(),
            })
            .collect(),
        None => Vec::new(),
    };
    app.set_marks(ModelRc::new(VecModel::from(rows)));
}

pub fn connect(app: &AppWindow, session: &Shared) {
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_durations_arrived(move || {
        let arrived: Vec<_> = std::mem::take(&mut *s.lengths.borrow().arrived.lock().unwrap());
        s.lengths.borrow_mut().known.extend(arrived);
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_media_ended(move || {
        let Some(app) = ui.upgrade() else { return };
        // The sleep timer said "end of this item".
        if crate::settings::sleeps_at_end(&app) {
            s.remember();
            return;
        }
        let prefs = crate::prefs::get();
        // Stop after each item (Settings › Playback).
        if prefs.stop_after_each {
            s.remember();
            return;
        }
        let advanced = s.queue.borrow_mut().advance().is_some();
        if advanced {
            play_current(&app, &s);
            return;
        }
        // The last item ended: a watched item is forgotten here.
        s.remember();
        let next = s.folder_next.borrow().clone();
        match prefs.on_finish {
            // Offer the next file in its folder, which plays after a countdown.
            crate::prefs::Finish::OfferNext => {
                if let Some(next) = next {
                    app.set_up_next_title(Source::File(next).title().into());
                    app.set_up_next_left(5);
                    app.set_up_next_shown(true);
                }
            }
            crate::prefs::Finish::PlayNext => {
                crate::controls::play_folder_next(&app, &s);
            }
            crate::prefs::Finish::Stop => {}
            crate::prefs::Finish::Quit => app.invoke_quit(),
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_play_item(move |index| {
        let moved = s.queue.borrow_mut().play_at(index as usize).is_some();
        if let (true, Some(app)) = (moved, ui.upgrade()) {
            play_current(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_move_item(move |from, to| {
        s.queue.borrow_mut().move_item(from as usize, to as usize);
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_remove_item(move |index| {
        let playing = s.queue.borrow_mut().remove(index as usize);
        let Some(app) = ui.upgrade() else { return };
        let empty = s.queue.borrow().items().is_empty();
        if empty {
            s.remember();
            s.with_player(Player::stop);
            *s.loaded.borrow_mut() = None;
            app.set_has_media(false);
            app.set_media_title("".into());
            app.set_folder_name("".into());
        } else if playing {
            // The playing item went: play what took its place.
            play_current(&app, &s);
            return;
        }
        refresh(&app, &s);
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_sort_items(move || {
        s.queue.borrow_mut().sort_by_title();
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_cycle_repeat(move || {
        let repeat = {
            let mut queue = s.queue.borrow_mut();
            queue.repeat = match queue.repeat {
                Repeat::Off => Repeat::All,
                Repeat::All => Repeat::One,
                Repeat::One => Repeat::Off,
            };
            queue.repeat
        };
        trace!("repeat {repeat:?}");
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_shuffle_queue(move || {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64 | 1);
        s.queue.borrow_mut().shuffle_rest(seed);
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });

    let s = session.clone();
    app.on_go_mark(move |index| {
        let Some(source) = s.loaded.borrow().clone() else {
            return;
        };
        let time = s
            .bookmarks
            .borrow()
            .of(&source)
            .get(index as usize)
            .map(|b| b.time);
        if let Some(time) = time {
            s.with_player(|p| p.set_time(Duration::from_secs_f64(time)));
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_add_mark(move || {
        let Some(source) = s.loaded.borrow().clone() else {
            return "".into();
        };
        let Some(clock) = s.player.borrow().as_ref().and_then(Player::clock) else {
            return "".into();
        };
        let index = s.bookmarks.borrow_mut().add(&source, clock.time);
        let name = s.bookmarks.borrow().of(&source)[index].name.clone();
        if let Some(app) = ui.upgrade() {
            refresh_marks(&app, &s);
        }
        name.into()
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_remove_mark(move |index| {
        let Some(source) = s.loaded.borrow().clone() else {
            return;
        };
        s.bookmarks.borrow_mut().remove(&source, index as usize);
        if let Some(app) = ui.upgrade() {
            refresh_marks(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_refresh_chapters(move || {
        let (Some(app), Some(p)) = (ui.upgrade(), &*s.player.borrow()) else {
            return;
        };
        let (chapters, playing) = p.chapters();
        let rows: Vec<ChapterRow> = chapters
            .into_iter()
            .enumerate()
            .map(|(i, c)| ChapterRow {
                title: c
                    .name
                    .unwrap_or_else(|| format!("Chapter {}", i + 1))
                    .into(),
                start: hms(c.start).into(),
                playing: Some(i) == playing,
            })
            .collect();
        app.set_chapters(ModelRc::new(VecModel::from(rows)));
    });

    let s = session.clone();
    app.on_go_chapter(move |index| {
        s.with_player(|p| p.set_chapter(index as usize));
    });
}

/// A row's length: "23:06", or "1:02:03" past an hour.
fn clock_text(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The queue's total: "2h 3m", "25m" or "40s".
fn span_text(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60) {
        (0, 0) => format!("{s}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// A bookmark's time, to the millisecond: "00:00:42:023".
fn precise_text(d: Duration) -> String {
    format!("{}:{:03}", hms(d), d.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_the_design() {
        assert_eq!(clock_text(Duration::from_secs(23 * 60 + 6)), "23:06");
        assert_eq!(clock_text(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(
            span_text(Duration::from_secs(2 * 3600 + 3 * 60 + 20)),
            "2h 3m"
        );
        assert_eq!(span_text(Duration::from_secs(25 * 60)), "25m");
        assert_eq!(span_text(Duration::from_secs(40)), "40s");
        assert_eq!(precise_text(Duration::from_millis(42_023)), "00:00:42:023");
    }
}
