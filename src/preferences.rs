//! The Settings window (ui/preferences.slint, ⋯ › Settings or Ctrl + P): its
//! pages, built from what works today, and what each setting does. The
//! values live in prefs (`app.toml`). Settings that are per folder (speed,
//! tracks, delays, the video, audio and subtitle look) stay in the player's
//! ⚙ sidebar, and those pages say so with a button there rather than
//! duplicate them (plan §7: merge duplicates). libVLC options take effect
//! the next time Ferricone starts; their info says so.

use crate::{
    AppWindow, SettingGroup, SettingItem, SettingKind, SettingsTab,
    controls::Shared,
    prefs::{self, AutoHide, Finish, LogLevel, Mode, Resume},
    theme,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::Cell;

const PAGES: [&str; 13] = [
    "General",
    "Interface",
    "Privacy",
    "Controls",
    "Playback",
    "Video",
    "Audio",
    "Subtitles",
    "Library",
    "Network",
    "Performance",
    "Recording",
    "Extensions",
];

/// The page titles, as in the Figma frames.
fn title(page: usize) -> &'static str {
    match page {
        0 => "Global Settings",
        1 => "Interface Settings",
        2 => "Privacy Settings",
        3 => "Controls",
        4 => "Playback Preferences",
        5 => "Video Defaults",
        6 => "Audio Defaults",
        7 => "Subtitle Defaults",
        8 => "Library",
        9 => "Network",
        10 => "Performance",
        11 => "Recording",
        _ => "Extensions",
    }
}

const NEXT_START: &str = " Takes effect the next time Ferricone starts.";

fn strings(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>(),
    ))
}

fn item(id: &str, kind: SettingKind, label: &str, info: &str) -> SettingItem {
    SettingItem {
        id: id.into(),
        kind,
        label: label.into(),
        info: info.into(),
        ..Default::default()
    }
}

fn toggle(id: &str, label: &str, info: &str, on: bool) -> SettingItem {
    SettingItem {
        on,
        ..item(id, SettingKind::Toggle, label, info)
    }
}

fn choice(id: &str, label: &str, info: &str, options: &[&str], selected: usize) -> SettingItem {
    SettingItem {
        options: strings(options),
        selected: selected as i32,
        ..item(id, SettingKind::Choice, label, info)
    }
}

fn segmented(id: &str, label: &str, info: &str, options: &[&str], selected: usize) -> SettingItem {
    SettingItem {
        kind: SettingKind::Segmented,
        ..choice(id, label, info, options, selected)
    }
}

fn text(id: &str, label: &str, info: &str, value: &str) -> SettingItem {
    SettingItem {
        value: value.into(),
        ..item(id, SettingKind::Text, label, info)
    }
}

fn button(id: &str, label: &str, info: &str, pressed: &str) -> SettingItem {
    SettingItem {
        value: pressed.into(),
        ..item(id, SettingKind::Button, label, info)
    }
}

/// A line of text, with a button when `pressed` isn't empty.
fn note(id: &str, label: &str, pressed: &str) -> SettingItem {
    SettingItem {
        value: pressed.into(),
        ..item(id, SettingKind::Note, label, "")
    }
}

fn disabled(mut item: SettingItem, off: bool) -> SettingItem {
    item.disabled = off;
    item
}

fn group(title: &str, items: Vec<SettingItem>) -> SettingGroup {
    SettingGroup {
        title: title.into(),
        advanced: false,
        items: ModelRc::new(VecModel::from(items)),
    }
}

fn advanced(title: &str, items: Vec<SettingItem>) -> SettingGroup {
    SettingGroup {
        advanced: true,
        ..group(title, items)
    }
}

/// The hide delay's slider: 0–10 s.
const HIDE_MAX: f32 = 10.0;
/// The network buffer's slider: 0.3–10 s.
const CACHE_MIN: f32 = 300.0;
const CACHE_MAX: f32 = 10_000.0;

