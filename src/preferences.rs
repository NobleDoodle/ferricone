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
    i18n::{n_, t, tr},
    theme,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::Cell;

const PAGES: [&str; 13] = [
    n_!("General"),
    n_!("Interface"),
    n_!("Privacy"),
    n_!("Controls"),
    n_!("Playback"),
    n_!("Video"),
    n_!("Audio"),
    n_!("Subtitles"),
    n_!("Library"),
    n_!("Network"),
    n_!("Performance"),
    n_!("Recording"),
    n_!("Extensions"),
];

/// The page titles, as in the Figma frames.
fn title(page: usize) -> &'static str {
    match page {
        0 => n_!("Global Settings"),
        1 => n_!("Interface Settings"),
        2 => n_!("Privacy Settings"),
        3 => n_!("Controls"),
        4 => n_!("Playback Preferences"),
        5 => n_!("Video Defaults"),
        6 => n_!("Audio Defaults"),
        7 => n_!("Subtitle Defaults"),
        8 => n_!("Library"),
        9 => n_!("Network"),
        10 => n_!("Performance"),
        11 => n_!("Recording"),
        _ => n_!("Extensions"),
    }
}

/// `info`, then that it applies at the next start: two sentences, translated
/// apart.
fn next_start(info: &str) -> String {
    format!("{} {}", t(info), t(n_!("Takes effect the next time Ferricone starts.")))
}

/// Translated: the helpers below take source text, marked with `n_!`.
fn strings(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.iter().map(|s| SharedString::from(t(s).as_ref())).collect::<Vec<_>>(),
    ))
}

fn item(id: &str, kind: SettingKind, label: &str, info: &str) -> SettingItem {
    SettingItem {
        id: id.into(),
        kind,
        label: t(label).as_ref().into(),
        info: t(info).as_ref().into(),
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
        value: t(pressed).as_ref().into(),
        ..item(id, SettingKind::Button, label, info)
    }
}

/// A line of text, with a button when `pressed` isn't empty.
fn note(id: &str, label: &str, pressed: &str) -> SettingItem {
    SettingItem {
        value: t(pressed).as_ref().into(),
        ..item(id, SettingKind::Note, label, "")
    }
}

fn disabled(mut item: SettingItem, off: bool) -> SettingItem {
    item.disabled = off;
    item
}

