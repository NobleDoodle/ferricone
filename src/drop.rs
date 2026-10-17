//! Plays files dropped onto the window. winit reports drops on X11, Windows
//! and macOS; on Wayland, where winit 0.30 has no drag and drop, wayland.rs
//! takes them from the compositor itself.

#[cfg(target_os = "linux")]
mod wayland;

use crate::{AppWindow, media::Source};
use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
use std::{cell::RefCell, rc::Rc, time::Duration};

type Open = Box<dyn Fn(Vec<Source>)>;

thread_local! {
    /// What a drop opens; set once by `connect`, called on the UI thread.
    static OPEN: RefCell<Option<Open>> = RefCell::default();
}

/// Keeps drops coming while it lives. Drop it before the window's event loop
/// ends, so the Wayland listener stops while its connection is still open.
pub struct Drops {
    #[cfg(target_os = "linux")]
    _wayland: Rc<RefCell<Option<wayland::Listener>>>,
}

pub fn connect(app: &AppWindow, open: impl Fn(Vec<Source>) + 'static) -> Drops {
    OPEN.set(Some(Box::new(open)));

    // winit sends one event per file; the files of one drop open together.
    let pending: Rc<RefCell<Vec<Source>>> = Rc::default();
    app.window().on_winit_window_event(move |_, event| {
        if let WindowEvent::DroppedFile(path) = event {
            let first = pending.borrow().is_empty();
            pending.borrow_mut().push(Source::File(path.clone()));
            if first {
                let pending = pending.clone();
                slint::Timer::single_shot(Duration::ZERO, move || deliver(pending.take()));
            }
        }
        EventResult::Propagate
    });

    #[cfg(target_os = "linux")]
    {
        let listener = Rc::new(RefCell::new(None));
        let (ui, slot) = (app.as_weak(), listener.clone());
        // The display handle exists once winit has made the window.
        let started = slint::spawn_local(async move {
            let Some(app) = ui.upgrade() else { return };
            let Ok(window) = app.window().winit_window().await else {
                return;
            };
            match wayland::listen(&window, |uris| {
                let sources = crate::media::from_uri_list(&uris);
                let _ = slint::invoke_from_event_loop(move || deliver(sources));
            }) {
                Ok(listener) => *slot.borrow_mut() = listener,
                Err(e) => eprintln!("ferricone: file drops: {e}"),
            }
        });
        if let Err(e) = started {
            eprintln!("ferricone: file drops: {e}");
        }
        Drops { _wayland: listener }
    }
    #[cfg(not(target_os = "linux"))]
    Drops {}
}

fn deliver(sources: Vec<Source>) {
    if sources.is_empty() {
        return;
    }
    OPEN.with_borrow(|open| {
        if let Some(open) = open {
            open(sources);
        }
    });
}
