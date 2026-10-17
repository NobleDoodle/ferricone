//! Presets: what the settings sidebar edits. One default preset, and one per
//! folder (a show) once chosen in the sidebar's footer, which then applies to
//! every file opened from that folder. Kept in `presets.toml` in the user's
//! config folder (`$XDG_CONFIG_HOME/ferricone` on Linux).

use crate::{
    media::Source,
    state::{read, write},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

/// How the two channels play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StereoMode {
    /// As the file has it.
    #[default]
    Original,
    Stereo,
    Reverse,
    Left,
    Right,
    Dolby,
    Mono,
}

impl StereoMode {
    pub const ALL: [StereoMode; 7] = [
        StereoMode::Original,
        StereoMode::Stereo,
        StereoMode::Reverse,
        StereoMode::Left,
        StereoMode::Right,
        StereoMode::Dolby,
        StereoMode::Mono,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StereoMode::Original => "As Recorded",
            StereoMode::Stereo => "Stereo",
            StereoMode::Reverse => "Reverse Stereo",
            StereoMode::Left => "Left Channel",
            StereoMode::Right => "Right Channel",
            StereoMode::Dolby => "Dolby Surround",
            StereoMode::Mono => "Mono",
        }
    }
}

/// Mirroring the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Flip {
    #[default]
    None,
    Horizontal,
    Vertical,
}

/// How the picture is shown: the Video tab's Defaults and Crop card. The
/// window draws this itself, on the GPU; VLC's frame stays as decoded.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct View {
    pub flip: Flip,
    /// Degrees, clockwise.
    pub rotate: f32,
    /// 0 fits the window; otherwise video pixels per screen pixel.
    pub zoom: f32,
    /// "16:9" and the like; empty keeps the video's own.
    pub aspect: String,
    /// Crops to this shape ("16:9", …); empty crops nothing.
    pub crop: String,
    /// Pixels off the top, bottom, left and right.
    pub edges: [u32; 4],
}

/// Which tracks an item opens with: VLC's `audio-language` /
/// `sub-language` (codes or names, comma-separated) and track numbers
/// (1-based; 0 lets VLC choose).
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Tracks {
    pub audio_language: String,
    pub audio_track: u32,
    pub subtitle_language: String,
    pub subtitle_track: u32,
}

/// How subtitles look: VLC's text renderer options, taken while playing
/// (the font and outline color through patches/0004, the opacities through
/// 0005). A change shows from
/// the next line drawn.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SubtitleStyle {
    /// A font family; empty is VLC's default.
    pub font: String,
    /// 0xRRGGBB.
    pub color: u32,
    pub outline_color: u32,
    /// 0–255, of the text and of its outline.
    pub opacity: u32,
    pub outline_opacity: u32,
    /// 0 none, 2 thin, 4 normal, 6 thick.
    pub outline: u32,
    /// 0–255.
    pub background: u32,
    /// Percent of the normal size, 10–500.
    pub scale: u32,
    /// Pixels up from the bottom.
    pub offset: i32,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font: String::new(),
            color: 0xFF_FFFF,
            outline_color: 0x00_0000,
            opacity: 255,
            outline_opacity: 255,
            outline: 4,
            background: 0,
            scale: 100,
            offset: 0,
        }
    }
}

/// An audio or video filter: on or off, and its options by VLC name.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Filter {
    pub on: bool,
    pub values: BTreeMap<String, f32>,
}

/// The equalizer: off, one of VLC's presets, or "Custom" bands.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Equalizer {
    /// `None` is off.
    pub preset: Option<String>,
    /// dB, −20 to +20.
    pub preamp: f32,
    /// dB per band, −20 to +20.
    pub bands: Vec<f32>,
    pub two_pass: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Preset {
    /// 0.25–2, 1 is normal.
    pub speed: f32,
    /// Seconds, −10 to +10; positive plays the audio later.
    pub audio_delay: f64,
    /// Seconds, −10 to +10; positive shows the subtitles later.
    pub subtitle_delay: f64,
    pub stereo: StereoMode,
    pub equalizer: Equalizer,
    pub compressor: Filter,
    pub spatializer: Filter,
    pub widener: Filter,
    /// Semitones, −12 to +12; 0 is off.
    pub pitch: f32,
    pub view: View,
    pub tracks: Tracks,
    pub subtitles: SubtitleStyle,
    /// The seek bar shows the subtitle timeline.
    pub subtitle_timeline: bool,
    /// The Video tab's filters, by VLC module.
    pub video_filters: BTreeMap<String, Filter>,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            speed: 1.0,
            audio_delay: 0.0,
            subtitle_delay: 0.0,
            stereo: StereoMode::Original,
            equalizer: Equalizer::default(),
            compressor: Filter::default(),
            spatializer: Filter::default(),
            widener: Filter::default(),
            pitch: 0.0,
            view: View::default(),
            tracks: Tracks::default(),
            subtitles: SubtitleStyle::default(),
            subtitle_timeline: false,
            video_filters: BTreeMap::new(),
        }
    }
}

