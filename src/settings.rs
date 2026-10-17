//! The settings sidebar: values from the preset in use (src/presets.rs),
//! edits saved to it and applied to the player at once, the footer that
//! gives a folder (a show) its own preset, the sleep timer and the
//! statistics overlay. Keys [ ] (speed), J K (audio delay) and G H
//! (subtitle delay) step the same values.

use crate::{
    AppWindow, FilterRow, Icons, audio_fx,
    controls::{Session, Shared},
    diag::trace,
    presets::{self, Preset, StereoMode},
    video_fx::{self, Change},
    vlc::Player,
};
use slint::{ComponentHandle, Global, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::time::Duration;

const SLEEP: [(&str, Option<u64>); 7] = [
    ("Off", None),
    ("15 Minutes", Some(15)),
    ("30 Minutes", Some(30)),
    ("45 Minutes", Some(45)),
    ("1 Hour", Some(60)),
    ("2 Hours", Some(120)),
    ("End of This Item", None),
];
/// The sleep option that pauses when the item ends.
const SLEEP_END_OF_ITEM: usize = 6;

/// Timers the sidebar runs.
#[derive(Default)]
pub struct Running {
    sleep: Option<Timer>,
    stats: Option<Timer>,
}

/// The preset for the loaded item.
fn current(s: &Session) -> (Preset, bool) {
    s.presets.borrow().for_source(s.loaded.borrow().as_ref())
}

/// The media options an item opens with: the preset's preferred tracks.
pub fn open_options(s: &Session) -> Vec<String> {
    let tracks = current(s).0.tracks;
    let mut options = Vec::new();
    if !tracks.audio_language.is_empty() {
        options.push(format!(":audio-language={}", tracks.audio_language));
    }
    if tracks.audio_track > 0 {
        options.push(format!(":audio-track={}", tracks.audio_track - 1));
    }
    if !tracks.subtitle_language.is_empty() {
        options.push(format!(":sub-language={}", tracks.subtitle_language));
    }
    if tracks.subtitle_track > 0 {
        options.push(format!(":sub-track={}", tracks.subtitle_track - 1));
    }
    options
}

/// Sends the subtitle look as an item opens, so its first line already
/// has it (the rest of the preset waits for playback to start).
pub fn apply_on_open(s: &Session) {
    let subtitles = current(s).0.subtitles;
    s.with_player(|p| apply_subtitles(p, &subtitles));
}

/// Sends the subtitle look to the text renderer, live.
fn apply_subtitles(p: &Player, style: &presets::SubtitleStyle) {
    let options = [
        ("freetype-font", style.font.clone()),
        ("freetype-color", style.color.to_string()),
        ("freetype-outline-color", style.outline_color.to_string()),
        ("freetype-opacity", style.opacity.to_string()),
        ("freetype-outline-opacity", style.outline_opacity.to_string()),
        ("freetype-outline-thickness", style.outline.to_string()),
        ("freetype-background-opacity", style.background.to_string()),
        ("sub-margin", style.offset.to_string()),
    ];
    for (option, value) in options {
        if let Err(e) = p.set_filter_option(option, &value) {
            eprintln!("ferricone: {option}: {e}");
        }
    }
    p.set_subtitle_scale(style.scale);
}

fn track_text(track: u32) -> String {
    if track == 0 {
        "Auto".into()
    } else {
        track.to_string()
    }
}

fn language_text(language: &str) -> String {
    if language.is_empty() {
        "Any".into()
    } else {
        language.into()
    }
}

/// "#RRGGBB", with the opacity as "AA" only when it isn't full.
fn color_text(color: u32, alpha: u32) -> String {
    if alpha >= 255 {
        format!("#{color:06X}")
    } else {
        format!("#{color:06X}{alpha:02X}")
    }
}

fn rgba(color: u32, alpha: u32) -> slint::Color {
    slint::Color::from_argb_u8(alpha as u8, (color >> 16) as u8, (color >> 8) as u8, color as u8)
}

const OUTLINES: [&str; 4] = ["None", "Thin", "Normal", "Thick"];

/// Shows the preset in use in the sidebar.
pub fn refresh(app: &AppWindow, s: &Session) {
    let (preset, own) = current(s);
    app.set_speed(preset.speed);
    app.set_audio_delay(preset.audio_delay as f32);
    app.set_audio_delay_text(presets::seconds_text(preset.audio_delay).into());
    app.set_subtitle_delay(preset.subtitle_delay as f32);
    app.set_subtitle_delay_text(presets::seconds_text(preset.subtitle_delay).into());
    app.set_stereo(
        StereoMode::ALL
            .iter()
            .position(|&m| m == preset.stereo)
            .unwrap_or(0) as i32,
    );
    let (eq_options, eq_selected) = audio_fx::eq_options(&preset.equalizer);
    app.set_eq_presets(strings(eq_options));
    app.set_eq_preset(eq_selected as i32);
    app.set_eq_rows(rows(audio_fx::eq_rows(&preset.equalizer)));
    app.set_two_pass(preset.equalizer.two_pass);
    app.set_compressor_on(preset.compressor.on);
    app.set_compressor_rows(rows(audio_fx::rows(
        &audio_fx::COMPRESSOR,
        &preset.compressor,
    )));
    app.set_spatializer_on(preset.spatializer.on);
    app.set_spatializer_rows(rows(audio_fx::rows(
        &audio_fx::SPATIALIZER,
        &preset.spatializer,
    )));
    app.set_widener_on(preset.widener.on);
    app.set_widener_rows(rows(audio_fx::rows(&audio_fx::WIDENER, &preset.widener)));
    app.set_pitch(preset.pitch);
    app.set_pitch_text(audio_fx::pitch_text(preset.pitch).into());
    app.set_timeline(preset.subtitle_timeline);
    show_view(app, &preset);
    let t = &preset.tracks;
    app.set_audio_track_text(track_text(t.audio_track).into());
    app.set_audio_language_text(language_text(&t.audio_language).into());
    app.set_subtitle_track_text(track_text(t.subtitle_track).into());
    app.set_subtitle_language_text(language_text(&t.subtitle_language).into());
    let st = &preset.subtitles;
    app.set_sub_font_text(if st.font.is_empty() {
        "Default".into()
    } else {
        st.font.clone().into()
    });
    app.set_sub_color_text(color_text(st.color, st.opacity).into());
    app.set_sub_color(rgba(st.color, st.opacity));
    app.set_sub_outline_color_text(color_text(st.outline_color, st.outline_opacity).into());
    app.set_sub_outline_color(rgba(st.outline_color, st.outline_opacity));
    app.set_sub_offset_text(format!("{}px", st.offset).into());
    app.set_sub_scale(st.scale as f32);
    app.set_sub_scale_text(if st.scale == 100 {
        "Normal".into()
    } else {
        format!("{} %", st.scale).into()
    });
    app.set_sub_outline(st.outline as f32 / 2.0);
    app.set_sub_outline_text(OUTLINES[(st.outline / 2).min(3) as usize].into());
    app.set_sub_background(st.background as f32);
    app.set_sub_background_text(format!("{} %", (st.background * 100 + 127) / 255).into());
    app.set_essential_rows(rows(video_fx::rows(&preset, "essential")));
    app.set_crop_rows(rows(video_fx::crop_rows(&preset.view)));
    app.set_colors_rows(rows(video_fx::rows(&preset, "colors")));
    let mut options: Vec<SharedString> = vec!["Default Preset".into()];
    let folder_name = s.loaded.borrow().as_ref().and_then(|source| {
        presets::folder(source)?;
        Some(source.folder())
    });
    if let Some(name) = folder_name {
        options.push(format!("This Folder: {name}").into());
    }
    app.set_presets(ModelRc::new(VecModel::from(options)));
    app.set_preset(i32::from(own));
}

/// Sends the preset in use to the player.
pub fn apply(s: &Session) {
    let (preset, _) = current(s);
    trace!("preset {preset:?}");
    s.with_player(|p| {
        p.set_rate(preset.speed);
        p.set_audio_delay(preset.audio_delay);
        p.set_subtitle_delay(preset.subtitle_delay);
        p.set_stereo(preset.stereo);
        apply_subtitles(p, &preset.subtitles);
        audio_fx::apply_equalizer(p, &preset.equalizer);
        for def in audio_fx::FILTERS {
            let filter = audio_fx::filter(&preset, def.id);
            // A filter that was never on stays untouched.
            if filter.on {
                audio_fx::apply_filter(p, def, filter);
            }
        }
        if preset.pitch != 0.0 {
            audio_fx::apply_pitch(p, preset.pitch);
        }
        for module in video_fx::MODULES {
            if video_fx::running(&preset, module) {
                video_fx::apply(p, &preset, module);
            }
        }
    });
}

/// How the window draws the frame, and the Video tab's Defaults.
fn show_view(app: &AppWindow, preset: &Preset) {
    let view = &preset.view;
    app.set_view_flip(match view.flip {
        presets::Flip::None => 0,
        presets::Flip::Horizontal => 1,
        presets::Flip::Vertical => 2,
    });
    app.set_view_rotate(view.rotate);
    app.set_rotate_text(format!("{} deg", view.rotate).into());
    app.set_view_zoom(view.zoom);
    app.set_zoom(video_fx::zoom_index(view.zoom) as i32);
    app.set_view_aspect(presets::ratio(&view.aspect).unwrap_or(0.0));
    app.set_aspect(video_fx::ratio_index(&view.aspect) as i32);
    app.set_view_crop(presets::ratio(&view.crop).unwrap_or(0.0));
    app.set_crop(video_fx::ratio_index(&view.crop) as i32);
    let [top, bottom, left, right] = view.edges.map(|e| e as i32);
    app.set_view_top(top);
    app.set_view_bottom(bottom);
    app.set_view_left(left);
    app.set_view_right(right);
}

/// Changes the preset in use and applies only the part that changed.
fn edit_part(
    app: &AppWindow,
    s: &Session,
    change: impl FnOnce(&mut Preset),
    send: impl FnOnce(&Player, &Preset),
) {
    s.presets
        .borrow_mut()
        .edit(s.loaded.borrow().as_ref(), change);
    let (preset, _) = current(s);
    s.with_player(|p| send(p, &preset));
    refresh(app, s);
}

/// A Video tab card changed: the crop edges are the window's, the rest a
/// VLC filter, applied on its own.
fn video_change(app: &AppWindow, s: &Session, card: &str, row: usize, change: Change) {
    if card == "crop" {
        edit_part(
            app,
            s,
            |p| video_fx::set_edge(&mut p.view, row, change),
            |_, _| {},
        );
        return;
    }
    let mut module = None;
    s.presets
        .borrow_mut()
        .edit(s.loaded.borrow().as_ref(), |p| {
            module = video_fx::set(p, card, row, change);
        });
    let (preset, _) = current(s);
    if let Some(module) = module {
        s.with_player(|p| video_fx::apply(p, &preset, module));
    }
    refresh(app, s);
}

fn strings(items: Vec<String>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(items))
}

