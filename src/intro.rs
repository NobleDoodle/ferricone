//! The introduction's Rust side (ui/onboarding.slint): the media folder it
//! offers and Browse, and, on Finish, remembering it was seen and adding the
//! folder to Videos. It opens by itself until it has been finished once.

use crate::{
    AppWindow,
    controls::{Session, Shared},
    library, prefs,
};
use slint::ComponentHandle;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

/// `~/…` for paths in the home folder.
fn shown(path: &std::path::Path) -> String {
    let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn connect(app: &AppWindow, session: &Shared) {
    let folder: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(library::first_video_folder(session)));
    app.set_intro_folder(folder.borrow().as_deref().map(shown).unwrap_or_default().into());

    let (ui, chosen) = (app.as_weak(), folder.clone());
    app.on_intro_browse(move || {
        let (ui, chosen) = (ui.clone(), chosen.clone());
        let task = slint::spawn_local(async move {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Choose a Media Folder")
                .pick_folder()
                .await;
            if let (Some(picked), Some(app)) = (picked, ui.upgrade()) {
                let dir = picked.path().to_path_buf();
                app.set_intro_folder(shown(&dir).into());
                *chosen.borrow_mut() = Some(dir);
            }
        });
        if let Err(e) = task {
            eprintln!("ferricone: folder dialog: {e}");
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_intro_finish(move || {
        let Some(app) = ui.upgrade() else { return };
        prefs::update(|p| p.onboarded = true);
        if !app.get_intro_only()
            && let Some(dir) = folder.borrow().clone()
        {
            add_folder(&app, &s, dir);
        }
    });

    if !prefs::get().onboarded {
        app.invoke_show_intro(0, false);
    }
}

fn add_folder(app: &AppWindow, s: &Session, dir: PathBuf) {
    if library::first_video_folder(s).as_deref() != Some(dir.as_path()) {
        library::add_video_folder(app, s, dir);
    }
}