fn pages(app: &AppWindow, page: usize, state: &State) -> Vec<SettingGroup> {
    let p = prefs::get();
    let omarchy = app.global::<crate::Appearance>().get_omarchy_available();
    let follows_omarchy = omarchy && p.omarchy;
    match page {
        0 => {
            let mut appearance = Vec::new();
            if omarchy {
                appearance.push(toggle(
                    "omarchy",
                    "Use Omarchy theme colors",
                    "Take the colors and light or dark mode of your Omarchy theme, and follow it when the theme changes.",
                    p.omarchy,
                ));
            }
            appearance.push(disabled(
                segmented(
                    "mode",
                    "Theme",
                    "Dark, Light, or System to follow the desktop's light or dark setting.",
                    &["Dark", "Light", "System"],
                    match p.mode {
                        Mode::Dark => 0,
                        Mode::Light => 1,
                        Mode::System => 2,
                    },
                ),
                follows_omarchy,
            ));
            appearance.push(disabled(
                item(
                    "accent",
                    SettingKind::Accents,
                    "Accent color",
                    "The color of the play button, selections and progress. The last swatch picks any color.",
                ),
                follows_omarchy,
            ));
            vec![
                group("Appearance", appearance),
                group(
                    "Diagnostics",
                    vec![
                        toggle(
                            "log",
                            "Save a log",
                            &format!("Write the player engine's messages to the log file below.{NEXT_START}"),
                            p.log,
                        ),
                        text("log-file", "Log file", "Where the log is written.", &p.log_file),
                        choice(
                            "log-level",
                            "Log details",
                            &format!("How much the engine reports, in the log and in a terminal.{NEXT_START}"),
                            &["Nothing", "Errors only", "Errors and warnings", "Everything"],
                            match p.log_level {
                                LogLevel::Nothing => 0,
                                LogLevel::Errors => 1,
                                LogLevel::Warnings => 2,
                                LogLevel::Everything => 3,
                            },
                        ),
                    ],
                ),
                advanced(
                    "",
                    vec![
                        text(
                            "extra-args",
                            "Additional player engine options",
                            &format!("libVLC command-line options, separated by spaces.{NEXT_START}"),
                            &p.extra_args,
                        ),
                        button(
                            "open-config",
                            "Open the settings folder",
                            "Where app.toml, presets.toml and library.toml are kept.",
                            "Open",
                        ),
                        button(
                            "reset",
                            "Reset all settings",
                            "Puts every setting on these pages back as it was at first. The library, playlists, history and folder presets stay.",
                            if state.confirm_reset.get() { "Really reset?" } else { "Reset" },
                        ),
                    ],
                ),
            ]
        }
        1 => vec![
            group(
                "Window",
                vec![toggle(
                    "title-file",
                    "Show the file name in the title bar",
                    "Otherwise the title bar always reads Ferricone Player.",
                    p.title_shows_file,
                )],
            ),
            group(
                "Playback Controls",
                vec![
                    choice(
                        "auto-hide",
                        "Auto-hide the controls",
                        "When the title and control bars slide away over the video.",
                        &["In fullscreen only", "Never"],
                        match p.auto_hide {
                            AutoHide::Fullscreen => 0,
                            AutoHide::Never => 1,
                        },
                    ),
                    disabled(
                        SettingItem {
                            position: p.hide_after / HIDE_MAX,
                            value: format!("{:.1} s", p.hide_after).into(),
                            labels: strings(&["0 s", "5 s", "10 s"]),
                            ..item(
                                "hide-after",
                                SettingKind::Slider,
                                "Hide after",
                                "How long the pointer can rest before the bars and the pointer hide; moving it shows them again.",
                            )
                        },
                        p.auto_hide == AutoHide::Never,
                    ),
                ],
            ),
            group(
                "Introduction",
                vec![button(
                    "intro",
                    "Show the introduction again",
                    "The welcome, theme and shortcut pages from the first start.",
                    "Show",
                )],
            ),
        ],
        2 => vec![
            group(
                "History",
                vec![
                    toggle(
                        "save-history",
                        "Save recently played items",
                        "Keeps what you play for the library's Recent, Home and watched progress.",
                        p.save_history,
                    ),
                    toggle(
                        "remember-position",
                        "Remember the playback position",
                        "Offers to continue an item where you left it.",
                        p.remember_position,
                    ),
                    text(
                        "never-remember",
                        "Never remember items matching",
                        "Words, separated by commas: items whose file path or address contains one are left out of the history and positions.",
                        &p.never_remember,
                    ),
                    button(
                        "clear-history",
                        "Clear history now",
                        "Forgets every played item and position. Playlists and the library stay.",
                        "Clear",
                    ),
                ],
            ),
            group(
                "Network",
                vec![toggle(
                    "fetch-art",
                    "Fetch titles and cover art from the internet",
                    &format!("Lets the player engine look up missing titles and covers online.{NEXT_START}"),
                    p.fetch_art,
                )],
            ),
        ],
        3 => vec![group(
            "Keyboard",
            vec![
                button(
                    "shortcuts",
                    "Keyboard shortcuts",
                    "See and change the keys the player acts on.",
                    "Change",
                ),
            ],
        )],
        4 => vec![
            group(
                "Startup",
                vec![
                    choice(
                        "resume",
                        "Continue where you left off",
                        "Ask offers it in a pill while the item starts from the top; Always jumps there.",
                        &["Ask", "Always", "Never"],
                        match p.resume {
                            Resume::Ask => 0,
                            Resume::Always => 1,
                            Resume::Never => 2,
                        },
                    ),
                    toggle(
                        "autoplay",
                        "Start playing as soon as a file opens",
                        "Otherwise items open paused.",
                        p.autoplay,
                    ),
                ],
            ),
            group(
                "Order",
                vec![
                    toggle(
                        "stop-after-each",
                        "Stop after each item",
                        "Don't go on to the next item in the queue.",
                        p.stop_after_each,
                    ),
                    choice(
                        "on-finish",
                        "When the queue ends",
                        "After the last item: offer or play the next file in its folder, stop, or quit.",
                        &["Offer the next in the folder", "Play the next in the folder", "Stop", "Quit"],
                        match p.on_finish {
                            Finish::OfferNext => 0,
                            Finish::PlayNext => 1,
                            Finish::Stop => 2,
                            Finish::Quit => 3,
                        },
                    ),
                ],
            ),
            group(
                "Seeking",
                vec![
                    toggle(
                        "precise-seek",
                        "Seek precisely",
                        "Land on the exact moment clicked; off jumps to the nearest keyframe, which is quicker on large files.",
                        p.precise_seek,
                    ),
                    toggle(
                        "scrub-preview",
                        "Show a thumbnail while scrubbing",
                        "The frame under the pointer above the seek bar.",
                        p.scrub_preview,
                    ),
                ],
            ),
            group(
                "Per Folder",
                vec![note(
                    "sidebar-playback",
                    "Speed, preferred tracks and the sleep timer are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder.",
                    "Open",
                )],
            ),
        ],
        5 => vec![group(
            "",
            vec![note(
                "sidebar-video",
                "Flip, rotation, zoom, aspect ratio, crop and the video filters are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder.",
                "Open",
            )],
        )],
        6 => vec![group(
            "",
            vec![note(
                "sidebar-audio",
                "The equalizer, compressor, spatial audio, stereo widening, pitch and delay are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder.",
                "Open",
            )],
        )],
        7 => vec![
            group(
                "OpenSubtitles",
                vec![
                    SettingItem {
                        secret: true,
                        ..text(
                            "os-key",
                            "API key",
                            "Add Subtitles searches opensubtitles.com with it. Get a free key from your account there (API consumers); it is kept in app.toml, readable only by you.",
                            &p.os_key,
                        )
                    },
                    text(
                        "os-username",
                        "Username",
                        "Optional: logging in raises how many subtitles you can download a day.",
                        &p.os_username,
                    ),
                    SettingItem {
                        secret: true,
                        ..text("os-password", "Password", "Your opensubtitles.com password, kept in app.toml, readable only by you.", &p.os_password)
                    },
                    choice(
                        "os-language",
                        "Search language",
                        "The language Add Subtitles looks for first.",
                        &crate::opensubtitles::LANGUAGES.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
                        crate::opensubtitles::LANGUAGES
                            .iter()
                            .position(|(c, _)| *c == p.os_language)
                            .unwrap_or(0),
                    ),
                ],
            ),
            group(
                "Per Folder",
                vec![note(
                    "sidebar-subtitles",
                    "Preferred subtitles, delay, font, colors, size and outline are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder.",
                    "Open",
                )],
            ),
        ],
        8 => vec![
            group(
                "Folders",
                vec![
                    button(
                        "add-videos",
                        "Add a video folder",
                        "Its videos appear under Videos in the library.",
                        "Add",
                    ),
                    button(
                        "add-music",
                        "Add a music folder",
                        "Its music appears under Music, read for albums, artists and genres.",
                        "Add",
                    ),
                ],
            ),
            group(
                "Thumbnails",
                vec![button(
                    "clear-thumbnails",
                    "Clear the thumbnail cache",
                    "Thumbnails are made again as the library shows them.",
                    "Clear",
                )],
            ),
        ],
        9 => vec![group(
            "Streaming",
            vec![SettingItem {
                position: (p.network_caching as f32 - CACHE_MIN) / (CACHE_MAX - CACHE_MIN),
                value: format!("{:.1} s", p.network_caching as f32 / 1000.0).into(),
                labels: strings(&["0.3 s", "5 s", "10 s"]),
                ..item(
                    "network-caching",
                    SettingKind::Slider,
                    "Network buffer",
                    &format!("How much of a stream is loaded ahead. More rides out a slow connection; less starts sooner.{NEXT_START}"),
                )
            }],
        )],
        10 => vec![group(
            "Decoding",
            vec![toggle(
                "hardware-decoding",
                "Hardware-accelerated decoding",
                &format!("Decode video on the graphics card where it can. Turn off if video shows glitches.{NEXT_START}"),
                p.hardware_decoding,
            )],
        )],
        11 => vec![group(
            "Recording",
            vec![
                text(
                    "record-dir",
                    "Recordings folder",
                    "Where R and the menu's Record save to. Empty uses your Videos folder.",
                    &p.record_dir,
                ),
                button(
                    "record-dir-browse",
                    "Choose the recordings folder",
                    "Pick the folder with the file chooser instead of typing it.",
                    "Browse",
                ),
            ],
        )],
        _ => vec![group(
            "",
            vec![note("", "Add-ons and skins arrive in a later update.", "")],
        )],
    }
}

