//! The keys the player acts on (ui/keys.slint's `Keys`) and the Keyboard
//! Shortcuts dialog (ui/shortcuts.slint) that changes them. Changes are kept
//! in prefs (`app.toml`, `keys`, only those that differ from the defaults);
//! a key moved to another action leaves the old one unassigned.

use crate::{
    AppWindow, KeyBinding, Keys, ShortcutRow,
    i18n::{n_, t},
    prefs,
};
use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

/// A changeable action: its `Keys` property, section, label and default key.
struct Action {
    id: &'static str,
    section: &'static str,
    label: &'static str,
    key: &'static str,
}

const fn a(id: &'static str, section: &'static str, label: &'static str, key: &'static str) -> Action {
    Action { id, section, label, key }
}

const ACTIONS: &[Action] = &[
    a("play-pause", n_!("Playback"), n_!("Play / pause"), " "),
    a("stop", n_!("Playback"), n_!("Stop"), "s"),
    a("previous", n_!("Playback"), n_!("Previous item"), "p"),
    a("next", n_!("Playback"), n_!("Next item"), "n"),
    a("back", n_!("Playback"), n_!("Back 10 seconds"), "\u{F702}"),
    a("forward", n_!("Playback"), n_!("Forward 10 seconds"), "\u{F703}"),
    a("slower", n_!("Playback"), n_!("Slower"), "["),
    a("faster", n_!("Playback"), n_!("Faster"), "]"),
    a("record", n_!("Playback"), n_!("Record"), "r"),
    a("fullscreen", n_!("View and Sound"), n_!("Toggle fullscreen"), "f"),
    a("volume-up", n_!("View and Sound"), n_!("Volume up"), "\u{F700}"),
    a("volume-down", n_!("View and Sound"), n_!("Volume down"), "\u{F701}"),
    a("mute", n_!("View and Sound"), n_!("Mute"), "m"),
    a("audio-track", n_!("View and Sound"), n_!("Audio tracks"), "b"),
    a("subtitle-track", n_!("View and Sound"), n_!("Subtitle tracks"), "v"),
    a("audio-earlier", n_!("Sync"), n_!("Audio earlier"), "j"),
    a("audio-later", n_!("Sync"), n_!("Audio later"), "k"),
    a("subtitles-earlier", n_!("Sync"), n_!("Subtitles earlier"), "g"),
    a("subtitles-later", n_!("Sync"), n_!("Subtitles later"), "h"),
];

/// Keys that can't be changed, shown under the rest.
const FIXED: &[(&str, &str)] = &[
    (n_!("Close a panel, a menu or fullscreen"), n_!("Esc")),
    (n_!("Continue where you left off, or play the next item"), n_!("Enter")),
    (n_!("Hide the interface"), n_!("Ctrl + \\")),
    (n_!("Open files"), n_!("Ctrl + O")),
    (n_!("Open a folder"), n_!("Ctrl + F")),
    (n_!("Open a network stream"), n_!("Ctrl + N")),
    (n_!("Add a bookmark"), n_!("Ctrl + B")),
    (n_!("Settings"), n_!("Ctrl + P")),
    (n_!("Quit"), n_!("Ctrl + Q")),
];

/// How a key's text is shown, in the interface's language.
pub fn name(key: &str) -> String {
    t(&raw_name(key)).into_owned()
}

fn raw_name(key: &str) -> String {
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return key.to_uppercase();
    };
    match c {
        ' ' => n_!("Space").into(),
        '\t' => n_!("Tab").into(),
        '\u{8}' => n_!("Backspace").into(),
        '\u{7f}' => n_!("Delete").into(),
        '\u{F700}' => "↑".into(),
        '\u{F701}' => "↓".into(),
        '\u{F702}' => "←".into(),
        '\u{F703}' => "→".into(),
        '\u{F704}'..='\u{F71B}' => format!("F{}", c as u32 - 0xF704 + 1),
        '\u{F727}' => n_!("Insert").into(),
        '\u{F729}' => n_!("Home").into(),
        '\u{F72B}' => n_!("End").into(),
        '\u{F72C}' => n_!("Page Up").into(),
        '\u{F72D}' => n_!("Page Down").into(),
        c => c.to_uppercase().collect(),
    }
}

/// Keys the player can't act on alone: modifiers, Esc, Enter, control
/// characters Slint reports for other keys.
fn usable(key: &str) -> bool {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => !matches!(c, '\u{0}'..='\u{7}' | '\u{a}'..='\u{1f}'),
        _ => false,
    }
}

/// Each action's key now: the default unless changed ("" = none).
fn current() -> Vec<String> {
    let keys = prefs::get().keys;
    ACTIONS
        .iter()
        .map(|a| keys.get(a.id).cloned().unwrap_or_else(|| a.key.to_string()))
        .collect()
}