pub const MIN_SPEED: f32 = 0.25;
pub const MAX_SPEED: f32 = 2.0;
pub const MAX_DELAY: f64 = 10.0;
pub const MAX_EDGE: u32 = 1000;

/// "16:9" as 16/9; `None` for anything else.
pub fn ratio(text: &str) -> Option<f32> {
    let (w, h) = text.split_once(':')?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0.0 && h > 0.0).then_some(w / h)
}

impl Preset {
    /// Brings values from the file, or typed ones, into range.
    pub fn clamped(self) -> Self {
        Self {
            speed: self.speed.clamp(MIN_SPEED, MAX_SPEED),
            audio_delay: self.audio_delay.clamp(-MAX_DELAY, MAX_DELAY),
            subtitle_delay: self.subtitle_delay.clamp(-MAX_DELAY, MAX_DELAY),
            pitch: self.pitch.clamp(-12.0, 12.0),
            view: View {
                rotate: self.view.rotate.rem_euclid(360.0),
                zoom: self.view.zoom.clamp(0.0, 8.0),
                edges: self.view.edges.map(|e| e.min(MAX_EDGE)),
                ..self.view
            },
            subtitles: SubtitleStyle {
                font: self.subtitles.font.trim().to_string(),
                color: self.subtitles.color & 0xFF_FFFF,
                outline_color: self.subtitles.outline_color & 0xFF_FFFF,
                opacity: self.subtitles.opacity.min(255),
                outline_opacity: self.subtitles.outline_opacity.min(255),
                outline: self.subtitles.outline.min(6) / 2 * 2,
                background: self.subtitles.background.min(255),
                scale: self.subtitles.scale.clamp(10, 500),
                offset: self.subtitles.offset.clamp(-500, 1000),
            },
            ..self
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
struct PresetFile {
    #[serde(default)]
    default: Preset,
    /// By folder path.
    #[serde(default)]
    folders: HashMap<String, Preset>,
}

pub struct Presets {
    path: Option<PathBuf>,
    file: PresetFile,
}

impl Presets {
    pub fn load() -> Self {
        let path = directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
            .map(|dirs| dirs.config_dir().join("presets.toml"));
        let file = read(path.as_deref());
        Self { path, file }
    }

    /// The preset for `source`, and whether it is its folder's own.
    pub fn for_source(&self, source: Option<&Source>) -> (Preset, bool) {
        match source
            .and_then(folder)
            .and_then(|f| self.file.folders.get(&f))
        {
            Some(preset) => (preset.clone().clamped(), true),
            None => (self.file.default.clone().clamped(), false),
        }
    }

    /// Changes the preset `source` uses (its folder's, or the default).
    pub fn edit(&mut self, source: Option<&Source>, change: impl FnOnce(&mut Preset)) {
        let key = source
            .and_then(folder)
            .filter(|k| self.file.folders.contains_key(k));
        let preset = match &key {
            Some(k) => self.file.folders.get_mut(k).expect("checked above"),
            None => &mut self.file.default,
        };
        change(preset);
        *preset = preset.clone().clamped();
        self.save();
    }

    /// Gives `source`'s folder its own preset, starting from the one in use.
    pub fn make_folder(&mut self, source: &Source) -> Option<String> {
        let key = folder(source)?;
        let current = self.for_source(Some(source)).0;
        self.file.folders.entry(key.clone()).or_insert(current);
        self.save();
        Some(key)
    }

    /// Goes back to the default preset for `source`'s folder.
    pub fn forget_folder(&mut self, source: &Source) {
        if let Some(key) = folder(source)
            && self.file.folders.remove(&key).is_some()
        {
            self.save();
        }
    }

    fn save(&self) {
        write(self.path.as_deref(), &self.file);
    }
}

/// The folder a file plays from; streams have none.
pub fn folder(source: &Source) -> Option<String> {
    match source {
        Source::File(path) => {
            let path = path.canonicalize().unwrap_or_else(|_| path.clone());
            Some(path.parent()?.to_string_lossy().into_owned())
        }
        Source::Location(_) => None,
    }
}

/// "#RRGGBB" or "RRGGBB".
/// "#RRGGBB", or "#RRGGBBAA" with the opacity too.
pub fn parse_color_alpha(text: &str) -> Option<(u32, Option<u32>)> {
    let hex = text.trim().trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some((value, None)),
        8 => Some((value >> 8, Some(value & 0xFF))),
        _ => None,
    }
}

/// "1.25x" and "1.25" alike.
pub fn parse_speed(text: &str) -> Option<f32> {
    text.trim()
        .trim_end_matches(['x', 'X', '×'])
        .trim()
        .parse()
        .ok()
}

/// "-4.000 s", "−4", "+0.5s" and "250 ms" alike, in seconds.
pub fn parse_seconds(text: &str) -> Option<f64> {
    let text = text.trim().replace('−', "-");
    if let Some(ms) = text.strip_suffix("ms") {
        return ms.trim().parse::<f64>().ok().map(|v| v / 1000.0);
    }
    text.trim_end_matches('s').trim().parse().ok()
}

/// "-4.000 s", as the design writes delays.
pub fn seconds_text(value: f64) -> String {
    format!("{value:.3} s")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Presets {
        Presets {
            path: None,
            file: PresetFile::default(),
        }
    }

    #[test]
    fn folders_keep_their_own_preset() {
        let mut p = store();
        let dir = std::env::temp_dir();
        let ep1 = Source::File(dir.join("ep1.mkv"));
        let ep2 = Source::File(dir.join("ep2.mkv"));
        let elsewhere = Source::Location("http://h/x".into());

        p.edit(Some(&ep1), |p| p.subtitle_delay = -4.0);
        assert_eq!(p.for_source(Some(&elsewhere)).0.subtitle_delay, -4.0); // the default

        p.make_folder(&ep1);
        p.edit(Some(&ep1), |p| p.subtitle_delay = 2.5);
        let (show, own) = p.for_source(Some(&ep2));
        assert!(own);
        assert_eq!(show.subtitle_delay, 2.5); // sticks for the next episode
        assert_eq!(p.for_source(Some(&elsewhere)).0.subtitle_delay, -4.0);

        p.forget_folder(&ep2);
        assert_eq!(p.for_source(Some(&ep1)), (p.file.default.clone(), false));
    }

    #[test]
    fn reads_typed_values() {
        assert_eq!(parse_speed(" 1.25x "), Some(1.25));
        assert_eq!(parse_seconds("−4.000 s"), Some(-4.0));
        assert_eq!(parse_seconds("+0.5s"), Some(0.5));
        assert_eq!(parse_seconds("250 ms"), Some(0.25));
        assert_eq!(parse_seconds("soon"), None);
        assert_eq!(seconds_text(-4.0), "-4.000 s");
        assert_eq!(parse_color_alpha("#ffcc00"), Some((0xFFCC00, None)));
        assert_eq!(parse_color_alpha("#FFCC0080"), Some((0xFFCC00, Some(0x80))));
        assert_eq!(parse_color_alpha("fc0"), None);
        assert_eq!(parse_color_alpha("+fcc00"), None);
        let style = SubtitleStyle {
            outline: 5,
            scale: 900,
            background: 400,
            ..SubtitleStyle::default()
        };
        let style = Preset {
            subtitles: style,
            ..Preset::default()
        }
        .clamped()
        .subtitles;
        assert_eq!(
            (style.outline, style.scale, style.background),
            (4, 500, 255)
        );
        let wild = Preset {
            speed: 9.0,
            audio_delay: -30.0,
            ..Preset::default()
        }
        .clamped();
        assert_eq!((wild.speed, wild.audio_delay), (2.0, -10.0));
    }
}