/// Every setting whose label or explanation contains `query`, grouped as
/// "Page › Section".
fn search(app: &AppWindow, query: &str, state: &State) -> Vec<SettingGroup> {
    let query = query.to_lowercase();
    let mut out = Vec::new();
    for (page, name) in PAGES.iter().enumerate() {
        for g in pages(app, page, state) {
            let items: Vec<SettingItem> = (0..g.items.row_count())
                .filter_map(|i| g.items.row_data(i))
                .filter(|it| {
                    !it.id.is_empty()
                        && (it.label.to_lowercase().contains(&query)
                            || it.info.to_lowercase().contains(&query))
                })
                .collect();
            if !items.is_empty() {
                let title = match g.title.as_str() {
                    "" => name.to_string(),
                    section => format!("{name} › {section}"),
                };
                out.push(group(&title, items));
            }
        }
    }
    out
}

#[derive(Default)]
struct State {
    page: Cell<usize>,
    /// The first press of Reset asks again.
    confirm_reset: Cell<bool>,
    query: std::cell::RefCell<String>,
}

fn show(app: &AppWindow, state: &State) {
    let query = state.query.borrow().clone();
    if query.trim().is_empty() {
        let page = state.page.get();
        let groups = pages(app, page, state);
        app.set_prefs_has_advanced(groups.iter().any(|g| g.advanced));
        app.set_prefs_title(title(page).into());
        app.set_prefs_page(page as i32);
        app.set_prefs_searching(false);
        app.set_prefs_groups(ModelRc::new(VecModel::from(groups)));
    } else {
        app.set_prefs_title("Search Results".into());
        app.set_prefs_searching(true);
        app.set_prefs_groups(ModelRc::new(VecModel::from(search(app, query.trim(), state))));
    }
}

