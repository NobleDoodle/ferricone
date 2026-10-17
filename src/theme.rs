//! Design tokens and the appearance settings. The `Theme` global's defaults
//! (ui/theme.slint) are the design's Dark palette; the Figma file's Light set
//! is `light()`. The user picks Light, Dark or System (the desktop's
//! preference, through the XDG settings portal) and an accent: one of the
//! design's nine, or any color. Where Omarchy is installed, "Use Omarchy
//! colors" instead takes its `colors.toml`, key by key over Dark or Light (by
//! its `light.mode`), and follows its theme switches. The choices are kept in
//! prefs (`app.toml`).

use crate::{
    Appearance, AppWindow, Theme,
    prefs::{self, Mode},
};
use slint::{Color, ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;

/// One field per `Theme` property, read from and written back to the global.
macro_rules! tokens {
    ($($field:ident: $get:ident, $set:ident;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Tokens { $(pub $field: Color,)* }

        impl Tokens {
            pub fn read(theme: &Theme) -> Self {
                Self { $($field: theme.$get(),)* }
            }

            pub fn write(&self, theme: &Theme) {
                $(theme.$set(self.$field);)*
            }

            #[cfg(test)]
            fn splat(color: Color) -> Self {
                Self { $($field: color,)* }
            }
        }
    };
}

tokens! {
    bg_b2: get_bg_b2, set_bg_b2;
    bg_b1: get_bg_b1, set_bg_b1;
    bg_b1_glass: get_bg_b1_glass, set_bg_b1_glass;
    bg_a5: get_bg_a5, set_bg_a5;
    bg_a4: get_bg_a4, set_bg_a4;
    bg_a3: get_bg_a3, set_bg_a3;
    bg_a3_glass: get_bg_a3_glass, set_bg_a3_glass;
    bg_a2: get_bg_a2, set_bg_a2;
    bg_a1: get_bg_a1, set_bg_a1;
    scrim: get_scrim, set_scrim;
    divider_dim: get_divider_dim, set_divider_dim;
    divider_bright: get_divider_bright, set_divider_bright;
    fg_1: get_fg_1, set_fg_1;
    fg_2: get_fg_2, set_fg_2;
    fg_a1: get_fg_a1, set_fg_a1;
    fg_a2: get_fg_a2, set_fg_a2;
    fg_a3: get_fg_a3, set_fg_a3;
    fg_a4: get_fg_a4, set_fg_a4;
    fg_b1: get_fg_b1, set_fg_b1;
    accent_main: get_accent_main, set_accent_main;
    accent_bright: get_accent_bright, set_accent_bright;
    accent_dim: get_accent_dim, set_accent_dim;
    bg_accent_bright: get_bg_accent_bright, set_bg_accent_bright;
    bg_accent_dark: get_bg_accent_dark, set_bg_accent_dark;
    volume_low: get_volume_low, set_volume_low;
    volume_mid: get_volume_mid, set_volume_mid;
    volume_high: get_volume_high, set_volume_high;
    volume_max: get_volume_max, set_volume_max;
    thumb: get_thumb, set_thumb;
    cue: get_cue, set_cue;
}

/// The `colors.toml` keys the tokens use. A missing or malformed key is `None`.
#[derive(Default)]
struct Palette {
    accent: Option<Color>,
    background: Option<Color>,
    dark_background: Option<Color>,
    darker_background: Option<Color>,
    lighter_background: Option<Color>,
    foreground: Option<Color>,
    light_foreground: Option<Color>,
    bright_foreground: Option<Color>,
    muted: Option<Color>,
    selection_foreground: Option<Color>,
    green: Option<Color>,
    yellow: Option<Color>,
    orange: Option<Color>,
    red: Option<Color>,
}

impl Palette {
    #[cfg(target_os = "linux")]
    fn parse(text: &str) -> Self {
        let table: toml::Table = text.parse().unwrap_or_default();
        let get = |key| table.get(key).and_then(toml::Value::as_str).and_then(hex);
        Self {
            accent: get("accent"),
            background: get("background"),
            dark_background: get("dark_background"),
            darker_background: get("darker_background"),
            lighter_background: get("lighter_background"),
            foreground: get("foreground"),
            light_foreground: get("light_foreground"),
            bright_foreground: get("bright_foreground"),
            muted: get("muted"),
            selection_foreground: get("selection_foreground"),
            green: get("green"),
            yellow: get("yellow"),
            orange: get("orange"),
            red: get("red"),
        }
    }

    /// `defaults` with every token whose keys are present replaced (plan §2 table).
    fn apply(&self, defaults: Tokens) -> Tokens {
        let mut t = defaults;
        let p = self;
        let white = Color::from_rgb_u8(255, 255, 255);
        if let Some(c) = p.darker_background {
            t.bg_b2 = c;
        }
        if let Some(c) = p.dark_background {
            (t.bg_b1, t.bg_b1_glass) = (c, c.with_alpha(0.95));
        }
        if let Some(c) = p.background {
            (t.bg_a4, t.bg_a3, t.bg_a3_glass) = (c, c, c.with_alpha(0.95));
        }
        if let (Some(dark), Some(bg)) = (p.dark_background, p.background) {
            t.bg_a5 = mix(dark, bg, 0.5);
        }
        if let (Some(bg), Some(light)) = (p.background, p.lighter_background) {
            (t.bg_a2, t.bg_a1, t.divider_dim) = (
                mix(bg, light, 0.3),
                mix(bg, light, 0.5),
                mix(bg, light, 0.3),
            );
        }
        if let Some(c) = p.lighter_background {
            t.divider_bright = c;
        }
        if let Some(c) = p.bright_foreground {
            t.fg_1 = c;
        }
        if let Some(c) = p.light_foreground {
            t.fg_2 = c;
        }
        if let Some(c) = p.foreground {
            (t.fg_a1, t.fg_a2) = (c, c);
        }
        if let Some(c) = p.muted {
            t.fg_a3 = c;
        }
        if let (Some(muted), Some(bg)) = (p.muted, p.background) {
            t.fg_a4 = mix(muted, bg, 0.5);
        }
        if let Some(c) = p.selection_foreground {
            t.fg_b1 = c;
        }
        if let Some(accent) = p.accent {
            (t.accent_main, t.accent_bright) = (accent, mix(accent, white, 0.2));
            if let Some(bg) = p.background {
                t.accent_dim = mix(accent, bg, 0.2);
            }
            if let Some(darker) = p.darker_background {
                t.bg_accent_bright = mix(darker, accent, 0.26);
            }
            if let Some(dark) = p.dark_background {
                t.bg_accent_dark = mix(dark, accent, 0.06);
            }
        }
        for (token, key) in [
            (&mut t.volume_low, p.green),
            (&mut t.volume_mid, p.yellow),
            (&mut t.volume_high, p.orange),
            (&mut t.volume_max, p.red),
        ] {
            if let Some(c) = key {
                *token = c;
            }
        }
        t
    }
}

/// The design's accents (Figma `Onboarding`): orange, red, gold, green, cyan,
/// blue, purple, pink and gray.
pub const ACCENTS: [u32; 9] = [
    0xd37918, 0xe15032, 0xc49e23, 0x19a234, 0x18aad3, 0x185dd3, 0x8e42ff, 0xf052b3, 0xafafaf,
];

fn rgb(v: u32) -> Color {
    Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn rgba(v: u32, alpha: f32) -> Color {
    rgb(v).with_alpha(alpha)
}

/// The Figma Light set. Its `FG/1`, `FG/2` and `FG/A4` keep dark-theme
/// values (leftovers, plan §2), so these use `FG/A2` light for the first two
/// and a mid gray for placeholders; `BG/A5` (#888) and `BG/A4` (#b3b3b3) are
/// softened toward the Light player frame's rail and search field.
fn light(dark: &Tokens) -> Tokens {
    Tokens {
        bg_b2: rgb(0xdedede),
        bg_b1: rgb(0xe7e7e7),
        bg_b1_glass: rgba(0xe7e7e7, 0.8),
        bg_a5: rgb(0xd0d0d0),
        bg_a4: rgb(0xc9c9c9),
        bg_a3: rgb(0xd5d5d5),
        bg_a3_glass: rgba(0xd5d5d5, 0.75),
        bg_a2: rgb(0xdddddd),
        bg_a1: rgb(0xededed),
        scrim: rgba(0x000000, 0.5),
        divider_dim: rgb(0xacacac),
        divider_bright: rgb(0x9b9b9b),
        fg_1: rgb(0x2e2e2e),
        fg_2: rgb(0x2e2e2e),
        fg_a1: rgb(0x3c3c3c),
        fg_a2: rgb(0x2e2e2e),
        fg_a3: rgb(0x6b6b6b),
        fg_a4: rgb(0x9a9a9a),
        thumb: rgb(0xc4c4c4),
        cue: rgba(0x000000, 0.18),
        ..*dark
    }
}

/// The accent and its derived tokens over a Dark or Light base (plan §2,
/// fitted to both Figma sets); text on it is dark or white for contrast.
fn with_accent(base: Tokens, accent: Color, is_light: bool) -> Tokens {
    let (white, black) = (rgb(0xffffff), rgb(0x000000));
    let luminance = (0.299 * f32::from(accent.red())
        + 0.587 * f32::from(accent.green())
        + 0.114 * f32::from(accent.blue()))
        / 255.0;
    Tokens {
        accent_main: accent,
        accent_bright: mix(accent, white, 0.2),
        accent_dim: if is_light {
            mix(accent, black, 0.35)
        } else {
            mix(accent, base.bg_a3, 0.2)
        },
        bg_accent_bright: mix(base.bg_b2, accent, if is_light { 0.35 } else { 0.26 }),
        bg_accent_dark: if is_light {
            mix(base.bg_a4, accent, 0.25)
        } else {
            mix(base.bg_b1, accent, 0.06)
        },
        fg_b1: if luminance > 0.5 { rgb(0x161616) } else { white },
        ..base
    }
}

fn mode_index(mode: Mode) -> i32 {
    match mode {
        Mode::Light => 0,
        Mode::Dark => 1,
        Mode::System => 2,
    }
}

pub fn mode_from_index(i: i32) -> Mode {
    match i {
        0 => Mode::Light,
        2 => Mode::System,
        _ => Mode::Dark,
    }
}

/// What the colors are made from, on the UI thread (the choices themselves
/// are in prefs).
struct State {
    dark: Tokens,
    /// Omarchy's colors and whether its theme is light, when installed.
    omarchy: Option<(Palette, bool)>,
    /// The desktop prefers light (System mode).
    system_light: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

impl State {
    fn tokens(&self) -> Tokens {
        let prefs = prefs::get();
        if prefs.omarchy
            && let Some((palette, is_light)) = &self.omarchy
        {
            let base = if *is_light { light(&self.dark) } else { self.dark };
            // Omarchy themes all set an accent; without one, keep ours.
            let base = match palette.accent {
                Some(_) => base,
                None => with_accent(base, self.accent(), *is_light),
            };
            return palette.apply(base);
        }
        let is_light = match prefs.mode {
            Mode::Light => true,
            Mode::Dark => false,
            Mode::System => self.system_light,
        };
        let base = if is_light { light(&self.dark) } else { self.dark };
        with_accent(base, self.accent(), is_light)
    }

    fn accent(&self) -> Color {
        hex(&prefs::get().accent).unwrap_or(rgb(ACCENTS[0]))
    }

    /// Writes the colors and the Appearance global's state.
    fn show(&self, app: &AppWindow) {
        self.tokens().write(&app.global::<Theme>());
        let a = app.global::<Appearance>();
        let prefs = prefs::get();
        a.set_mode(mode_index(prefs.mode));
        a.set_accent(self.accent());
        let accent = self.accent();
        a.set_custom_accent(!ACCENTS.iter().any(|&v| rgb(v) == accent));
        a.set_omarchy_available(self.omarchy.is_some());
        a.set_omarchy(prefs.omarchy && self.omarchy.is_some());
    }
}

fn with_state(app: &AppWindow, change: impl FnOnce(&mut State)) {
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            change(state);
            state.show(app);
        }
    });
}

/// Applies the saved appearance and follows the desktop and Omarchy.
pub fn start(app: &AppWindow) {
    let dark = Tokens::read(&app.global::<Theme>());
    let state = State {
        dark,
        omarchy: omarchy::read(),
        system_light: false,
    };
    state.show(app);
    STATE.with(|cell| *cell.borrow_mut() = Some(state));

    let a = app.global::<Appearance>();
    a.set_accents(ModelRc::new(VecModel::from(
        ACCENTS.iter().map(|&v| rgb(v)).collect::<Vec<_>>(),
    )));
    let ui = app.as_weak();
    a.on_set_mode(move |i| {
        if let Some(app) = ui.upgrade() {
            prefs::update(|p| p.mode = mode_from_index(i));
            redraw(&app);
        }
    });
    let ui = app.as_weak();
    a.on_set_accent(move |c| {
        if let Some(app) = ui.upgrade() {
            prefs::update(|p| {
                p.accent = format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue());
            });
            redraw(&app);
        }
    });
    let ui = app.as_weak();
    a.on_set_omarchy(move |on| {
        if let Some(app) = ui.upgrade() {
            prefs::update(|p| p.omarchy = on);
            redraw(&app);
        }
    });

    omarchy::follow(app, |app, read| with_state(app, |s| s.omarchy = read));
    portal::follow(app, |app, light| with_state(app, |s| s.system_light = light));
}

