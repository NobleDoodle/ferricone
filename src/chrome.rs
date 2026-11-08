//! Window chrome for the frameless window: the title bar moves it, the edges
//! resize it (through the compositor's own interactive move/resize),
//! fullscreen toggles, and Quit closes it. Without a frame, only the app can
//! offer minimize, maximize and close: Omarchy's desktop manages windows by
//! keyboard and has no use for them, every other desktop gets them.

use crate::{AppWindow, ResizeEdge};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, Weak};
use slint::winit_030::{WinitWindowAccessor, winit::window::ResizeDirection};
use std::{path::Path, time::Duration};

pub fn connect(app: &AppWindow) {
    app.set_window_buttons(!on_omarchy());

    // Through winit, not Slint's set_minimized: Wayland can't report a window
    // coming back, so Slint would ignore every minimize after the first.
    let ui = app.as_weak();
    app.on_minimize_window(move || {
        if let Some(app) = ui.upgrade() {
            app.window().with_winit_window(|w| w.set_minimized(true));
        }
    });

    let ui = app.as_weak();
    app.on_toggle_maximized(move || {
        if let Some(app) = ui.upgrade() {
            app.window().with_winit_window(|w| w.set_maximized(!w.is_maximized()));
            // A window that already fills the screen won't resize, so no
            // resize would report the new state.
            let ui = ui.clone();
            slint::Timer::single_shot(Duration::from_millis(200), move || sync_maximized(&ui));
        }
    });

    let ui = app.as_weak();
    app.on_window_resized(move || sync_maximized(&ui));

    let ui = app.as_weak();
    app.on_start_move(move || {
        if let Some(app) = ui.upgrade() {
            app.window().with_winit_window(|w| report(w.drag_window()));
            release_pointer(&ui);
        }
    });

    let ui = app.as_weak();
    app.on_toggle_fullscreen(move || {
        if let Some(app) = ui.upgrade() {
            let fullscreen = !app.window().is_fullscreen();
            app.window().set_fullscreen(fullscreen);
            app.set_fullscreen(fullscreen);
        }
    });

    app.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let ui = app.as_weak();
    app.on_start_resize(move |edge| {
        if let Some(app) = ui.upgrade() {
            app.window()
                .with_winit_window(|w| report(w.drag_resize_window(direction(edge))));
            release_pointer(&ui);
        }
    });
}

/// The compositor keeps the pointer during an interactive move or resize, so
/// the button's release never reaches the app, and Slint would send every
/// later click to the item still pressed. Deferred: this runs inside the
/// handling of a pointer event.
/// X11 compositors (xfwm4, picom) show a new window at its first damage after
/// they start tracking it. When the first frame lands before that, an idle
/// window stays invisible until something redraws it: about half the starts
/// under picom, and reported on a Raspberry Pi's Xfce. So on X11 a few more
/// frames follow the first. Call when rendering is set up.
pub fn redraw_after_map(app: &AppWindow) {
    use slint::winit_030::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let on_x11 = app.window().with_winit_window(|w| {
        w.display_handle()
            .is_ok_and(|h| matches!(h.as_raw(), RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)))
    });
    if on_x11 != Some(true) {
        return;
    }
    for ms in [250, 1000, 3000] {
        let ui = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(ms), move || {
            if let Some(app) = ui.upgrade() {
                app.window().request_redraw();
            }
        });
    }
}

fn release_pointer(ui: &Weak<AppWindow>) {
    let ui = ui.clone();
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(app) = ui.upgrade() {
            app.window().dispatch_event(WindowEvent::PointerExited);
        }
    });
}

/// The compositor has the last word on whether the window is maximized.
fn sync_maximized(ui: &Weak<AppWindow>) {
    if let Some(app) = ui.upgrade() {
        let maximized = app.window().with_winit_window(|w| w.is_maximized());
        app.set_window_maximized(maximized.unwrap_or(false));
    }
}

/// Whether this system is Omarchy: os-release since Omarchy became its own
/// distribution, the install folder before that, when it was set up on Arch.
fn on_omarchy() -> bool {
    let os_release = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok());
    os_release.is_some_and(|text| names_omarchy(&text))
        || std::env::var_os("OMARCHY_PATH").is_some_and(|dir| Path::new(&dir).is_dir())
        || std::env::var_os("HOME")
            .is_some_and(|home| Path::new(&home).join(".local/share/omarchy/bin").is_dir())
}

/// os-release's `ID`, or `ID_LIKE` for a system built on it, is Omarchy.
fn names_omarchy(os_release: &str) -> bool {
    os_release.lines().any(|line| {
        line.strip_prefix("ID=")
            .or_else(|| line.strip_prefix("ID_LIKE="))
            .is_some_and(|ids| {
                ids.trim_matches(['"', '\''])
                    .split_whitespace()
                    .any(|id| id == "omarchy")
            })
    })
}

fn direction(edge: ResizeEdge) -> ResizeDirection {
    match edge {
        ResizeEdge::North => ResizeDirection::North,
        ResizeEdge::South => ResizeDirection::South,
        ResizeEdge::East => ResizeDirection::East,
        ResizeEdge::West => ResizeDirection::West,
        ResizeEdge::NorthEast => ResizeDirection::NorthEast,
        ResizeEdge::NorthWest => ResizeDirection::NorthWest,
        ResizeEdge::SouthEast => ResizeDirection::SouthEast,
        ResizeEdge::SouthWest => ResizeDirection::SouthWest,
    }
}

fn report<E: std::fmt::Display>(result: Result<(), E>) {
    if let Err(e) = result {
        eprintln!("ferricone: window move/resize: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::names_omarchy;

    #[test]
    fn os_release_names_omarchy() {
        assert!(names_omarchy("NAME=\"Omarchy\"\nID=omarchy\nID_LIKE=arch\n"));
        assert!(names_omarchy("ID=\"omarchy\"\n"));
        assert!(names_omarchy("ID=someos\nID_LIKE=\"omarchy arch\"\n"));
        assert!(!names_omarchy("NAME=\"Arch Linux\"\nID=arch\n"));
        assert!(!names_omarchy("NAME=\"Omarchy\"\nID=fedora\nID_LIKE=\"rhel centos\"\n"));
        assert!(!names_omarchy(""));
    }
}
