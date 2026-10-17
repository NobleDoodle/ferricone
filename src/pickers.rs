//! What the Subtitles tab's pickers need from outside the UI: the color
//! picker's custom colors (kept in `colors.toml` in the state folder) and
//! its hex field's parsing, and the font field's list of installed families,
//! searched as the user types.

use crate::{AppWindow, FontList, Palette, presets, state};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    rc::Rc,
    sync::{Arc, OnceLock},
};

/// One row of the grid beside the + button.
const CUSTOM_LIMIT: usize = 8;
/// The most families listed at once; typing narrows them.
const MATCH_LIMIT: usize = 300;
/// The font field's first entry: VLC's own default.
pub const DEFAULT_FONT: &str = "Default";

#[derive(Default, Deserialize, Serialize)]
struct ColorsFile {
    /// "#RRGGBBAA", newest first.
    #[serde(default)]
    custom: Vec<String>,
}

pub fn connect(app: &AppWindow) {
    connect_palette(app);
    connect_fonts(app);
}

fn connect_palette(app: &AppWindow) {
    let path = state::state_path("colors.toml");
    let file: ColorsFile = state::read(path.as_deref());
    let colors: Vec<slint::Color> = file
        .custom
        .iter()
        .filter_map(|text| parse(text))
        .take(CUSTOM_LIMIT)
        .collect();
    let model = Rc::new(VecModel::from(colors));
    let palette = app.global::<Palette>();
    palette.set_custom(ModelRc::from(model.clone()));

    let save = {
        let model = model.clone();
        move || {
            let custom = model.iter().map(hex).collect();
            state::write(path.as_deref(), &ColorsFile { custom });
        }
    };
    let (m, save_added) = (model.clone(), save.clone());
    palette.on_add_custom(move |color| {
        if let Some(at) = m.iter().position(|c| c == color) {
            m.remove(at);
        }
        m.insert(0, color);
        while m.row_count() > CUSTOM_LIMIT {
            m.remove(m.row_count() - 1);
        }
        save_added();
    });
    palette.on_remove_custom(move |index| {
        let index = index as usize;
        if index < model.row_count() {
            model.remove(index);
            save();
        }
    });
    palette.on_parse_hex(|text, fallback| {
        presets::parse_color_alpha(&text).map_or(fallback, |(color, alpha)| {
            rgba(color, alpha.unwrap_or(fallback.alpha() as u32))
        })
    });
    palette.on_hex(|color| hex(color).into());
}

fn parse(text: &str) -> Option<slint::Color> {
    presets::parse_color_alpha(text).map(|(color, alpha)| rgba(color, alpha.unwrap_or(255)))
}

fn rgba(color: u32, alpha: u32) -> slint::Color {
    slint::Color::from_argb_u8(alpha as u8, (color >> 16) as u8, (color >> 8) as u8, color as u8)
}

/// "#RRGGBB", or "#RRGGBBAA" when it isn't opaque.
fn hex(color: slint::Color) -> String {
    let rgb = format!("#{:02X}{:02X}{:02X}", color.red(), color.green(), color.blue());
    match color.alpha() {
        255 => rgb,
        alpha => format!("{rgb}{alpha:02X}"),
    }
}

// Fonts --------------------------------------------------------------------

type Families = Arc<OnceLock<Vec<String>>>;

fn connect_fonts(app: &AppWindow) {
    // Scanning the system's fonts takes a moment; do it off the UI thread.
    let families: Families = Arc::default();
    let filling = families.clone();
    std::thread::spawn(move || {
        let _ = filling.set(installed_families());
    });
    let fonts = app.global::<FontList>();
    fonts.on_search(move |query| {
        let names = families.get().map_or(&[][..], Vec::as_slice);
        let found: Vec<SharedString> = matches(names, &query)
            .into_iter()
            .map(SharedString::from)
            .collect();
        ModelRc::from(Rc::new(VecModel::from(found)))
    });
}

fn installed_families() -> Vec<String> {
    let mut collection = fontique::Collection::new(fontique::CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut names: Vec<String> = collection
        .family_names()
        .filter(|name| !name.starts_with('.'))
        .map(str::to_string)
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// "Default", then the families containing `query`, those starting with it
/// first.
fn matches<'a>(names: &'a [String], query: &str) -> Vec<&'a str> {
    let query = query.trim().to_lowercase();
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for name in names {
        let lower = name.to_lowercase();
        if lower.starts_with(&query) {
            starts.push(name.as_str());
        } else if lower.contains(&query) {
            contains.push(name.as_str());
        }
    }
    let default = DEFAULT_FONT.to_lowercase().contains(&query);
    default
        .then_some(DEFAULT_FONT)
        .into_iter()
        .chain(starts)
        .chain(contains)
        .take(MATCH_LIMIT)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_families_as_typed() {
        let names = ["Cantarell", "DejaVu Sans", "JetBrains Mono", "Noto Sans Mono"]
            .map(String::from);
        assert_eq!(matches(&names, "").len(), 5);
        assert_eq!(matches(&names, "mono"), ["JetBrains Mono", "Noto Sans Mono"]);
        assert_eq!(matches(&names, "sans"), ["DejaVu Sans", "Noto Sans Mono"]);
        assert_eq!(matches(&names, "Def"), [DEFAULT_FONT]);
    }

    #[test]
    fn writes_hex_with_opacity_only_when_needed() {
        assert_eq!(hex(rgba(0xBF4040, 255)), "#BF4040");
        assert_eq!(hex(rgba(0xBF4040, 0x80)), "#BF404080");
        assert_eq!(parse("#BF404080"), Some(rgba(0xBF4040, 0x80)));
    }
}