fn group(title: &str, items: Vec<SettingItem>) -> SettingGroup {
    SettingGroup {
        title: t(title).as_ref().into(),
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
                    n_!("Use Omarchy theme colors"),
                    n_!("Take the colors and light or dark mode of your Omarchy theme, and follow it when the theme changes."),
                    p.omarchy,
                ));
            }
            appearance.push(disabled(
                segmented(
                    "mode",
                    n_!("Theme"),
                    n_!("Dark, Light, or System to follow the desktop's light or dark setting."),
                    &[n_!("Dark"), n_!("Light"), n_!("System")],
                    match p.mode {
                        Mode::Dark => 0,
                        Mode::Light => 1,
                        Mode::System => 2,
                    },
                ),
                follows_omarchy,
            ));
            let languages = crate::i18n::languages();
            let mut names = vec![t(n_!("System")).into_owned()];
            names.extend(languages.iter().map(|&(_, name)| name.to_string()));
            appearance.push(SettingItem {
                options: strings(&names.iter().map(String::as_str).collect::<Vec<_>>()),
                selected: languages.iter().position(|&(code, _)| code == p.language).map_or(0, |i| i + 1) as i32,
                ..item(
                    "language",
                    SettingKind::Choice,
                    n_!("Language"),
                    n_!("The language of the interface. System follows the desktop's, in English when Ferricone doesn't have it."),
                )
            });
            appearance.push(disabled(
                item(
                    "accent",
                    SettingKind::Accents,
                    n_!("Accent color"),
                    n_!("The color of the play button, selections and progress. The last swatch picks any color."),
                ),
                follows_omarchy,
            ));
            let (status, action) = crate::update::describe(now());
            let mut updates = vec![
                toggle(
                    "check-updates",
                    n_!("Check for updates automatically"),
                    n_!("Once a day, Ferricone asks GitHub for its newest release. Nothing about you or what you play is sent."),
                    p.check_updates,
                ),
                note("update", &status, action),
            ];
            if let Some(version) = crate::update::news() {
                updates.push(button(
                    "release-notes",
                    &tr!("What's new in {}", version),
                    n_!("The release notes on GitHub."),
                    n_!("Open"),
                ));
            }
            vec![
                group(n_!("Appearance"), appearance),
                group(n_!("Updates"), updates),
                group(
                    n_!("Diagnostics"),
                    vec![
                        toggle(
                            "log",
                            n_!("Save a log"),
                            &next_start(n_!("Write the player engine's messages to the log file below.")),
                            p.log,
                        ),
                        text("log-file", n_!("Log file"), n_!("Where the log is written."), &p.log_file),
                        choice(
                            "log-level",
                            n_!("Log details"),
                            &next_start(n_!("How much the engine reports, in the log and in a terminal.")),
                            &[n_!("Nothing"), n_!("Errors only"), n_!("Errors and warnings"), n_!("Everything")],
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
                            n_!("Additional player engine options"),
                            &next_start(n_!("libVLC command-line options, separated by spaces.")),
                            &p.extra_args,
                        ),
                        button(
                            "open-config",
                            n_!("Open the settings folder"),
                            n_!("Where app.toml, presets.toml and library.toml are kept."),
                            n_!("Open"),
                        ),
                        button(
                            "reset",
                            n_!("Reset all settings"),
                            n_!("Puts every setting on these pages back as it was at first. The library, playlists, history and folder presets stay."),
                            if state.confirm_reset.get() { n_!("Really reset?") } else { n_!("Reset") },
                        ),
                    ],
                ),
            ]
        }
        1 => vec![
            group(
                n_!("Window"),
                vec![toggle(
                    "title-file",
                    n_!("Show the file name in the title bar"),
                    n_!("Otherwise the title bar always reads Ferricone Player."),
                    p.title_shows_file,
                )],
            ),
            group(
                n_!("Playback Controls"),
                vec![
                    choice(
                        "auto-hide",
                        n_!("Auto-hide the controls"),
                        n_!("When the title and control bars slide away over the video."),
                        &[n_!("In fullscreen only"), n_!("Never")],
                        match p.auto_hide {
                            AutoHide::Fullscreen => 0,
                            AutoHide::Never => 1,
                        },
                    ),
                    disabled(
                        SettingItem {
                            position: p.hide_after / HIDE_MAX,
                            value: format!("{:.1} s", p.hide_after).into(),
                            labels: strings(&[n_!("0 s"), n_!("5 s"), n_!("10 s")]),
                            ..item(
                                "hide-after",
                                SettingKind::Slider,
                                n_!("Hide after"),
                                n_!("How long the pointer can rest before the bars and the pointer hide; moving it shows them again."),
                            )
                        },
                        p.auto_hide == AutoHide::Never,
                    ),
                ],
            ),
            group(
                n_!("Introduction"),
                vec![button(
                    "intro",
                    n_!("Show the introduction again"),
                    n_!("The welcome, theme and shortcut pages from the first start."),
                    n_!("Show"),
                )],
            ),
        ],
        2 => vec![
            group(
                n_!("History"),
                vec![
                    toggle(
                        "save-history",
                        n_!("Save recently played items"),
                        n_!("Keeps what you play for the library's Recent, Home and watched progress."),
                        p.save_history,
                    ),
                    toggle(
                        "remember-position",
                        n_!("Remember the playback position"),
                        n_!("Offers to continue an item where you left it."),
                        p.remember_position,
                    ),
                    text(
                        "never-remember",
                        n_!("Never remember items matching"),
                        n_!("Words, separated by commas: items whose file path or address contains one are left out of the history and positions."),
                        &p.never_remember,
                    ),
                    button(
                        "clear-history",
                        n_!("Clear history now"),
                        n_!("Forgets every played item and position. Playlists and the library stay."),
                        n_!("Clear"),
                    ),
                ],
            ),
            group(
                n_!("Network"),
                vec![toggle(
                    "fetch-art",
                    n_!("Fetch titles and cover art from the internet"),
                    &next_start(n_!("Lets the player engine look up missing titles and covers online.")),
                    p.fetch_art,
                )],
            ),
        ],
        3 => vec![group(
            n_!("Keyboard"),
            vec![
                button(
                    "shortcuts",
                    n_!("Keyboard shortcuts"),
                    n_!("See and change the keys the player acts on."),
                    n_!("Change"),
                ),
            ],
        )],
        4 => vec![
            group(
                n_!("Startup"),
                vec![
                    choice(
                        "resume",
                        n_!("Continue where you left off"),
                        n_!("Ask offers it in a pill while the item starts from the top; Always jumps there."),
                        &[n_!("Ask"), n_!("Always"), n_!("Never")],
                        match p.resume {
                            Resume::Ask => 0,
                            Resume::Always => 1,
                            Resume::Never => 2,
                        },
                    ),
                    toggle(
                        "autoplay",
                        n_!("Start playing as soon as a file opens"),
                        n_!("Otherwise items open paused."),
                        p.autoplay,
                    ),
                ],
            ),
            group(
                n_!("Order"),
                vec![
                    toggle(
                        "stop-after-each",
                        n_!("Stop after each item"),
                        n_!("Don't go on to the next item in the queue."),
                        p.stop_after_each,
                    ),
                    choice(
                        "on-finish",
                        n_!("When the queue ends"),
                        n_!("After the last item: offer or play the next file in its folder, stop, or quit."),
                        &[n_!("Offer the next in the folder"), n_!("Play the next in the folder"), n_!("Stop"), n_!("Quit")],
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
                n_!("Seeking"),
                vec![
                    toggle(
                        "precise-seek",
                        n_!("Seek precisely"),
                        n_!("Land on the exact moment clicked; off jumps to the nearest keyframe, which is quicker on large files."),
                        p.precise_seek,
                    ),
                    toggle(
                        "scrub-preview",
                        n_!("Show a thumbnail while scrubbing"),
                        n_!("The frame under the pointer above the seek bar."),
                        p.scrub_preview,
                    ),
                ],
            ),
            group(
                n_!("Per Folder"),
                vec![note(
                    "sidebar-playback",
                    n_!("Speed, preferred tracks and the sleep timer are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder."),
                    n_!("Open"),
                )],
            ),
        ],
        5 => vec![group(
            "",
            vec![note(
                "sidebar-video",
                n_!("Flip, rotation, zoom, aspect ratio, crop and the video filters are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder."),
                n_!("Open"),
            )],
        )],
        6 => vec![group(
            "",
            vec![note(
                "sidebar-audio",
                n_!("The equalizer, compressor, spatial audio, stereo widening, pitch and delay are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder."),
                n_!("Open"),
            )],
        )],
        7 => vec![
            group(
                n_!("OpenSubtitles"),
                vec![
                    SettingItem {
                        secret: true,
                        ..text(
                            "os-key",
                            n_!("API key"),
                            n_!("Add Subtitles searches opensubtitles.com with it. Get a free key from your account there (API consumers); it is kept in app.toml, readable only by you."),
                            &p.os_key,
                        )
                    },
                    text(
                        "os-username",
                        n_!("Username"),
                        n_!("Optional: logging in raises how many subtitles you can download a day."),
                        &p.os_username,
                    ),
                    SettingItem {
                        secret: true,
                        ..text("os-password", n_!("Password"), n_!("Your opensubtitles.com password, kept in app.toml, readable only by you."), &p.os_password)
                    },
                    choice(
                        "os-language",
                        n_!("Search language"),
                        n_!("The language Add Subtitles looks for first."),
                        &crate::opensubtitles::LANGUAGES.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
                        crate::opensubtitles::LANGUAGES
                            .iter()
                            .position(|(c, _)| *c == p.os_language)
                            .unwrap_or(0),
                    ),
                ],
            ),
            group(
                n_!("Per Folder"),
                vec![note(
                    "sidebar-subtitles",
                    n_!("Preferred subtitles, delay, font, colors, size and outline are in the player's settings sidebar (the gear on the control bar), where they can be kept for a folder."),
                    n_!("Open"),
                )],
            ),
        ],
        8 => vec![
            group(
                n_!("Folders"),
                vec![
                    button(
                        "add-videos",
                        n_!("Add a video folder"),
                        n_!("Its videos appear under Videos in the library."),
                        n_!("Add"),
                    ),
                    button(
                        "add-music",
                        n_!("Add a music folder"),
                        n_!("Its music appears under Music, read for albums, artists and genres."),
                        n_!("Add"),
                    ),
                ],
            ),
            group(
                n_!("Thumbnails"),
                vec![button(
                    "clear-thumbnails",
                    n_!("Clear the thumbnail cache"),
                    n_!("Thumbnails are made again as the library shows them."),
                    n_!("Clear"),
                )],
            ),
        ],
        9 => vec![group(
            n_!("Streaming"),
            vec![SettingItem {
                position: (p.network_caching as f32 - CACHE_MIN) / (CACHE_MAX - CACHE_MIN),
                value: format!("{:.1} s", p.network_caching as f32 / 1000.0).into(),
                labels: strings(&[n_!("0.3 s"), n_!("5 s"), n_!("10 s")]),
                ..item(
                    "network-caching",
                    SettingKind::Slider,
                    n_!("Network buffer"),
                    &next_start(n_!("How much of a stream is loaded ahead. More rides out a slow connection; less starts sooner.")),
                )
            }],
        )],
        10 => vec![group(
            n_!("Decoding"),
            vec![toggle(
                "hardware-decoding",
                n_!("Hardware-accelerated decoding"),
                &next_start(n_!("Decode video on the graphics card where it can. Turn off if video shows glitches.")),
                p.hardware_decoding,
            )],
        )],
        11 => vec![group(
            n_!("Recording"),
            vec![
                text(
                    "record-dir",
                    n_!("Recordings folder"),
                    n_!("Where R and the menu's Record save to. Empty uses your Videos folder."),
                    &p.record_dir,
                ),
                button(
                    "record-dir-browse",
                    n_!("Choose the recordings folder"),
                    n_!("Pick the folder with the file chooser instead of typing it."),
                    n_!("Browse"),
                ),
            ],
        )],
        _ => vec![group(
            "",
            vec![note("", n_!("Add-ons and skins arrive in a later update."), "")],
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
                    "" => t(name).into_owned(),
                    section => format!("{} › {section}", t(name)),
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
        app.set_prefs_title(t(title(page)).as_ref().into());
        app.set_prefs_page(page as i32);
        app.set_prefs_searching(false);
        app.set_prefs_groups(ModelRc::new(VecModel::from(groups)));
    } else {
        app.set_prefs_title(tr!("Search Results").into());
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

thread_local! {
    static OPEN: std::cell::RefCell<Option<std::rc::Rc<State>>> = const { std::cell::RefCell::new(None) };
}

/// Shows the rows again while the General page is open: the update status
/// changed.
pub fn refresh(app: &AppWindow) {
    if !app.get_prefs_open() {
        return;
    }
    OPEN.with(|open| {
        if let Some(state) = &*open.borrow()
            && state.page.get() == 0
            && state.query.borrow().trim().is_empty()
        {
            show(app, state);
        }
    });
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn connect(app: &AppWindow, session: &Shared) {
    apply(app);
    let state = std::rc::Rc::new(State::default());
    OPEN.with(|open| *open.borrow_mut() = Some(state.clone()));

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
                "check-updates" => p.check_updates = on,
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
        if id == "check-updates" && on {
            crate::update::check_if_due(&app);
        }
    });

    let (ui, st) = (app.as_weak(), state.clone());
    app.on_prefs_chosen(move |id, index| {
        let Some(app) = ui.upgrade() else { return };
        match id.as_str() {
            // Dark, Light, System here; the Appearance global counts Light first.
            "mode" => app
                .global::<crate::Appearance>()
                .invoke_set_mode([1, 0, 2][index.clamp(0, 2) as usize]),
            // System, then the languages as listed.
            "language" => {
                let code = match index {
                    i if i >= 1 => crate::i18n::languages().get(i as usize - 1).map_or("", |&(c, _)| c),
                    _ => "",
                };
                prefs::update(|p| p.language = code.into());
                crate::i18n::select(code);
            }
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
        "update" => crate::update::act(app),
        "release-notes" => crate::update::open_page(&crate::update::release_page()),
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
                    .set_title(tr!("Choose the Recordings Folder"))
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