/// Draws the colors again after the preferences changed.
pub fn redraw(app: &AppWindow) {
    with_state(app, |_| {});
}

/// `from` moved toward `to` by `t` (0 = `from`, 1 = `to`).
fn mix(from: Color, to: Color, t: f32) -> Color {
    let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color::from_rgb_u8(
        lerp(from.red(), to.red()),
        lerp(from.green(), to.green()),
        lerp(from.blue(), to.blue()),
    )
}

/// `#rrggbb`
fn hex(s: &str) -> Option<Color> {
    let digits = s.strip_prefix('#').filter(|d| d.len() == 6)?;
    let v = u32::from_str_radix(digits, 16).ok()?;
    Some(Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Omarchy's theme: `colors.toml` and `light.mode` in
/// `~/.local/state/omarchy/current/theme`.
#[cfg(target_os = "linux")]
mod omarchy {
    use super::Palette;
    use crate::AppWindow;
    use notify::{RecursiveMode, Watcher};
    use slint::ComponentHandle;
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    fn current() -> Option<PathBuf> {
        // omarchy-theme-set uses $HOME/.local/state, not $XDG_STATE_HOME.
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".local/state/omarchy/current"))
            .filter(|dir| dir.is_dir())
    }

    /// The colors and whether the theme is light; None without Omarchy.
    pub fn read() -> Option<(Palette, bool)> {
        let theme = current()?.join("theme");
        let text = std::fs::read_to_string(theme.join("colors.toml")).ok()?;
        Some((Palette::parse(&text), theme.join("light.mode").exists()))
    }

    /// Calls `changed` on the UI thread after each theme switch.
    pub fn follow(
        app: &AppWindow,
        changed: impl Fn(&AppWindow, Option<(Palette, bool)>) + Send + Clone + 'static,
    ) {
        let Some(current) = current() else { return };
        let theme_dir = current.join("theme");
        let ui = app.as_weak();
        // omarchy-theme-set replaces the whole theme directory (rm -rf, then
        // mv), so watch its parent and reload once the swap has settled.
        std::thread::spawn(move || {
            let (tx, rx) = mpsc::channel();
            let mut watcher = match notify::recommended_watcher(tx) {
                Ok(w) => w,
                Err(e) => return eprintln!("ferricone: theme watch: {e}"),
            };
            if let Err(e) = watcher.watch(&current, RecursiveMode::NonRecursive) {
                return eprintln!("ferricone: theme watch: {e}");
            }
            let touches_theme = |event: &notify::Result<notify::Event>| {
                event.as_ref().is_ok_and(|e| e.paths.contains(&theme_dir))
            };
            while let Ok(event) = rx.recv() {
                if !touches_theme(&event) {
                    continue;
                }
                while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}
                let read = read();
                let (ui, changed) = (ui.clone(), changed.clone());
                let applied = slint::invoke_from_event_loop(move || {
                    if let Some(app) = ui.upgrade() {
                        changed(&app, read);
                    }
                });
                if applied.is_err() {
                    return; // the event loop has ended
                }
            }
        });
    }
}

