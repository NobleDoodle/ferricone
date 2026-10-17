//! Ferricone Player: plays media through libVLC 4 in a Slint window.
//!
//! `ferricone [file, folder or URL]…` plays them in order; with none, the
//! window waits for something to be opened or dropped.

mod addsubs;
mod art;
mod cast;
mod audio_fx;
mod chrome;
mod controls;
mod diag;
mod drop;
mod intro;
mod library;
mod media;
mod music;
mod open;
mod opensubtitles;
mod pickers;
mod playlists;
mod preferences;
mod prefs;
mod presets;
mod queue;
mod remote;
mod seekbar;
mod settings;
mod shortcuts;
#[cfg(test)]
mod snapshot;
mod state;
mod subtimeline;
mod theme;
mod thumbs;
mod video;
mod video_fx;
mod vlc;
mod waveform;

use media::Source;
use slint::{ComponentHandle, GraphicsAPI, RenderingState};
use std::{rc::Rc, sync::Arc};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `ferricone --version` also shows that the bundled libVLC loads.
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("Ferricone Player {} (libVLC {})", env!("CARGO_PKG_VERSION"), vlc::version());
            return Ok(());
        }
        Some("--help" | "-h") => {
            println!(
                "Usage: ferricone [FILE | FOLDER | URL]...\n\nPlays the files, folders (their media) and network addresses given,\nor opens the player empty. --version shows the version."
            );
            return Ok(());
        }
        _ => {}
    }
    let sources: Vec<Source> = std::env::args_os()
        .skip(1)
        .filter_map(|arg| match arg.to_str() {
            Some(text) => Source::parse(text),
            None => Some(Source::File(arg.into())),
        })
        .collect();

    // libVLC's OpenGL output needs desktop GL; Slint picks GLES 2.0 by default.
    slint::BackendSelector::new().require_opengl().select()?;

    let app = AppWindow::new()?;
    theme::start(&app);
    chrome::connect(&app);

    let vlc = Rc::new(vlc::Vlc::new(&diag::vlc_args())?);
    let ui = app.as_weak();
    let session = controls::Session::new(vlc.clone());
    session.extras.start(&app);
    controls::connect(&app, &session);
    queue::connect(&app, &session);
    settings::connect(&app, &session);
    pickers::connect(&app);
    library::connect(&app, &session);
    library::start(&app, &session);
    intro::connect(&app, &session);
    preferences::connect(&app, &session);
    shortcuts::connect(&app);
    addsubs::connect(&app, &session);
    cast::connect(&app, &session, vlc.clone());
    subtimeline::connect(&app, &session);
    queue::start(&app, &session, &vlc);
    open::connect(&app, &session);
    let drops = drop::connect(&app, {
        let (ui, session) = (ui.clone(), session.clone());
        move |sources| {
            let Some(app) = ui.upgrade() else { return };
            // Into the New Playlist dialog's file box while it is open.
            if app.get_playlist_dialog_open() {
                open::add_to_draft(&app, &session, sources);
            } else {
                controls::open(&app, &session, sources);
            }
        }
    });
    if !sources.is_empty() {
        // Shows the title now; playback starts once the player exists.
        controls::open(&app, &session, sources);
    }
    remote::start({
        let ui = ui.clone();
        move |command| {
            let _ = ui.upgrade_in_event_loop(move |app| controls::remote_command(&app, command));
        }
    });
    let diag = diag::Diagnostics::from_env(&session);
    diag::open_panel(&app);

    app.window()
        .set_rendering_notifier(move |state, api| match state {
            RenderingState::RenderingSetup => {
                let GraphicsAPI::NativeOpenGL { get_proc_address } = api else {
                    return eprintln!("ferricone: Slint is not rendering with OpenGL");
                };
                let redraw = {
                    let ui = ui.clone();
                    move || {
                        let ui = ui.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui.upgrade() {
                                ui.window().request_redraw();
                            }
                        });
                    }
                };
                let Some(app) = ui.upgrade() else { return };
                let events = controls::event_sink(&app);
                match video::Bridge::new(get_proc_address, redraw)
                    .and_then(|bridge| vlc.player(Arc::new(bridge), events))
                {
                    Ok(p) => {
                        remote::set_clock(p.clock_source());
                        *session.player.borrow_mut() = Some(p);
                        controls::play_current(&app, &session);
                    }
                    Err(e) => eprintln!("ferricone: {e}"),
                }
            }
            RenderingState::BeforeRendering => {
                let (Some(ui), Some(p)) = (ui.upgrade(), &*session.player.borrow()) else {
                    return;
                };
                if let Some(frame) = p.video().take_frame() {
                    diag.frame_shown();
                    ui.set_video_frame(frame);
                }
                // Blurred only while a frosted overlay is over the video.
                if ui.get_wants_backdrop()
                    && let Some(backdrop) = p.video().backdrop(ui.get_video_aspect())
                {
                    ui.set_video_backdrop(backdrop);
                }
            }
            RenderingState::RenderingTeardown => {
                // The window is closing: keep the place for next time.
                session.remember();
                if let Some(p) = session.player.borrow_mut().take() {
                    p.video().release_ui_gl();
                }
            }
            _ => {}
        })?;

    let result = app.run();
    drop(drops);
    Ok(result?)
}