fn rows(rows: Vec<FilterRow>) -> ModelRc<FilterRow> {
    ModelRc::new(VecModel::from(rows))
}

/// Changes the preset in use, applies and shows it.
fn edit(app: &AppWindow, s: &Session, change: impl FnOnce(&mut Preset)) {
    s.presets
        .borrow_mut()
        .edit(s.loaded.borrow().as_ref(), change);
    apply(s);
    refresh(app, s);
}

/// Whether the sleep timer wants a pause at the end of the item; turns it
/// off when it does.
pub fn sleeps_at_end(app: &AppWindow) -> bool {
    if app.get_sleep() as usize != SLEEP_END_OF_ITEM {
        return false;
    }
    app.set_sleep(0);
    true
}

fn osd(app: &AppWindow, icon: crate::IconLayers, text: String, key: &str) {
    app.invoke_show_osd(icon, text.into(), key.into());
}

pub fn connect(app: &AppWindow, session: &Shared) {
    let names: Vec<SharedString> = SLEEP.iter().map(|(n, _)| (*n).into()).collect();
    app.set_sleep_options(ModelRc::new(VecModel::from(names)));
    let stereo: Vec<SharedString> = StereoMode::ALL.iter().map(|m| m.label().into()).collect();
    app.set_stereo_options(ModelRc::new(VecModel::from(stereo)));
    refresh(app, session);

    // Once per item, when it starts: its preset.
    let s = session.clone();
    app.on_media_started(move || {
        if !s.preset_applied.replace(true) {
            apply(&s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_speed(move |speed| {
        if let Some(app) = ui.upgrade() {
            edit(&app, &s, |p| p.speed = speed);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_typed_speed(move |text| {
        let Some(app) = ui.upgrade() else { return };
        match presets::parse_speed(&text) {
            Some(speed) => edit(&app, &s, |p| p.speed = speed),
            None => refresh(&app, &s),
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_nudge_speed(move |step| {
        let Some(app) = ui.upgrade() else { return };
        edit(&app, &s, |p| {
            p.speed = ((p.speed + step) * 4.0).round() / 4.0
        });
        let speed = current(&s).0.speed;
        let key = if step < 0.0 { "[" } else { "]" };
        osd(
            &app,
            Icons::get(&app).get_speed(),
            format!("Speed {speed}x"),
            key,
        );
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_audio_delay(move |seconds| {
        if let Some(app) = ui.upgrade() {
            edit(&app, &s, |p| p.audio_delay = f64::from(seconds));
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_typed_audio_delay(move |text| {
        let Some(app) = ui.upgrade() else { return };
        match presets::parse_seconds(&text) {
            Some(seconds) => edit(&app, &s, |p| p.audio_delay = seconds),
            None => refresh(&app, &s),
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_nudge_audio_delay(move |step| {
        let Some(app) = ui.upgrade() else { return };
        edit(&app, &s, |p| {
            p.audio_delay = round_ms(p.audio_delay + f64::from(step))
        });
        let delay = presets::seconds_text(current(&s).0.audio_delay);
        let key = if step < 0.0 { "J" } else { "K" };
        osd(
            &app,
            Icons::get(&app).get_audio(),
            format!("Audio delay {delay}"),
            key,
        );
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_subtitle_delay(move |seconds| {
        if let Some(app) = ui.upgrade() {
            edit(&app, &s, |p| p.subtitle_delay = f64::from(seconds));
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_typed_subtitle_delay(move |text| {
        let Some(app) = ui.upgrade() else { return };
        match presets::parse_seconds(&text) {
            Some(seconds) => edit(&app, &s, |p| p.subtitle_delay = seconds),
            None => refresh(&app, &s),
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_nudge_subtitle_delay(move |step| {
        let Some(app) = ui.upgrade() else { return };
        edit(&app, &s, |p| {
            p.subtitle_delay = round_ms(p.subtitle_delay + f64::from(step))
        });
        let delay = presets::seconds_text(current(&s).0.subtitle_delay);
        let key = if step < 0.0 { "G" } else { "H" };
        osd(
            &app,
            Icons::get(&app).get_subtitles(),
            format!("Subtitle delay {delay}"),
            key,
        );
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_stereo(move |index| {
        let (Some(app), Some(&mode)) = (ui.upgrade(), StereoMode::ALL.get(index as usize)) else {
            return;
        };
        edit(&app, &s, |p| p.stereo = mode);
    });

    // Audio filters: each applied on its own, live.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_choose_eq(move |index| {
        if let Some(app) = ui.upgrade() {
            edit_part(
                &app,
                &s,
                |p| audio_fx::choose_eq(&mut p.equalizer, index as usize),
                |pl, p| audio_fx::apply_equalizer(pl, &p.equalizer),
            );
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_eq_moved(move |row, at, _done| {
        if let Some(app) = ui.upgrade() {
            let gain = (at * 2.0 - 1.0) * audio_fx::GAIN;
            edit_part(
                &app,
                &s,
                |p| audio_fx::set_gain(&mut p.equalizer, row as usize, gain),
                |pl, p| audio_fx::apply_equalizer(pl, &p.equalizer),
            );
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_eq_typed(move |row, text| {
        let Some(app) = ui.upgrade() else { return };
        let gain = text
            .trim()
            .trim_end_matches("dB")
            .trim()
            .replace('−', "-")
            .parse::<f32>();
        match gain {
            Ok(gain) => edit_part(
                &app,
                &s,
                |p| audio_fx::set_gain(&mut p.equalizer, row as usize, gain),
                |pl, p| audio_fx::apply_equalizer(pl, &p.equalizer),
            ),
            Err(_) => refresh(&app, &s),
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_two_pass(move |on| {
        if let Some(app) = ui.upgrade() {
            edit_part(
                &app,
                &s,
                |p| p.equalizer.two_pass = on,
                |pl, p| audio_fx::apply_equalizer(pl, &p.equalizer),
            );
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_filter(move |id, on| {
        let (Some(app), Some(def)) = (ui.upgrade(), audio_fx::def(&id)) else {
            return;
        };
        edit_part(
            &app,
            &s,
            |p| audio_fx::filter_mut(p, def.id).on = on,
            |pl, p| audio_fx::apply_filter(pl, def, audio_fx::filter(p, def.id)),
        );
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_filter_moved(move |id, row, at, _done| {
        let (Some(app), Some(def)) = (ui.upgrade(), audio_fx::def(&id)) else {
            return;
        };
        let Some(param) = def.params.get(row as usize) else {
            return;
        };
        let value = param.at(at);
        edit_part(
            &app,
            &s,
            |p| {
                audio_fx::filter_mut(p, def.id)
                    .values
                    .insert(param.option.into(), value);
            },
            |pl, p| {
                // Only a running filter takes it now; it starts with it later.
                if audio_fx::filter(p, def.id).on
                    && let Err(e) = pl.set_filter_option(param.option, &value.to_string())
                {
                    eprintln!("ferricone: {}: {e}", param.option);
                }
            },
        );
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_pitch(move |semitones| {
        if let Some(app) = ui.upgrade() {
            edit_part(
                &app,
                &s,
                |p| p.pitch = semitones,
                |pl, p| audio_fx::apply_pitch(pl, p.pitch),
            );
        }
    });

    // The Video tab: the view is drawn by the window, so changing it only
    // needs the preset saved and shown; the cards' filters run in VLC.
    let names: Vec<String> = video_fx::ZOOM.iter().map(|(n, _)| (*n).into()).collect();
    app.set_zoom_options(strings(names));
    app.set_ratio_options(strings(
        video_fx::RATIOS.iter().map(|r| (*r).into()).collect(),
    ));
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_flip(move |index| {
        let flip = match index {
            1 => presets::Flip::Horizontal,
            2 => presets::Flip::Vertical,
            _ => presets::Flip::None,
        };
        if let Some(app) = ui.upgrade() {
            edit_part(&app, &s, |p| p.view.flip = flip, |_, _| {});
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_typed_rotate(move |text| {
        let Some(app) = ui.upgrade() else { return };
        let degrees = text
            .trim()
            .trim_end_matches("deg")
            .trim_end_matches('°')
            .trim()
            .parse::<f32>();
        match degrees {
            Ok(degrees) => edit_part(&app, &s, |p| p.view.rotate = degrees, |_, _| {}),
            Err(_) => refresh(&app, &s),
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_zoom(move |index| {
        let zoom = video_fx::ZOOM.get(index as usize).map_or(0.0, |(_, z)| *z);
        if let Some(app) = ui.upgrade() {
            edit_part(&app, &s, |p| p.view.zoom = zoom, |_, _| {});
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_aspect(move |index| {
        if let Some(app) = ui.upgrade() {
            edit_part(
                &app,
                &s,
                |p| p.view.aspect = video_fx::ratio_at(index as usize),
                |_, _| {},
            );
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_crop(move |index| {
        if let Some(app) = ui.upgrade() {
            edit_part(
                &app,
                &s,
                |p| p.view.crop = video_fx::ratio_at(index as usize),
                |_, _| {},
            );
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_video_moved(move |card, row, at, _done| {
        if let Some(app) = ui.upgrade() {
            video_change(&app, &s, &card, row as usize, Change::Position(at));
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_video_typed(move |card, row, text| {
        if let Some(app) = ui.upgrade() {
            video_change(&app, &s, &card, row as usize, Change::Text(text.into()));
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_video_toggled(move |card, row, on| {
        if let Some(app) = ui.upgrade() {
            video_change(&app, &s, &card, row as usize, Change::Switch(on));
        }
    });

    // Preferred tracks: they apply to the next item that opens.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_typed_track(move |which, text| {
        let Some(app) = ui.upgrade() else { return };
        let text = text.trim().to_string();
        let track = match text.to_lowercase().as_str() {
            "" | "auto" => Some(0),
            t => t.parse::<u32>().ok(),
        };
        let language = match text.to_lowercase().as_str() {
            "any" => String::new(),
            _ => text.clone(),
        };
        let mut known = true;
        edit_part(
            &app,
            &s,
            |p| match which.as_str() {
                "audio-track" => match track {
                    Some(t) => p.tracks.audio_track = t,
                    None => known = false,
                },
                "subtitle-track" => match track {
                    Some(t) => p.tracks.subtitle_track = t,
                    None => known = false,
                },
                "audio-language" => p.tracks.audio_language = language,
                _ => p.tracks.subtitle_language = language,
            },
            |_, _| {},
        );
        if known {
            osd(
                &app,
                Icons::get(&app).get_settings_two_tone(),
                "Applies to the next item opened".into(),
                "",
            );
        }
    });

    // Subtitle look: live, from the next line drawn.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_typed_subtitle_style(move |which, text| {
        let Some(app) = ui.upgrade() else { return };
        let text = text.trim().to_string();
        edit_part(
            &app,
            &s,
            |p| {
                let st = &mut p.subtitles;
                match which.as_str() {
                    "font" if text.eq_ignore_ascii_case("default") => st.font.clear(),
                    "font" => st.font = text,
                    // Six digits keep the opacity; eight set it.
                    "color" | "outline-color" => {
                        let Some((color, alpha)) = presets::parse_color_alpha(&text) else {
                            return;
                        };
                        let (c, a) = if which == "color" {
                            (&mut st.color, &mut st.opacity)
                        } else {
                            (&mut st.outline_color, &mut st.outline_opacity)
                        };
                        *c = color;
                        *a = alpha.unwrap_or(*a);
                    }
                    _ => {
                        let px = text.trim_end_matches("px").trim().parse();
                        st.offset = px.unwrap_or(st.offset);
                    }
                }
            },
            |pl, p| apply_subtitles(pl, &p.subtitles),
        );
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_subtitle_level(move |which, value| {
        let Some(app) = ui.upgrade() else { return };
        edit_part(
            &app,
            &s,
            |p| {
                let st = &mut p.subtitles;
                match which.as_str() {
                    "scale" => st.scale = (value / 5.0).round() as u32 * 5,
                    "outline" => st.outline = value.round() as u32 * 2,
                    _ => st.background = value.round() as u32,
                }
            },
            |pl, p| apply_subtitles(pl, &p.subtitles),
        );
    });

    // The footer: the default preset, or this folder's own.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_choose_preset(move |index| {
        let Some(app) = ui.upgrade() else { return };
        let Some(source) = s.loaded.borrow().clone() else {
            return;
        };
        let text = if index == 1 {
            s.presets.borrow_mut().make_folder(&source);
            format!("Settings now stick to “{}”", source.folder())
        } else {
            s.presets.borrow_mut().forget_folder(&source);
            format!("“{}” uses the default preset", source.folder())
        };
        apply(&s);
        refresh(&app, &s);
        osd(&app, Icons::get(&app).get_settings_two_tone(), text, "");
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_sleep(move |index| {
        let Some(app) = ui.upgrade() else { return };
        app.set_sleep(index);
        let minutes = SLEEP.get(index as usize).and_then(|(_, m)| *m);
        s.running.borrow_mut().sleep = minutes.map(|minutes| {
            let (ui, s) = (app.as_weak(), s.clone());
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(minutes * 60),
                move || {
                    s.with_player(|p| p.set_paused(true));
                    if let Some(app) = ui.upgrade() {
                        app.set_sleep(0);
                        osd(
                            &app,
                            Icons::get(&app).get_pause(),
                            "Sleep timer: paused".into(),
                            "",
                        );
                    }
                },
            );
            timer
        });
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_set_statistics(move |on| {
        let Some(app) = ui.upgrade() else { return };
        app.set_statistics(on);
        s.running.borrow_mut().stats = on.then(|| {
            let (ui, s) = (app.as_weak(), s.clone());
            show_stats(&app, &s);
            let timer = Timer::default();
            timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
                if let Some(app) = ui.upgrade() {
                    show_stats(&app, &s);
                }
            });
            timer
        });
    });
}

/// The statistics overlay's lines.
fn show_stats(app: &AppWindow, s: &Session) {
    let player = s.player.borrow();
    let Some(p) = player.as_ref() else { return };
    let mut lines = Vec::new();
    if let Some((w, h)) = p.video_size() {
        lines.push(format!("Video {w} × {h}"));
    }
    if let Some(stats) = p.stats() {
        lines.push(format!("Frames decoded {}", stats.decoded));
        lines.push(format!("Frames shown {}", stats.displayed));
        lines.push(format!("Late {} · lost {}", stats.late, stats.lost));
    }
    let (preset, _) = current(s);
    lines.push(format!("Speed {}x", preset.speed));
    app.set_stats_text(lines.join("\n").into());
}

fn round_ms(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}