/// The preferences the window itself shows (title, hiding, preview).
pub fn apply(app: &AppWindow) {
    let p = prefs::get();
    app.set_pref_title_file(p.title_shows_file);
    app.set_pref_bars_always(p.auto_hide == AutoHide::Never);
    app.set_pref_hide_after((p.hide_after * 1000.0) as i32);
    app.set_pref_scrub_preview(p.scrub_preview);
}

pub fn connect(app: &AppWindow, session: &Shared) {
    apply(app);
    let state = std::rc::Rc::new(State::default());

    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_show_page(move |page| {
        let Some(app) = ui.upgrade() else { return };
        st.page.set(page.clamp(0, PAGES.len() as i32 - 1) as usize);
        st.confirm_reset.set(false);
        st.query.borrow_mut().clear();
        show(&app, &st);
    });
    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_search(move |text| {
        let Some(app) = ui.upgrade() else { return };
        *st.query.borrow_mut() = text.to_string();
        show(&app, &st);
    });

    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_toggled(move |id, on| {
        let Some(app) = ui.upgrade() else { return };
        match id.as_str() {
            "omarchy" => app.global::<crate::Appearance>().invoke_set_omarchy(on),
            id => prefs::update(|p| match id {
                "log" => p.log = on,
                "title-file" => p.title_shows_file = on,
                "save-history" => p.save_history = on,
                "remember-position" => p.remember_position = on,
                "fetch-art" => p.fetch_art = on,
                "autoplay" => p.autoplay = on,
                "stop-after-each" => p.stop_after_each = on,
                "precise-seek" => p.precise_seek = on,
                "scrub-preview" => p.scrub_preview = on,
                "hardware-decoding" => p.hardware_decoding = on,
                _ => {}
            }),
        }
        apply(&app);
        show(&app, &st);
    });

    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_chosen(move |id, index| {
        let Some(app) = ui.upgrade() else { return };
        match id.as_str() {
            // Dark, Light, System here; the Appearance global counts Light first.
            "mode" => app
                .global::<crate::Appearance>()
                .invoke_set_mode([1, 0, 2][index.clamp(0, 2) as usize]),
            id => prefs::update(|p| match id {
                "log-level" => {
                    p.log_level = [LogLevel::Nothing, LogLevel::Errors, LogLevel::Warnings, LogLevel::Everything]
                        [index.clamp(0, 3) as usize];
                }
                "auto-hide" => p.auto_hide = if index == 1 { AutoHide::Never } else { AutoHide::Fullscreen },
                "resume" => p.resume = [Resume::Ask, Resume::Always, Resume::Never][index.clamp(0, 2) as usize],
                "os-language" => {
                    if let Some((code, _)) = crate::opensubtitles::LANGUAGES.get(index as usize) {
                        p.os_language = (*code).into();
                    }
                }
                "on-finish" => {
                    p.on_finish = [Finish::OfferNext, Finish::PlayNext, Finish::Stop, Finish::Quit]
                        [index.clamp(0, 3) as usize];
                }
                _ => {}
            }),
        }
        apply(&app);
        show(&app, &st);
    });

    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_slid(move |id, at, done| {
        let Some(app) = ui.upgrade() else { return };
        prefs::update(|p| match id.as_str() {
            "hide-after" => p.hide_after = (at * HIDE_MAX * 2.0).round() / 2.0,
            "network-caching" => {
                let ms = CACHE_MIN + at * (CACHE_MAX - CACHE_MIN);
                p.network_caching = ((ms / 100.0).round() * 100.0) as u32;
            }
            _ => {}
        });
        apply(&app);
        // Redrawing the rows mid-drag would cut the drag short.
        if done {
            show(&app, &st);
        }
    });

    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_typed(move |id, text| {
        let Some(app) = ui.upgrade() else { return };
        let text = text.trim().to_string();
        prefs::update(|p| match id.as_str() {
            "log-file" => p.log_file = text,
            "extra-args" => p.extra_args = text,
            "never-remember" => p.never_remember = text,
            "record-dir" => p.record_dir = text,
            "os-key" => p.os_key = text,
            "os-username" => p.os_username = text,
            "os-password" => p.os_password = text,
            _ => {}
        });
        show(&app, &st);
    });

    let (ui, s, st) = (app.as_weak(), session.clone(), state.clone());
    app.on_prefs_pressed(move |id| {
        let Some(app) = ui.upgrade() else { return };
        pressed(&app, &s, &st, id.as_str());
    });
}

