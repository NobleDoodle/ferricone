//! Playing on another device (Chromecast and similar): the settings
//! sidebar's Video › Play On. libVLC's renderer discoverers look for devices
//! from the first time the sidebar opens; picking one reopens the item on
//! it at the same moment, since libVLC takes a renderer only as an item
//! starts.

use crate::{
    AppWindow,
    controls::{self, Shared},
    vlc::{Discovery, Renderer, Vlc},
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Cast {
    discovery: Option<Discovery>,
    /// The device playing on, None for this one.
    current: Option<Renderer>,
}

thread_local! {
    static CAST: RefCell<Cast> = RefCell::default();
}

fn show(app: &AppWindow) {
    CAST.with(|c| {
        let c = c.borrow();
        let found = c.discovery.as_ref().map(Discovery::found).unwrap_or_default();
        let mut options = vec![SharedString::from("This Device")];
        options.extend(found.iter().map(|r| SharedString::from(r.name.as_str())));
        // The current device stays listed while it is in use.
        let selected = match &c.current {
            None => 0,
            Some(current) => match found.iter().position(|r| r.name == current.name) {
                Some(i) => i + 1,
                None => {
                    options.push(current.name.as_str().into());
                    options.len() - 1
                }
            },
        };
        app.set_render_options(ModelRc::new(VecModel::from(options)));
        app.set_render_selected(selected as i32);
        app.set_render_searching(found.is_empty());
    });
}

pub fn connect(app: &AppWindow, session: &Shared, vlc: Rc<Vlc>) {
    show(app);
    let ui = app.as_weak();
    app.on_render_start(move || {
        let Some(app) = ui.upgrade() else { return };
        CAST.with(|c| {
            let mut c = c.borrow_mut();
            if c.discovery.is_none() {
                let ui = app.as_weak();
                c.discovery = Some(vlc.discover_renderers(move || {
                    let _ = ui.upgrade_in_event_loop(|app| show(&app));
                }));
            }
        });
        show(&app);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_render_choose(move |index| {
        let Some(app) = ui.upgrade() else { return };
        let chosen = CAST.with(|c| {
            let c = c.borrow();
            let found = c.discovery.as_ref().map(Discovery::found).unwrap_or_default();
            match index {
                0 => Some(None),
                i => found.get(i as usize - 1).cloned().map(Some),
            }
        });
        let Some(chosen) = chosen else { return };
        s.with_player(|p| p.set_renderer(chosen.as_ref()));
        CAST.with(|c| c.borrow_mut().current = chosen);
        // Reopen the item where it is, on the chosen device.
        if s.loaded.borrow().is_some() {
            let at = s.player.borrow().as_ref().and_then(crate::vlc::Player::clock).map(|c| c.time);
            s.start_at.set(at);
            controls::play_current(&app, &s);
        }
        show(&app);
    });
}