#[cfg(not(target_os = "linux"))]
mod omarchy {
    use super::Palette;
    use crate::AppWindow;

    pub fn read() -> Option<(Palette, bool)> {
        None
    }

    pub fn follow(_: &AppWindow, _: impl Fn(&AppWindow, Option<(Palette, bool)>)) {}
}

/// The desktop's light or dark preference (System mode), from the XDG
/// settings portal's `org.freedesktop.appearance color-scheme`: 1 dark,
/// 2 light, 0 none (dark).
#[cfg(target_os = "linux")]
mod portal {
    use crate::AppWindow;
    use dbus::{
        Message,
        arg::{RefArg, Variant},
        blocking::Connection,
        message::MatchRule,
    };
    use slint::ComponentHandle;
    use std::time::Duration;

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";

    fn is_light(value: &dyn RefArg) -> bool {
        // ReadOne gives the u32; the older Read nests it once more.
        let inner = value.as_iter().and_then(|mut i| i.next().map(|v| v.as_u64()));
        value.as_u64().or(inner.flatten()) == Some(2)
    }

    /// Reads the preference now and calls `changed` on the UI thread with it
    /// and with every change.
    pub fn follow(app: &AppWindow, changed: impl Fn(&AppWindow, bool) + Send + Clone + 'static) {
        let ui = app.as_weak();
        std::thread::spawn(move || {
            let Ok(conn) = Connection::new_session() else { return };
            let send = |light: bool| {
                let (ui, changed) = (ui.clone(), changed.clone());
                slint::invoke_from_event_loop(move || {
                    if let Some(app) = ui.upgrade() {
                        changed(&app, light);
                    }
                })
                .is_ok()
            };
            let proxy = conn.with_proxy(DEST, PATH, Duration::from_secs(2));
            let now: Result<(Variant<Box<dyn RefArg>>,), _> = proxy.method_call(
                SETTINGS,
                "ReadOne",
                ("org.freedesktop.appearance", "color-scheme"),
            );
            match now {
                Ok((value,)) => {
                    if !send(is_light(&*value.0)) {
                        return;
                    }
                }
                // No portal: System stays dark.
                Err(_) => return,
            }
            let rule = MatchRule::new_signal(SETTINGS, "SettingChanged");
            let ui_alive = ui.clone();
            let added = conn.add_match(rule, move |(): (), _, message: &Message| {
                let mut args = message.iter_init();
                let namespace: Option<String> = args.read().ok();
                let key: Option<String> = args.read().ok();
                let value: Option<Variant<Box<dyn RefArg>>> = args.read().ok();
                if namespace.as_deref() == Some("org.freedesktop.appearance")
                    && key.as_deref() == Some("color-scheme")
                    && let Some(value) = value
                {
                    let light = is_light(&*value.0);
                    let (ui, changed) = (ui_alive.clone(), changed.clone());
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = ui.upgrade() {
                            changed(&app, light);
                        }
                    });
                }
                true
            });
            if added.is_err() {
                return;
            }
            while ui.upgrade_in_event_loop(|_| {}).is_ok() {
                if conn.process(Duration::from_secs(5)).is_err() {
                    return;
                }
            }
        });
    }
}

