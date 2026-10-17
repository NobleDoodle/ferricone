//! The Video tab. Its Defaults (flip, rotate, zoom, aspect ratio, crop) and
//! the Crop card change how the window draws the frame, on the GPU, so they
//! cost nothing. The Essential and Colors cards drive VLC's video filters
//! (adjust, sharpen, gradfun, grain, extract, colorthres, invert, posterize,
//! gradient, sepia) live, through patches/0001; those run on the processor.
//! Ranges and defaults are VLC's own (`modules/video_filter`).

use crate::{
    FilterRow, RowKind,
    audio_fx::Param,
    presets::{Filter, MAX_EDGE, Preset, View},
    vlc::Player,
};
use slint::Color;

/// A card row: a slider, a module's switch, an option's switch, or a color.
pub enum Row {
    /// A slider; the module runs while its value is off `neutral`.
    Slider {
        module: &'static str,
        param: Param,
        neutral: f32,
    },
    Toggle {
        module: &'static str,
        label: &'static str,
    },
    /// An on/off option of a module.
    Option {
        module: &'static str,
        option: &'static str,
        label: &'static str,
        default: bool,
    },
    Color {
        module: &'static str,
        option: &'static str,
        label: &'static str,
        default: u32,
    },
}

const fn slider(
    module: &'static str,
    label: &'static str,
    option: &'static str,
    (min, max, default): (f32, f32, f32),
    neutral: f32,
    unit: &'static str,
    decimals: usize,
) -> Row {
    Row::Slider {
        module,
        param: Param {
            label,
            option,
            min,
            max,
            default,
            scale: 1.0,
            unit,
            decimals,
        },
        neutral,
    }
}

/// Hue to gamma are VLC's `adjust` (on while any differs from its default);
/// the rest start at 0, which turns them off.
pub const ESSENTIAL: &[Row] = &[
    slider("adjust", "Hue", "hue", (-180.0, 180.0, 0.0), 0.0, "°", 0),
    slider(
        "adjust",
        "Brightness",
        "brightness",
        (0.0, 2.0, 1.0),
        1.0,
        "",
        2,
    ),
    slider(
        "adjust",
        "Contrast",
        "contrast",
        (0.0, 2.0, 1.0),
        1.0,
        "",
        2,
    ),
    slider(
        "adjust",
        "Saturation",
        "saturation",
        (0.0, 3.0, 1.0),
        1.0,
        "",
        2,
    ),
    // VLC allows up to 10; past 3 the picture is mostly white.
    slider("adjust", "Gamma", "gamma", (0.01, 3.0, 1.0), 1.0, "", 2),
    slider(
        "sharpen",
        "Sharpen Sigma",
        "sharpen-sigma",
        (0.0, 2.0, 0.0),
        0.0,
        "",
        2,
    ),
    // gradfun takes 4–32; under 4 is off.
    slider(
        "gradfun",
        "Banding Removal Radius",
        "gradfun-radius",
        (0.0, 32.0, 0.0),
        0.0,
        "",
        0,
    ),
    slider(
        "grain",
        "Film Grain Variance",
        "grain-variance",
        (0.0, 10.0, 0.0),
        0.0,
        "",
        1,
    ),
];

pub const COLORS: &[Row] = &[
    Row::Toggle {
        module: "extract",
        label: "Color Extraction",
    },
    Row::Color {
        module: "extract",
        option: "extract-component",
        label: "Extract",
        default: 0xFF0000,
    },
    Row::Toggle {
        module: "colorthres",
        label: "Color Threshold",
    },
    Row::Color {
        module: "colorthres",
        option: "colorthres-color",
        label: "Keep",
        default: 0xFF0000,
    },
    slider(
        "colorthres",
        "Saturation",
        "colorthres-saturationthres",
        (0.0, 255.0, 20.0),
        f32::NAN,
        "",
        0,
    ),
    slider(
        "colorthres",
        "Similarity",
        "colorthres-similaritythres",
        (0.0, 255.0, 15.0),
        f32::NAN,
        "",
        0,
    ),
    Row::Toggle {
        module: "invert",
        label: "Negate Colors",
    },
    Row::Toggle {
        module: "posterize",
        label: "Posterize",
    },
    slider(
        "posterize",
        "Levels",
        "posterize-level",
        (2.0, 32.0, 6.0),
        f32::NAN,
        "",
        0,
    ),
    Row::Toggle {
        module: "gradient",
        label: "Gradient",
    },
    Row::Option {
        module: "gradient",
        option: "gradient-color",
        label: "Color",
        default: false,
    },
    Row::Option {
        module: "gradient",
        option: "gradient-cartoon",
        label: "Cartoon",
        default: true,
    },
    slider(
        "sepia",
        "Sepia Intensity",
        "sepia-intensity",
        (0.0, 255.0, 0.0),
        0.0,
        "",
        0,
    ),
];

