//! The subtitle timeline on the seek bar (Controls `sub timeline`): when
//! each subtitle line shows, as blocks in a window of time around the
//! playhead. libVLC has no cue-list API, so the times come from a subtitle
//! file beside the video (SRT, WebVTT, ASS/SSA), or, for an embedded track,
//! from `ffmpeg` when it is installed. Reading runs on a thread of its own.

use crate::{
    AppWindow, CueBlock, Icons,
    controls::{Session, Shared},
    media::Source,
    presets,
    vlc::{Clock, Player, TrackKind},
};
use slint::{ComponentHandle, Global, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

/// A video file and its subtitle stream (among its subtitle streams).
type ReadFor = (PathBuf, Option<usize>);
/// Cues read on the thread, with what they were read for.
type Arrived = (ReadFor, Vec<Cue>);

/// The timeline's state: the cues for the loaded item and subtitle track,
/// the window's span, and a cue drag in progress.
#[derive(Default)]
pub struct Timeline {
    cues: RefCell<Vec<Cue>>,
    /// What the cues were read for: the file and its subtitle stream.
    read_for: RefCell<Option<ReadFor>>,
    arrived: Arc<Mutex<Option<Arrived>>>,
    span: Cell<usize>,
    window_start: Cell<f64>,
    /// The subtitle delay when the drag began.
    drag_base: Cell<Option<f64>>,
    /// Bumped when new cues arrive, so the whole-track blocks are redrawn.
    generation: Cell<u64>,
    /// What the whole-track blocks were last drawn for: the cues, the delay
    /// (ms), the length (ms), and Adjust being off.
    drawn: Cell<Option<(u64, i64, u64)>>,
}

impl Timeline {
    fn span(&self) -> f64 {
        SPANS[self.span.get().min(SPANS.len() - 1)]
    }
}

fn on(s: &Session) -> bool {
    s.presets
        .borrow()
        .for_source(s.loaded.borrow().as_ref())
        .0
        .subtitle_timeline
}

/// Reads the cues for the loaded item's selected subtitle track, unless
/// they are read already. Call when the item, its tracks or the toggle change.
pub fn reload(app: &AppWindow, s: &Session) {
    if !on(s) {
        return;
    }
    let Some(Source::File(video)) = s.loaded.borrow().clone() else {
        s.timeline.cues.borrow_mut().clear();
        return;
    };
    // The selected subtitle track's place among the subtitle tracks.
    let embedded = s.player.borrow().as_ref().and_then(|p| {
        p.tracks(TrackKind::Subtitle)
            .iter()
            .position(|t| t.selected)
    });
    let key = (video.clone(), embedded);
    if s.timeline.read_for.borrow().as_ref() == Some(&key) {
        return;
    }
    *s.timeline.read_for.borrow_mut() = Some(key.clone());
    s.timeline.cues.borrow_mut().clear();
    let (ui, slot) = (app.as_weak(), s.timeline.arrived.clone());
    std::thread::spawn(move || {
        let cues = read(&video, embedded);
        *slot.lock().unwrap() = Some((key, cues));
        let _ = ui.upgrade_in_event_loop(|app| app.invoke_cues_arrived());
    });
}

/// Each clock tick while the timeline shows: the cues across the whole
/// track, or, while adjusting their timing, the window around now.
pub fn tick(app: &AppWindow, s: &Session, clock: &Clock) {
    if !on(s) {
        return;
    }
    let delay = s
        .presets
        .borrow()
        .for_source(s.loaded.borrow().as_ref())
        .0
        .subtitle_delay;
    if !app.get_timeline_adjust() {
        let length = clock.length.as_secs_f64();
        let key = (
            s.timeline.generation.get(),
            (delay * 1000.0).round() as i64,
            clock.length.as_millis() as u64,
        );
        if s.timeline.drawn.get() != Some(key) {
            s.timeline.drawn.set(Some(key));
            let blocks = whole(&s.timeline.cues.borrow(), length, delay);
            app.set_cues(ModelRc::new(VecModel::from(blocks)));
        }
        return;
    }
    s.timeline.drawn.set(None);
    let span = s.timeline.span();
    let now = clock.time.as_secs_f64();
    let (start, blocks) = window(&s.timeline.cues.borrow(), now, span, delay);
    s.timeline.window_start.set(start);
    app.set_cues(ModelRc::new(VecModel::from(blocks)));
    app.set_time_text(window_text(start).into());
    app.set_length_text(window_text(start + span).into());
}

pub fn connect(app: &AppWindow, session: &Shared) {
    session.timeline.span.set(DEFAULT_SPAN);

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_cues_arrived(move || {
        let Some((key, cues)) = s.timeline.arrived.lock().unwrap().take() else {
            return;
        };
        if s.timeline.read_for.borrow().as_ref() == Some(&key) {
            *s.timeline.cues.borrow_mut() = cues;
            s.timeline.generation.set(s.timeline.generation.get() + 1);
            if let Some(app) = ui.upgrade() {
                app.invoke_tick();
            }
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_timeline_adjust(move |on| {
        let Some(app) = ui.upgrade() else { return };
        app.set_timeline_adjust(on);
        s.timeline.drawn.set(None);
        app.invoke_tick();
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_timeline(move |on| {
        let Some(app) = ui.upgrade() else { return };
        s.presets
            .borrow_mut()
            .edit(s.loaded.borrow().as_ref(), |p| p.subtitle_timeline = on);
        app.set_timeline(on);
        if !on {
            app.set_timeline_adjust(false);
        }
        s.timeline.drawn.set(None);
        reload(&app, &s);
        app.invoke_tick();
    });

    let s = session.clone();
    app.on_seek_window(move |at| {
        let time = s.timeline.window_start.get() + f64::from(at) * s.timeline.span();
        s.with_player(|p| p.set_time(Duration::from_secs_f64(time.max(0.0))));
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_timeline_zoom(move |step| {
        let span = s.timeline.span.get() as i32 - step;
        s.timeline
            .span
            .set(span.clamp(0, SPANS.len() as i32 - 1) as usize);
        if let Some(app) = ui.upgrade() {
            app.invoke_tick();
        }
    });

    // Dragging a line shifts the subtitle delay; the preset keeps it.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_cue_dragged(move |by, done| {
        let Some(app) = ui.upgrade() else { return };
        let source = s.loaded.borrow().clone();
        let base = match s.timeline.drag_base.get() {
            Some(base) => base,
            None => {
                let base = s
                    .presets
                    .borrow()
                    .for_source(source.as_ref())
                    .0
                    .subtitle_delay;
                s.timeline.drag_base.set(Some(base));
                base
            }
        };
        let delay = delay_after_drag(base, by, s.timeline.span());
        s.presets
            .borrow_mut()
            .edit(source.as_ref(), |p| p.subtitle_delay = delay);
        s.with_player(|p: &Player| p.set_subtitle_delay(delay));
        crate::settings::refresh(&app, &s);
        app.invoke_tick();
        if done {
            s.timeline.drag_base.set(None);
            let text = format!("Subtitle delay {}", presets::seconds_text(delay));
            app.invoke_show_osd(Icons::get(&app).get_subtitles(), text.into(), "".into());
        }
    });
}

/// When one subtitle line shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
}

/// The window's span in seconds: zoomed in, normal, out.
pub const SPANS: [f64; 6] = [10.0, 20.0, 50.0, 120.0, 300.0, 600.0];
pub const DEFAULT_SPAN: usize = 2;

const SIDECARS: [&str; 4] = ["srt", "vtt", "ass", "ssa"];

/// A subtitle file beside `video`: the same name with a subtitle
/// extension, or with a language between ("Show.en.srt").
pub fn sidecar(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent()?;
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            ext.is_some_and(|e| SIDECARS.contains(&e.as_str()))
                && (name == stem
                    || name
                        .strip_prefix(&stem)
                        .is_some_and(|rest| rest.starts_with('.')))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The cues of `video`'s subtitles: a file beside it, else embedded
/// subtitle stream `embedded` (0-based among its subtitle streams) through
/// `ffmpeg`. Blocking; call on a thread.
pub fn read(video: &Path, embedded: Option<usize>) -> Vec<Cue> {
    if let Some(file) = sidecar(video)
        && let Ok(text) = std::fs::read_to_string(&file)
    {
        let cues = parse(&text);
        if !cues.is_empty() {
            return cues;
        }
    }
    let Some(index) = embedded else {
        return Vec::new();
    };
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(video)
        .args(["-map", &format!("0:s:{index}"), "-f", "srt", "-"])
        .output();
    match output {
        Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
        Ok(_) => Vec::new(),
        Err(_) => Vec::new(), // no ffmpeg: no timeline for embedded tracks
    }
}

/// SRT, WebVTT or ASS/SSA, told apart by their contents.
pub fn parse(text: &str) -> Vec<Cue> {
    let mut cues = if text.contains("[Events]") {
        parse_ass(text)
    } else {
        parse_arrows(text)
    };
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// SRT and WebVTT: "00:01:02,500 --> 00:01:04,000" (or "." and no hours).
fn parse_arrows(text: &str) -> Vec<Cue> {
    text.lines()
        .filter_map(|line| {
            let (start, rest) = line.split_once("-->")?;
            let end = rest.split_whitespace().next()?;
            Some(Cue {
                start: clock(start.trim())?,
                end: clock(end.trim())?,
            })
        })
        .filter(|c| c.end > c.start)
        .collect()
}

/// ASS/SSA: "Dialogue: 0,0:01:02.50,0:01:04.00,…" (start and end are the
/// second and third fields).
fn parse_ass(text: &str) -> Vec<Cue> {
    text.lines()
        .filter_map(|line| {
            let fields = line.trim().strip_prefix("Dialogue:")?;
            let mut f = fields.split(',');
            let _layer = f.next()?;
            Some(Cue {
                start: clock(f.next()?.trim())?,
                end: clock(f.next()?.trim())?,
            })
        })
        .filter(|c| c.end > c.start)
        .collect()
}

/// "h:mm:ss,fff", "hh:mm:ss.ff" or "mm:ss.fff", in seconds.
fn clock(text: &str) -> Option<f64> {
    let text = text.replace(',', ".");
    let parts: Vec<&str> = text.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + s)
}

/// The window around `now` (seconds): its start, and the cues in it as
/// blocks placed 0–1 across it, shifted by the subtitle `delay`.
pub fn window(cues: &[Cue], now: f64, span: f64, delay: f64) -> (f64, Vec<CueBlock>) {
    let start = now - span / 2.0;
    let end = start + span;
    let blocks = cues
        .iter()
        .map(|c| (c.start + delay, c.end + delay))
        .filter(|&(s, e)| e > start && s < end)
        .map(|(s, e)| CueBlock {
            start: ((s - start) / span) as f32,
            end: ((e - start) / span) as f32,
        })
        .collect();
    (start, blocks)
}

/// The cues as blocks placed 0–1 along a track `length` seconds long,
/// shifted by the subtitle `delay`; none while the length is unknown.
pub fn whole(cues: &[Cue], length: f64, delay: f64) -> Vec<CueBlock> {
    if length <= 0.0 {
        return Vec::new();
    }
    cues.iter()
        .map(|c| (c.start + delay, c.end + delay))
        .filter(|&(s, e)| e > 0.0 && s < length)
        .map(|(s, e)| CueBlock {
            start: (s / length) as f32,
            end: (e / length) as f32,
        })
        .collect()
}

/// "00:01:50*": a window end, starred as in the design.
pub fn window_text(seconds: f64) -> String {
    format!(
        "{}*",
        crate::controls::hms(Duration::from_secs_f64(seconds.max(0.0)))
    )
}

/// The subtitle delay after dragging a cue `fraction` of the window.
pub fn delay_after_drag(delay: f64, fraction: f32, span: f64) -> f64 {
    let moved = f64::from(fraction) * span;
    ((delay + moved) * 1000.0)
        .round()
        .clamp(-presets::MAX_DELAY * 1000.0, presets::MAX_DELAY * 1000.0)
        / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_cues_along_the_whole_track() {
        let cues = [
            Cue { start: 10.0, end: 20.0 },
            Cue { start: 95.0, end: 105.0 },
        ];
        let blocks = whole(&cues, 100.0, 0.0);
        assert_eq!(blocks.len(), 2);
        assert_eq!((blocks[0].start, blocks[0].end), (0.1, 0.2));
        let later = whole(&cues, 100.0, 10.0);
        assert_eq!(later.len(), 1, "a cue pushed past the end is dropped");
        assert!(whole(&cues, 0.0, 0.0).is_empty());
    }

    #[test]
    fn reads_srt_vtt_and_ass() {
        let srt = "1\n00:00:01,500 --> 00:00:03,000\nHello\n\n2\n00:01:02,000 --> 00:01:04,250 X:1\nWorld\n";
        assert_eq!(
            parse(srt),
            [
                Cue {
                    start: 1.5,
                    end: 3.0
                },
                Cue {
                    start: 62.0,
                    end: 64.25
                }
            ]
        );
        let vtt = "WEBVTT\n\n00:05.000 --> 00:07.500 line:90%\nHi\n";
        assert_eq!(
            parse(vtt),
            [Cue {
                start: 5.0,
                end: 7.5
            }]
        );
        let ass = "[Script Info]\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:10.00,0:00:12.50,Default,,0,0,0,,Hi, there\n";
        assert_eq!(
            parse(ass),
            [Cue {
                start: 10.0,
                end: 12.5
            }]
        );
    }

    #[test]
    fn places_cues_in_the_window() {
        let cues = [
            Cue {
                start: 10.0,
                end: 15.0,
            },
            Cue {
                start: 100.0,
                end: 101.0,
            },
        ];
        let (start, blocks) = window(&cues, 20.0, 20.0, 0.0);
        assert_eq!(start, 10.0);
        assert_eq!(blocks.len(), 1);
        assert_eq!((blocks[0].start, blocks[0].end), (0.0, 0.25));
        // A 2 s delay moves it right by a tenth.
        let (_, shifted) = window(&cues, 20.0, 20.0, 2.0);
        assert_eq!((shifted[0].start, shifted[0].end), (0.1, 0.35));
        assert_eq!(window_text(110.0), "00:01:50*");
        assert_eq!(delay_after_drag(0.5, 0.01, 50.0), 1.0);
    }

    #[test]
    fn finds_subtitles_beside_the_video() {
        let dir = std::env::temp_dir().join(format!("ferricone-subs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Show S01E01.mkv");
        std::fs::write(&video, b"").unwrap();
        std::fs::write(dir.join("Show S01E01.en.srt"), b"").unwrap();
        std::fs::write(dir.join("Show S01E02.srt"), b"").unwrap();
        assert_eq!(sidecar(&video), Some(dir.join("Show S01E01.en.srt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
