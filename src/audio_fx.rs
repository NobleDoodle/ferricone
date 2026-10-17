//! The Audio tab's filters: VLC's compressor, spatializer and stereo widener
//! (set live through `Player::set_filter` / `set_filter_option`, patches/0001),
//! its 10-band equalizer, and the pitch shifter. Ranges and defaults are
//! VLC's own (`modules/audio_filter`); the design's cards show the same
//! numbers.

use crate::{
    FilterRow, RowKind,
    presets::{Equalizer, Filter, Preset},
    vlc::{self, EqualizerPreset, Player},
};
use std::sync::OnceLock;

/// One slider of a filter card.
pub struct Param {
    pub label: &'static str,
    /// The VLC option it sets.
    pub option: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// The design shows some values scaled (the spatializer's 0–1 as 0–10).
    pub scale: f32,
    pub unit: &'static str,
    pub decimals: usize,
}

pub struct FilterDef {
    /// The preset field and the UI's name for it.
    pub id: &'static str,
    /// The VLC module.
    pub module: &'static str,
    pub params: &'static [Param],
}

const fn param(
    label: &'static str,
    option: &'static str,
    (min, max, default): (f32, f32, f32),
    scale: f32,
    unit: &'static str,
    decimals: usize,
) -> Param {
    Param {
        label,
        option,
        min,
        max,
        default,
        scale,
        unit,
        decimals,
    }
}

pub const COMPRESSOR: FilterDef = FilterDef {
    id: "compressor",
    module: "compressor",
    params: &[
        param(
            "RMS / Peak",
            "compressor-rms-peak",
            (0.0, 1.0, 0.2),
            1.0,
            "",
            1,
        ),
        param(
            "Attack",
            "compressor-attack",
            (1.5, 400.0, 25.0),
            1.0,
            " ms",
            1,
        ),
        param(
            "Release",
            "compressor-release",
            (2.0, 800.0, 100.0),
            1.0,
            " ms",
            1,
        ),
        param(
            "Threshold",
            "compressor-threshold",
            (-30.0, 0.0, -11.0),
            1.0,
            " dB",
            1,
        ),
        param(
            "Ratio",
            "compressor-ratio",
            (1.0, 20.0, 4.0),
            1.0,
            " : 1",
            1,
        ),
        param(
            "Knee Radius",
            "compressor-knee",
            (1.0, 10.0, 5.0),
            1.0,
            " dB",
            1,
        ),
        param(
            "Makeup Gain",
            "compressor-makeup-gain",
            (0.0, 24.0, 7.0),
            1.0,
            " dB",
            1,
        ),
    ],
};

pub const SPATIALIZER: FilterDef = FilterDef {
    id: "spatializer",
    module: "spatializer",
    params: &[
        param(
            "Size",
            "spatializer-roomsize",
            (0.0, 1.1, 0.85),
            10.0,
            "",
            1,
        ),
        param("Width", "spatializer-width", (0.0, 1.0, 1.0), 10.0, "", 1),
        param("Wet", "spatializer-wet", (0.0, 1.0, 0.4), 10.0, "", 1),
        param("Dry", "spatializer-dry", (0.0, 1.0, 0.5), 10.0, "", 1),
        param("Damp", "spatializer-damp", (0.0, 1.0, 0.5), 10.0, "", 1),
    ],
};

pub const WIDENER: FilterDef = FilterDef {
    id: "widener",
    module: "stereo_widen",
    params: &[
        param(
            "Delay Time",
            "stereowiden-delay",
            (1.0, 100.0, 20.0),
            1.0,
            " ms",
            1,
        ),
        param(
            "Feedback Gain",
            "stereowiden-feedback",
            (0.0, 0.9, 0.3),
            100.0,
            " %",
            0,
        ),
        param(
            "Crossfeed",
            "stereowiden-crossfeed",
            (0.0, 0.8, 0.3),
            100.0,
            " %",
            0,
        ),
        param(
            "Dry Mix",
            "stereowiden-dry-mix",
            (0.0, 1.0, 0.8),
            100.0,
            " %",
            0,
        ),
    ],
};

pub const FILTERS: [&FilterDef; 3] = [&COMPRESSOR, &SPATIALIZER, &WIDENER];

pub fn def(id: &str) -> Option<&'static FilterDef> {
    FILTERS.into_iter().find(|d| d.id == id)
}

pub fn filter<'a>(preset: &'a Preset, id: &str) -> &'a Filter {
    match id {
        "spatializer" => &preset.spatializer,
        "widener" => &preset.widener,
        _ => &preset.compressor,
    }
}

pub fn filter_mut<'a>(preset: &'a mut Preset, id: &str) -> &'a mut Filter {
    match id {
        "spatializer" => &mut preset.spatializer,
        "widener" => &mut preset.widener,
        _ => &mut preset.compressor,
    }
}

impl Param {
    pub fn value(&self, filter: &Filter) -> f32 {
        filter
            .values
            .get(self.option)
            .copied()
            .unwrap_or(self.default)
            .clamp(self.min, self.max)
    }

    pub fn at(&self, position: f32) -> f32 {
        let value = self.min + position.clamp(0.0, 1.0) * (self.max - self.min);
        // Steps of the shown precision.
        let step = self.scale * 10f32.powi(self.decimals as i32);
        (value * step).round() / step
    }

    pub fn row(&self, value: f32) -> FilterRow {
        FilterRow {
            kind: RowKind::Slider,
            label: self.label.into(),
            value: format!("{:.*}{}", self.decimals, value * self.scale, self.unit).into(),
            position: (value - self.min) / (self.max - self.min),
            ..Default::default()
        }
    }
}

