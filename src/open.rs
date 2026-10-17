//! Asks what to open: the system's file and folder dialogs (the XDG portal on
//! Linux), typed network links and saved playlists, and fills the New
//! Playlist dialog. Dialogs run beside the event loop, so the video keeps
//! playing behind them.

use crate::{
    AppWindow, Icons, MenuAction, MenuEntry,
    controls::{self, Session, Shared},
    diag::trace,
    media::{self, MEDIA_EXTENSIONS, Source},
    playlists,
};
use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel, Weak};
use std::{cell::Cell, future::Future, rc::Rc};

#[derive(Clone)]
struct Opener {
    ui: Weak<AppWindow>,
    session: Shared,
    /// One dialog at a time.
    asking: Rc<Cell<bool>>,
}

impl Opener {
    /// Shows a dialog (`pick`) and plays what it returns.
    fn ask(&self, pick: impl Future<Output = Vec<Source>> + 'static) {
        self.ask_then(pick, controls::open);
    }

    /// Shows a dialog (`pick`) and hands what it returns to `then`.
    fn ask_then(
        &self,
        pick: impl Future<Output = Vec<Source>> + 'static,
        then: fn(&AppWindow, &controls::Session, Vec<Source>),
    ) {
        if self.asking.replace(true) {
            return;
        }
        let this = self.clone();
        let task = slint::spawn_local(async move {
            let sources = pick.await;
            this.asking.set(false);
            if let (false, Some(app)) = (sources.is_empty(), this.ui.upgrade()) {
                then(&app, &this.session, sources);
            }
        });
        if let Err(e) = task {
            self.asking.set(false);
            eprintln!("ferricone: open dialog: {e}");
        }
    }
}

/// The most items ⋯ › Open Recent lists.
const RECENT_ITEMS: usize = 10;

/// ⋯ › Open Recent: the last items played that are still there, then Show
/// All in Library.
fn refresh_recent(app: &AppWindow, s: &Session) {
    let items: Vec<Source> = s
        .history
        .borrow()
        .recent()
        .into_iter()
        .map(|(source, _)| source)
        .filter(|src| match src {
            Source::File(p) => p.exists(),
            Source::Location(_) => true,
        })
        .take(RECENT_ITEMS)
        .collect();
    let icons = Icons::get(app);
    let mut entries: Vec<MenuEntry> = items
        .iter()
        .enumerate()
        .map(|(i, src)| MenuEntry {
            action: MenuAction::OpenRecentItem,
            icon: match src {
                Source::File(p) if media::is_audio(p) => icons.get_songs(),
                Source::File(_) => icons.get_video_two_tone(),
                Source::Location(_) => icons.get_streaming_two_tone(),
            },
            label: src.title().into(),
            index: i as i32,
            ..Default::default()
        })
        .collect();
    if entries.is_empty() {
        entries.push(MenuEntry {
            action: MenuAction::None,
            icon: icons.get_recent_two_tone(),
            label: "Nothing played yet".into(),
            disabled: true,
            ..Default::default()
        });
    }
    entries.push(MenuEntry {
        action: MenuAction::ShowRecent,
        icon: icons.get_recent_two_tone(),
        label: "Show All in Library".into(),
        divider_before: true,
        ..Default::default()
    });
    *s.recent.borrow_mut() = items;
    app.set_recent_entries(ModelRc::new(VecModel::from(entries)));
}