/// Gives the player its keys.
pub fn apply(app: &AppWindow) {
    let k = Keys::get(app);
    for (action, key) in ACTIONS.iter().zip(current()) {
        let binding = KeyBinding {
            name: name(&key).into(),
            key: key.into(),
        };
        match action.id {
            "play-pause" => k.set_play_pause(binding),
            "stop" => k.set_stop(binding),
            "previous" => k.set_previous(binding),
            "next" => k.set_next(binding),
            "back" => k.set_back(binding),
            "forward" => k.set_forward(binding),
            "slower" => k.set_slower(binding),
            "faster" => k.set_faster(binding),
            "record" => k.set_record(binding),
            "fullscreen" => k.set_fullscreen(binding),
            "volume-up" => k.set_volume_up(binding),
            "volume-down" => k.set_volume_down(binding),
            "mute" => k.set_mute(binding),
            "audio-track" => k.set_audio_track(binding),
            "subtitle-track" => k.set_subtitle_track(binding),
            "audio-earlier" => k.set_audio_earlier(binding),
            "audio-later" => k.set_audio_later(binding),
            "subtitles-earlier" => k.set_subtitles_earlier(binding),
            "subtitles-later" => k.set_subtitles_later(binding),
            _ => {}
        }
    }
}

fn rows(draft: &[String]) -> ModelRc<ShortcutRow> {
    let mut out = Vec::new();
    let mut section = "";
    for (action, key) in ACTIONS.iter().zip(draft) {
        out.push(ShortcutRow {
            section: if action.section != section { t(action.section).as_ref().into() } else { SharedString::new() },
            id: action.id.into(),
            label: t(action.label).as_ref().into(),
            name: if key.is_empty() { SharedString::new() } else { name(key).into() },
        });
        section = action.section;
    }
    for (i, (label, keys)) in FIXED.iter().enumerate() {
        out.push(ShortcutRow {
            section: if i == 0 { t(n_!("Fixed")).as_ref().into() } else { SharedString::new() },
            id: SharedString::new(),
            label: t(label).as_ref().into(),
            name: t(keys).as_ref().into(),
        });
    }
    ModelRc::new(VecModel::from(out))
}

/// Puts `key` on action `index` of `draft`, taking it from any other.
fn assign(draft: &mut [String], index: usize, key: &str) {
    for (i, k) in draft.iter_mut().enumerate() {
        if i != index && k == key {
            k.clear();
        }
    }
    if let Some(k) = draft.get_mut(index) {
        *k = key.into();
    }
}

pub fn connect(app: &AppWindow) {
    apply(app);
    let draft: Rc<RefCell<Vec<String>>> = Rc::default();

    // Key names and the dialog's rows in a new language.
    let (ui, d) = (app.as_weak(), draft.clone());
    crate::i18n::on_change(move || {
        let Some(app) = ui.upgrade() else { return };
        apply(&app);
        if !d.borrow().is_empty() {
            app.set_shortcut_rows(rows(&d.borrow()));
        }
    });

    let (ui, d) = (app.as_weak(), draft.clone());
    app.on_open_shortcuts(move || {
        let Some(app) = ui.upgrade() else { return };
        *d.borrow_mut() = current();
        app.set_shortcut_rows(rows(&d.borrow()));
        app.set_shortcuts_open(true);
    });
    let (ui, d) = (app.as_weak(), draft.clone());
    app.on_shortcut_assign(move |index, key| {
        let Some(app) = ui.upgrade() else { return false };
        let index = index as usize;
        if index >= ACTIONS.len() || !usable(&key) {
            return false;
        }
        assign(&mut d.borrow_mut(), index, &key);
        app.set_shortcut_rows(rows(&d.borrow()));
        true
    });
    let (ui, d) = (app.as_weak(), draft.clone());
    app.on_shortcuts_reset(move || {
        let Some(app) = ui.upgrade() else { return };
        *d.borrow_mut() = ACTIONS.iter().map(|a| a.key.to_string()).collect();
        app.set_shortcut_rows(rows(&d.borrow()));
    });
    let (ui, d) = (app.as_weak(), draft.clone());
    app.on_shortcuts_save(move || {
        let Some(app) = ui.upgrade() else { return };
        let draft = d.borrow().clone();
        prefs::update(|p| {
            p.keys = ACTIONS
                .iter()
                .zip(draft)
                .filter(|(a, key)| a.key != key)
                .map(|(a, key)| (a.id.to_string(), key))
                .collect();
        });
        apply(&app);
        app.set_shortcuts_open(false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keys_as_shown() {
        assert_eq!(name(" "), "Space");
        assert_eq!(name("\u{F702}"), "←");
        assert_eq!(name("\u{F70F}"), "F12");
        assert_eq!(name("x"), "X");
        assert!(!usable("\u{10}"), "a lone Shift");
        assert!(usable("x") && usable(" "));
    }

    #[test]
    fn a_taken_key_moves() {
        let mut draft: Vec<String> = ["s", "p", "n"].map(String::from).to_vec();
        assign(&mut draft, 2, "s");
        assert_eq!(draft, ["", "p", "s"]);
    }
}