pub fn rows(def: &FilterDef, filter: &Filter) -> Vec<FilterRow> {
    def.params.iter().map(|p| p.row(p.value(filter))).collect()
}

pub fn apply_filter(player: &Player, def: &FilterDef, filter: &Filter) {
    // Options first, so the filter starts with them.
    for p in def.params {
        if let Err(e) = player.set_filter_option(p.option, &p.value(filter).to_string()) {
            eprintln!("ferricone: {}: {e}", p.option);
        }
    }
    if let Err(e) = player.set_filter(def.module, filter.on) {
        eprintln!("ferricone: {}: {e}", def.module);
    }
}

// The equalizer ------------------------------------------------------------

/// Gains run −20 to +20 dB, as in VLC.
pub const GAIN: f32 = 20.0;
pub const CUSTOM: &str = "Custom";

struct EqualizerInfo {
    presets: Vec<EqualizerPreset>,
    bands: Vec<f32>,
}

fn info() -> &'static EqualizerInfo {
    static INFO: OnceLock<EqualizerInfo> = OnceLock::new();
    INFO.get_or_init(|| EqualizerInfo {
        presets: vlc::equalizer_presets(),
        bands: vlc::equalizer_bands(),
    })
}

/// The dropdown: None, VLC's presets, and Custom once bands were moved.
pub fn eq_options(eq: &Equalizer) -> (Vec<String>, usize) {
    let mut options = vec!["None".to_string()];
    options.extend(info().presets.iter().map(|p| p.name.clone()));
    if eq.preset.as_deref() == Some(CUSTOM) {
        options.push(CUSTOM.into());
    }
    let selected = eq
        .preset
        .as_ref()
        .and_then(|name| options.iter().position(|o| o == name))
        .unwrap_or(0);
    (options, selected)
}

/// Picks dropdown entry `index` (0 is None).
pub fn choose_eq(eq: &mut Equalizer, index: usize) {
    if index == 0 {
        eq.preset = None;
        return;
    }
    if let Some(preset) = info().presets.get(index - 1) {
        eq.preset = Some(preset.name.clone());
        eq.preamp = preset.preamp;
        eq.bands = preset.bands.clone();
    }
}

/// Sets row `row` (0 is the preamp, then the bands) to `gain` dB; the
/// equalizer becomes Custom.
pub fn set_gain(eq: &mut Equalizer, row: usize, gain: f32) {
    let gain = (gain.clamp(-GAIN, GAIN) * 10.0).round() / 10.0;
    let bands = info().bands.len();
    eq.bands.resize(bands, 0.0);
    match row {
        0 => eq.preamp = gain,
        b if b <= bands => eq.bands[b - 1] = gain,
        _ => return,
    }
    eq.preset = Some(CUSTOM.into());
}

pub fn eq_rows(eq: &Equalizer) -> Vec<FilterRow> {
    let row = |label: String, gain: f32| FilterRow {
        kind: RowKind::Field,
        label: label.into(),
        value: format!("{gain:.1} dB").into(),
        position: (gain + GAIN) / (2.0 * GAIN),
        centered: true,
        ..Default::default()
    };
    std::iter::once(row("Preamp".into(), eq.preamp))
        .chain(info().bands.iter().enumerate().map(|(i, &hz)| {
            let label = if hz >= 1000.0 {
                format!("{} kHz", hz / 1000.0)
            } else {
                format!("{hz} Hz")
            };
            row(label, eq.bands.get(i).copied().unwrap_or(0.0))
        }))
        .collect()
}

pub fn apply_equalizer(player: &Player, eq: &Equalizer) {
    if eq.preset.is_none() {
        player.set_equalizer(None);
        return;
    }
    let two_pass = if eq.two_pass { "1" } else { "0" };
    if let Err(e) = player.set_filter_option("equalizer-2pass", two_pass) {
        eprintln!("ferricone: equalizer-2pass: {e}");
    }
    player.set_equalizer(Some((eq.preamp, &eq.bands)));
}

// Pitch -------------------------------------------------------------------

pub fn apply_pitch(player: &Player, semitones: f32) {
    let on = semitones != 0.0;
    if on && let Err(e) = player.set_filter_option("pitch-shift", &semitones.to_string()) {
        eprintln!("ferricone: pitch-shift: {e}");
    }
    if let Err(e) = player.set_filter("scaletempo_pitch", on) {
        eprintln!("ferricone: scaletempo_pitch: {e}");
    }
}

pub fn pitch_text(semitones: f32) -> String {
    match semitones {
        0.0 => "Off".into(),
        s => format!("{s:+.1} semitones"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sliders_to_vlc_values() {
        let ratio = &COMPRESSOR.params[4];
        assert_eq!(ratio.at(0.0), 1.0);
        assert_eq!(ratio.at(1.0), 20.0);
        let filter = Filter::default();
        let row = ratio.row(ratio.value(&filter));
        assert_eq!(row.value.as_str(), "4.0 : 1");
        let size = &SPATIALIZER.params[0];
        assert_eq!(size.row(size.value(&filter)).value.as_str(), "8.5");
        let feedback = &WIDENER.params[1];
        assert_eq!(feedback.row(0.3).value.as_str(), "30 %");
        assert_eq!(feedback.at(0.5), 0.45);
        assert_eq!(pitch_text(-2.5), "-2.5 semitones");
    }

    #[test]
    fn custom_bands_replace_the_preset() {
        let mut eq = Equalizer::default();
        set_gain(&mut eq, 0, 25.0);
        assert_eq!(eq.preamp, 20.0);
        assert_eq!(eq.preset.as_deref(), Some(CUSTOM));
        set_gain(&mut eq, 3, -3.33);
        assert_eq!(eq.bands[2], -3.3);
    }
}