/// The modules the cards drive, each once.
pub const MODULES: &[&str] = &[
    "adjust",
    "sharpen",
    "gradfun",
    "grain",
    "extract",
    "colorthres",
    "invert",
    "posterize",
    "gradient",
    "sepia",
];

pub fn card(id: &str) -> &'static [Row] {
    match id {
        "colors" => COLORS,
        _ => ESSENTIAL,
    }
}

fn filter<'a>(preset: &'a Preset, module: &str) -> Option<&'a Filter> {
    preset.video_filters.get(module)
}

fn value(preset: &Preset, module: &str, param: &Param) -> f32 {
    filter(preset, module)
        .and_then(|f| f.values.get(param.option))
        .copied()
        .unwrap_or(param.default)
        .clamp(param.min, param.max)
}

/// Whether `module` runs: switched on, or (slider-only modules) any slider
/// away from neutral.
pub fn running(preset: &Preset, module: &str) -> bool {
    let switched = COLORS
        .iter()
        .any(|r| matches!(r, Row::Toggle { module: m, .. } if *m == module));
    if switched {
        return filter(preset, module).is_some_and(|f| f.on);
    }
    ESSENTIAL.iter().chain(COLORS).any(|row| match row {
        Row::Slider {
            module: m,
            param,
            neutral,
        } if *m == module => {
            let v = value(preset, module, param);
            if module == "gradfun" {
                v >= 4.0
            } else {
                v != *neutral
            }
        }
        _ => false,
    })
}

fn rgb(value: u32) -> Color {
    Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

pub fn rows(preset: &Preset, id: &str) -> Vec<FilterRow> {
    card(id)
        .iter()
        .map(|row| match row {
            Row::Slider { module, param, .. } => param.row(value(preset, module, param)),
            Row::Toggle { module, label } => FilterRow {
                kind: RowKind::Toggle,
                label: (*label).into(),
                on: filter(preset, module).is_some_and(|f| f.on),
                ..Default::default()
            },
            Row::Option {
                module,
                option,
                label,
                default,
            } => FilterRow {
                kind: RowKind::Toggle,
                label: (*label).into(),
                on: filter(preset, module)
                    .and_then(|f| f.values.get(*option))
                    .map_or(*default, |v| *v != 0.0),
                ..Default::default()
            },
            Row::Color {
                module,
                option,
                label,
                default,
            } => {
                let color = filter(preset, module)
                    .and_then(|f| f.values.get(*option))
                    .map_or(*default, |v| *v as u32);
                FilterRow {
                    kind: RowKind::Color,
                    label: (*label).into(),
                    value: format!("#{color:06X}").into(),
                    swatch: rgb(color),
                    ..Default::default()
                }
            }
        })
        .collect()
}

/// Changes row `index` of card `id`; returns the module it belongs to.
pub fn set(preset: &mut Preset, id: &str, index: usize, change: Change) -> Option<&'static str> {
    let row = card(id).get(index)?;
    let (module, option, value) = match (row, change) {
        (Row::Slider { module, param, .. }, Change::Position(at)) => {
            (*module, param.option, param.at(at))
        }
        (Row::Slider { module, param, .. }, Change::Text(text)) => {
            let typed = text
                .trim()
                .trim_end_matches('°')
                .trim()
                .parse::<f32>()
                .ok()?;
            (*module, param.option, typed.clamp(param.min, param.max))
        }
        (Row::Toggle { module, .. }, Change::Switch(on)) => {
            preset.video_filters.entry((*module).into()).or_default().on = on;
            return Some(module);
        }
        (Row::Option { module, option, .. }, Change::Switch(on)) => {
            (*module, *option, f32::from(u8::from(on)))
        }
        (Row::Color { module, option, .. }, Change::Text(text)) => {
            let hex = text.trim().trim_start_matches('#');
            let color = u32::from_str_radix(hex, 16)
                .ok()
                .filter(|c| *c <= 0xFF_FFFF)?;
            (*module, *option, color as f32)
        }
        _ => return None,
    };
    preset
        .video_filters
        .entry(module.into())
        .or_default()
        .values
        .insert(option.into(), value);
    Some(module)
}