fn pressed(app: &AppWindow, s: &Shared, st: &std::rc::Rc<State>, id: &str) {
    let sidebar = |tab| {
        app.set_prefs_open(false);
        app.invoke_show_settings(tab);
    };
    match id {
        "open-config" => {
            if let Some(dir) = prefs::path().as_deref().and_then(|p| p.parent()) {
                let _ = std::fs::create_dir_all(dir);
                open_folder(dir);
            }
        }
        "reset" => {
            if st.confirm_reset.replace(false) {
                prefs::reset();
                theme::redraw(app);
                crate::shortcuts::apply(app);
                apply(app);
            } else {
                st.confirm_reset.set(true);
            }
        }
        "intro" => {
            app.set_prefs_open(false);
            app.invoke_show_intro(0, false);
        }
        "shortcuts" => app.invoke_open_shortcuts(),
        "clear-history" => {
            s.history.borrow_mut().clear();
            s.positions.borrow_mut().clear();
            crate::library::refresh(app, s);
        }
        "sidebar-playback" => sidebar(SettingsTab::Playback),
        "sidebar-video" => sidebar(SettingsTab::Video),
        "sidebar-audio" => sidebar(SettingsTab::Audio),
        "sidebar-subtitles" => sidebar(SettingsTab::Subtitles),
        "add-videos" => crate::library::ask_folder(app, s, false),
        "add-music" => crate::library::ask_folder(app, s, true),
        "clear-thumbnails" => s.library.thumbs.clear_cache(),
        "record-dir-browse" => {
            let (ui, st) = (app.as_weak(), st.clone());
            let task = slint::spawn_local(async move {
                let picked = rfd::AsyncFileDialog::new()
                    .set_title("Choose the Recordings Folder")
                    .pick_folder()
                    .await;
                if let (Some(dir), Some(app)) = (picked, ui.upgrade()) {
                    prefs::update(|p| p.record_dir = dir.path().to_string_lossy().into_owned());
                    show(&app, &st);
                }
            });
            if let Err(e) = task {
                eprintln!("ferricone: folder dialog: {e}");
            }
            return;
        }
        _ => {}
    }
    show(app, st);
}

fn open_folder(dir: &std::path::Path) {
    #[cfg(target_os = "linux")]
    if let Err(e) = std::process::Command::new("xdg-open").arg(dir).spawn() {
        eprintln!("ferricone: opening {}: {e}", dir.display());
    }
    #[cfg(not(target_os = "linux"))]
    let _ = dir;
}