#[cfg(not(target_os = "linux"))]
mod portal {
    use crate::AppWindow;

    pub fn follow(_: &AppWindow, _: impl Fn(&AppWindow, bool)) {}
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    const DEFAULT: Color = Color::from_argb_u8(255, 1, 2, 3);

    fn apply(toml: &str) -> Tokens {
        Palette::parse(toml).apply(Tokens::splat(DEFAULT))
    }

    #[test]
    fn missing_or_malformed_file_keeps_defaults() {
        assert_eq!(apply(""), Tokens::splat(DEFAULT));
        assert_eq!(apply("this is [not toml"), Tokens::splat(DEFAULT));
    }

    #[test]
    fn keys_map_to_tokens() {
        let t = apply(
            r##"accent = "#498d70"
background = "#1e2422"
darker_background = "#0f1211""##,
        );
        assert_eq!(t.accent_main, hex("#498d70").unwrap());
        assert_eq!(
            (t.bg_a3, t.bg_a4),
            (hex("#1e2422").unwrap(), hex("#1e2422").unwrap())
        );
        assert_eq!(t.bg_a3_glass.alpha(), 242);
        assert_eq!(t.bg_b2, hex("#0f1211").unwrap());
        assert_eq!(
            t.accent_dim,
            mix(hex("#498d70").unwrap(), hex("#1e2422").unwrap(), 0.2)
        );
    }

