//! Window chrome for the frameless window: the title bar moves it, the edges
//! resize it (through the compositor's own interactive move/resize),
//! fullscreen toggles, and Quit closes it.

use crate::{AppWindow, ResizeEdge};
use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit::window::ResizeDirection};

pub fn connect(app: &AppWindow) {
    let ui = app.as_weak();
    app.on_start_move(move || {
        if let Some(app) = ui.upgrade() {
            app.window().with_winit_window(|w| report(w.drag_window()));
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
        }
    });
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