pub enum Change {
    Position(f32),
    Text(String),
    Switch(bool),
}

/// Sends `module`'s options, then turns it on or off.
pub fn apply(player: &Player, preset: &Preset, module: &str) {
    for row in ESSENTIAL.iter().chain(COLORS) {
        let (m, option, value) = match row {
            Row::Slider {
                module: m, param, ..
            } => (*m, param.option, value(preset, m, param).to_string()),
            Row::Option {
                module: m,
                option,
                default,
                ..
            } => {
                let on = filter(preset, m)
                    .and_then(|f| f.values.get(*option))
                    .map_or(*default, |v| *v != 0.0);
                (*m, *option, on.to_string())
            }
            Row::Color {
                module: m,
                option,
                default,
                ..
            } => {
                let color = filter(preset, m)
                    .and_then(|f| f.values.get(*option))
                    .map_or(*default, |v| *v as u32);
                (*m, *option, color.to_string())
            }
            Row::Toggle { .. } => continue,
        };
        if m == module
            && let Err(e) = player.set_filter_option(option, &value)
        {
            eprintln!("ferricone: {option}: {e}");
        }
    }
    if let Err(e) = player.set_filter(module, running(preset, module)) {
        eprintln!("ferricone: {module}: {e}");
    }
}

// The picture -------------------------------------------------------------

pub const ZOOM: [(&str, f32); 5] = [
    ("Fit to Window", 0.0),
    ("1:4 Quarter", 0.25),
    ("1:2 Half", 0.5),
    ("1:1 Original", 1.0),
    ("2:1 Double", 2.0),
];

/// Aspect ratios and crop shapes, as VLC offers them.
pub const RATIOS: [&str; 10] = [
    "Default", "16:9", "4:3", "1:1", "16:10", "2.21:1", "2.35:1", "2.39:1", "5:4", "9:16",
];

pub fn ratio_index(text: &str) -> usize {
    RATIOS.iter().position(|r| *r == text).unwrap_or(0)
}

pub fn ratio_at(index: usize) -> String {
    match RATIOS.get(index) {
        Some(&"Default") | None => String::new(),
        Some(r) => (*r).into(),
    }
}

pub fn zoom_index(zoom: f32) -> usize {
    ZOOM.iter().position(|(_, z)| *z == zoom).unwrap_or(0)
}

const EDGES: [&str; 4] = ["Top", "Bottom", "Left", "Right"];

pub fn crop_rows(view: &View) -> Vec<FilterRow> {
    EDGES
        .iter()
        .zip(view.edges)
        .map(|(label, px)| FilterRow {
            kind: RowKind::Field,
            label: (*label).into(),
            value: format!("{px} px").into(),
            position: px as f32 / MAX_EDGE as f32,
            ..Default::default()
        })
        .collect()
}

pub fn set_edge(view: &mut View, index: usize, change: Change) {
    let px = match change {
        Change::Position(at) => (at.clamp(0.0, 1.0) * MAX_EDGE as f32).round() as u32,
        Change::Text(text) => match text.trim().trim_end_matches("px").trim().parse::<u32>() {
            Ok(px) => px.min(MAX_EDGE),
            Err(_) => return,
        },
        Change::Switch(_) => return,
    };
    if let Some(edge) = view.edges.get_mut(index) {
        *edge = px;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_run_only_when_changed() {
        let mut preset = Preset::default();
        assert!(MODULES.iter().all(|m| !running(&preset, m)));
        set(&mut preset, "essential", 1, Change::Position(0.75)); // brightness 1.5
        assert!(running(&preset, "adjust"));
        set(&mut preset, "essential", 6, Change::Text("3".into())); // gradfun under 4
        assert!(!running(&preset, "gradfun"));
        set(&mut preset, "colors", 0, Change::Switch(true));
        assert!(running(&preset, "extract"));
        set(&mut preset, "colors", 1, Change::Text("#00ff7f".into()));
        assert_eq!(rows(&preset, "colors")[1].value.as_str(), "#00FF7F");
        assert!(set(&mut preset, "colors", 1, Change::Text("#zz".into())).is_none());
    }

    #[test]
    fn crops_in_range() {
        let mut view = View::default();
        set_edge(&mut view, 2, Change::Text("120 px".into()));
        set_edge(&mut view, 0, Change::Position(2.0));
        assert_eq!(view.edges, [MAX_EDGE, 0, 120, 0]);
        assert_eq!(ratio_at(ratio_index("2.35:1")), "2.35:1");
        assert_eq!(ratio_at(0), "");
    }
}