    #[test]
    fn each_missing_key_keeps_its_default() {
        let t = apply(
            r##"accent = "#498d70"
foreground = "not a color""##,
        );
        assert_eq!(t.accent_main, hex("#498d70").unwrap());
        assert_eq!((t.fg_a1, t.fg_a2, t.bg_b1), (DEFAULT, DEFAULT, DEFAULT));
        // accent_dim also needs `background`, which is missing
        assert_eq!(t.accent_dim, DEFAULT);
    }

    #[test]
    fn accent_sets_its_tokens_and_readable_text() {
        let dark = Tokens::splat(rgb(0x1b1b1b));
        let orange = with_accent(dark, rgb(0xd37918), false);
        assert_eq!(orange.accent_main, rgb(0xd37918));
        assert_eq!(orange.fg_b1, rgb(0x161616), "dark text on a light accent");
        let blue = with_accent(dark, rgb(0x185dd3), false);
        assert_eq!(blue.fg_b1, rgb(0xffffff), "white text on a dark accent");
        let lit = light(&dark);
        assert_eq!(lit.bg_b1, rgb(0xe7e7e7));
        assert_eq!(lit.volume_low, dark.volume_low, "volume colors stay");
    }

    #[test]
    fn mix_and_hex() {
        let (black, white) = (hex("#000000").unwrap(), hex("#ffffff").unwrap());
        assert_eq!(mix(black, white, 0.5), hex("#808080").unwrap());
        assert_eq!(mix(black, white, 0.0), black);
        assert_eq!(hex("#12345"), None);
        assert_eq!(hex("123456"), None);
    }
}