pub fn connect(app: &AppWindow, session: &Shared) {
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_refresh_recent(move || {
        if let Some(app) = ui.upgrade() {
            refresh_recent(&app, &s);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_open_recent_item(move |index| {
        let item = s.recent.borrow().get(index as usize).cloned();
        if let (Some(app), Some(item)) = (ui.upgrade(), item) {
            controls::open(&app, &s, vec![item]);
        }
    });
    // About Ferricone.
    let ui = app.as_weak();
    app.on_show_about(move || {
        if let Some(app) = ui.upgrade() {
            app.set_about_version(env!("CARGO_PKG_VERSION").into());
            app.set_about_engine(crate::vlc::version().into());
            app.set_about_open(true);
        }
    });
    app.on_open_link(|url| {
        if let Err(e) = std::process::Command::new("xdg-open").arg(url.as_str()).spawn() {
            eprintln!("ferricone: couldn't open {url}: {e}");
        }
    });
    let opener = Opener {
        ui: app.as_weak(),
        session: session.clone(),
        asking: Rc::default(),
    };

    let o = opener.clone();
    app.on_open_files(move || {
        o.ask(async {
            let files = rfd::AsyncFileDialog::new()
                .set_title("Open File(s)")
                .add_filter("Media files", MEDIA_EXTENSIONS)
                .add_filter("Playlists", playlists::EXTENSIONS)
                .add_filter("All files", &["*"])
                .pick_files()
                .await;
            files
                .unwrap_or_default()
                .into_iter()
                .map(|f| Source::File(f.path().into()))
                .collect()
        });
    });

    let o = opener.clone();
    app.on_open_folder(move || {
        o.ask(async {
            let folder = rfd::AsyncFileDialog::new()
                .set_title("Open Folder")
                .pick_folder()
                .await;
            folder
                .map(|f| Source::File(f.path().into()))
                .into_iter()
                .collect()
        });
    });

    // The queue panel's add: files after what is queued.
    let o = opener.clone();
    app.on_add_items(move || {
        o.ask_then(pick_media("Add to Queue"), controls::append);
    });

    let o = opener.clone();
    app.on_open_locations(move |text| {
        if let Some(app) = o.ui.upgrade() {
            controls::open(&app, &o.session, media::from_lines(&text));
        }
    });

    // Playlists: the menu's list, opening one, and the New Playlist dialog.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_refresh_playlists(move || {
        if let Some(app) = ui.upgrade() {
            refresh_playlists(&app, &s);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_open_playlist(move |index| {
        let path = s
            .playlists
            .borrow()
            .get(index as usize)
            .map(|p| p.path.clone());
        if let (Some(app), Some(path)) = (ui.upgrade(), path) {
            controls::open(&app, &s, vec![Source::File(path)]);
        }
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_new_playlist(move |from_queue| {
        let Some(app) = ui.upgrade() else { return };
        *s.draft.borrow_mut() = if from_queue {
            s.queue.borrow().items().to_vec()
        } else {
            Vec::new()
        };
        *s.draft_cover.borrow_mut() = None;
        app.set_playlist_cover(slint::Image::default());
        show_draft(&app, &s);
        app.set_playlist_dialog_open(true);
    });

    let o = opener.clone();
    app.on_playlist_add_files(move || {
        o.ask_then(pick_media("Add to Playlist"), add_to_draft);
    });

    let o = opener;
    app.on_playlist_pick_cover(move || {
        o.ask_then(
            async {
                let image = rfd::AsyncFileDialog::new()
                    .set_title("Playlist Cover")
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                    .await;
                image
                    .map(|f| Source::File(f.path().into()))
                    .into_iter()
                    .collect()
            },
            set_cover,
        );
    });

    let (ui, s) = (app.as_weak(), session.clone());
    app.on_save_playlist(move |name| {
        let cover = s.draft_cover.borrow().clone();
        match playlists::save(&name, &s.draft.borrow(), cover.as_deref()) {
            Ok(path) => {
                trace!("saved playlist {}", path.display());
                if let Some(app) = ui.upgrade() {
                    refresh_playlists(&app, &s);
                    crate::library::refresh(&app, &s);
                }
                true
            }
            Err(e) => {
                eprintln!("ferricone: saving playlist {name}: {e}");
                false
            }
        }
    });
}

/// The file picker for media, playlists included.
async fn pick_media(title: &'static str) -> Vec<Source> {
    let files = rfd::AsyncFileDialog::new()
        .set_title(title)
        .add_filter("Media files", MEDIA_EXTENSIONS)
        .add_filter("Playlists", playlists::EXTENSIONS)
        .add_filter("All files", &["*"])
        .pick_files()
        .await;
    files
        .unwrap_or_default()
        .into_iter()
        .map(|f| Source::File(f.path().into()))
        .collect()
}

/// Adds files (folders and playlists expanded) to the New Playlist draft.
pub fn add_to_draft(app: &AppWindow, s: &Session, sources: Vec<Source>) {
    s.draft.borrow_mut().extend(media::expand(sources));
    show_draft(app, s);
}

fn show_draft(app: &AppWindow, s: &Session) {
    let names: Vec<SharedString> = s.draft.borrow().iter().map(|i| i.title().into()).collect();
    app.set_playlist_files(ModelRc::new(VecModel::from(names)));
}

fn set_cover(app: &AppWindow, s: &Session, picked: Vec<Source>) {
    let Some(Source::File(path)) = picked.into_iter().next() else {
        return;
    };
    match slint::Image::load_from_path(&path) {
        Ok(image) => {
            app.set_playlist_cover(image);
            *s.draft_cover.borrow_mut() = Some(path);
        }
        Err(e) => eprintln!("ferricone: playlist cover {}: {e:?}", path.display()),
    }
}

/// The right-click menu's Playlist submenu: the queue, New Playlist, Save
/// Queue as Playlist, then the saved playlists.
pub fn refresh_playlists(app: &AppWindow, s: &Session) {
    let lists = playlists::list();
    let icons = Icons::get(app);
    let entry = |action, icon, label: &str| MenuEntry {
        action,
        icon,
        label: label.into(),
        ..Default::default()
    };
    let mut entries = vec![
        entry(
            MenuAction::ShowQueue,
            icons.get_queue_two_tone(),
            "Show Queue",
        ),
        MenuEntry {
            divider_before: true,
            ..entry(MenuAction::NewPlaylist, icons.get_add(), "New Playlist…")
        },
        MenuEntry {
            disabled: s.queue.borrow().items().is_empty(),
            ..entry(
                MenuAction::SaveQueue,
                icons.get_save_two_tone(),
                "Save Queue as Playlist…",
            )
        },
    ];
    for (i, list) in lists.iter().enumerate() {
        entries.push(MenuEntry {
            divider_before: i == 0,
            index: i as i32,
            ..entry(
                MenuAction::OpenPlaylist,
                icons.get_playlist_two_tone(),
                &list.name,
            )
        });
    }
    *s.playlists.borrow_mut() = lists;
    app.set_playlist_entries(ModelRc::new(VecModel::from(entries)));
}
