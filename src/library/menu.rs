//! The library's menus: right-clicking a row (Figma `file context menu`,
//! `playlist context menu`; folders get the parts of the file menu that
//! apply) and the ⋯ of a page (Play All, Shuffle, Add to Queue, Add to
//! Playlist, and, after Figma's `folder context menu`, Add File / Add Folder).
//! Entries for what later phases build (streaming, converting, favorites,
//! track editing, properties) show dimmed, as in the ⋯ menu.

use super::{Library, Place, Section, Target, add_folder, refresh};
use crate::{
    AppWindow, Icons, MenuAction, MenuEntry,
    controls::{self, Session, Shared},
    media::{self, MEDIA_EXTENSIONS, Source},
    playlists,
};
use slint::{ComponentHandle, Global, ModelRc, VecModel};
use std::path::{Path, PathBuf};

/// What the open menu is for.
#[derive(Clone, Debug)]
pub(super) enum MenuFor {
    Row(usize),
    Page,
}

/// A row as the menus see it.
enum Kind {
    /// A file, its folder's (or list's) items, and its place among them.
    File(Source, Vec<Source>, usize),
    Folder(PathBuf),
    /// A saved playlist or stream list.
    List(PathBuf),
    Remote(String),
}

fn kind(lib: &Library, index: usize) -> Option<Kind> {
    match lib.targets.borrow().get(index)?.as_ref()? {
        Target::Play(items, at) => Some(Kind::File(items.get(*at)?.clone(), items.clone(), *at)),
        Target::Go(Place::Section(_, Some(dir)) | Place::Folder(dir)) => Some(Kind::Folder(dir.clone())),
        Target::Go(Place::Playlist(path) | Place::Streams(path)) => Some(Kind::List(path.clone())),
        Target::Go(Place::Remote(url)) => Some(Kind::Remote(url.clone())),
        _ => None,
    }
}

impl Kind {
    fn sources(&self) -> Vec<Source> {
        match self {
            Kind::File(source, ..) => vec![source.clone()],
            Kind::Folder(dir) => media::expand([Source::File(dir.clone())]),
            Kind::List(path) => playlists::load(path),
            Kind::Remote(url) => vec![Source::Location(url.clone())],
        }
    }
}

/// Every item a page plays, in its order.
fn page_items(lib: &Library) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    for target in lib.targets.borrow().iter().flatten() {
        if let Target::Play(items, at) = target
            && let Some(item) = items.get(*at)
            && !out.contains(item)
        {
            out.push(item.clone());
        }
    }
    out
}

/// The folder a page shows, if it is one.
fn page_folder(lib: &Library) -> Option<PathBuf> {
    match &*lib.at.borrow() {
        Place::Section(_, Some(dir)) | Place::Folder(dir) => Some(dir.clone()),
        Place::Section(section, None) => {
            let roots = lib.roots.borrow();
            match roots.of(*section) {
                [only] => Some(only.clone()),
                _ => None,
            }
        }
        Place::Songs => lib.roots.borrow().music.first().cloned(),
        _ => None,
    }
}

pub(super) fn connect(app: &AppWindow, session: &Shared) {
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_row_menu(move |index| {
        let Some(app) = ui.upgrade() else { return false };
        let Some(kind) = kind(&s.library, index as usize) else {
            return false;
        };
        *s.library.menu_for.borrow_mut() = Some(MenuFor::Row(index as usize));
        app.set_lib_menu_entries(ModelRc::new(VecModel::from(row_entries(&app, &s, &kind))));
        app.set_lib_playlist_entries(ModelRc::new(VecModel::from(playlist_entries(&app))));
        true
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_page_menu(move || {
        let Some(app) = ui.upgrade() else { return };
        *s.library.menu_for.borrow_mut() = Some(MenuFor::Page);
        app.set_lib_menu_entries(ModelRc::new(VecModel::from(page_entries(&app, &s))));
        app.set_lib_playlist_entries(ModelRc::new(VecModel::from(playlist_entries(&app))));
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_menu_action(move |action, index| {
        if let Some(app) = ui.upgrade() {
            act(&app, &s, action, index);
        }
    });
}

fn entry(action: MenuAction, icon: crate::IconLayers, label: &str) -> MenuEntry {
    MenuEntry {
        action,
        icon,
        label: label.into(),
        ..Default::default()
    }
}

fn later(mut e: MenuEntry) -> MenuEntry {
    e.disabled = true;
    e
}

fn divided(mut e: MenuEntry) -> MenuEntry {
    e.divider_before = true;
    e
}

fn submenu(mut e: MenuEntry) -> MenuEntry {
    e.submenu = true;
    e
}

fn row_entries(app: &AppWindow, s: &Session, kind: &Kind) -> Vec<MenuEntry> {
    let i = Icons::get(app);
    let add_to_playlist = submenu(entry(MenuAction::AddToPlaylist, i.get_playlist_two_tone(), "Add to Playlist"));
    match kind {
        Kind::File(source, ..) => {
            let watched = s.history.borrow().get(source).is_some_and(|p| p.watched);
            let mut out = vec![
                entry(MenuAction::Play, i.get_playback_two_tone(), "Play"),
                if watched {
                    entry(MenuAction::MarkUnwatched, i.get_check_selected_two_tone(), "Mark as Unwatched")
                } else {
                    entry(MenuAction::MarkWatched, i.get_check_selected_two_tone(), "Mark as Watched")
                },
                later(entry(MenuAction::Favorite, i.get_favorite_two_tone(), "Favorite")),
                later(entry(MenuAction::Stream, i.get_streaming_two_tone(), "Stream")),
                later(entry(MenuAction::Convert, i.get_swap_two_tone(), "Convert")),
                divided(entry(MenuAction::AddToQueue, i.get_queue_two_tone(), "Add to Queue")),
                add_to_playlist,
            ];
            if let Source::File(path) = source {
                out.push(divided(entry(MenuAction::ShowInFiles, i.get_folder_two_tone(), "Show in File Explorer")));
                // Away from the file's own folder: go there.
                let here = page_folder(&s.library);
                if here.as_deref() != path.parent() {
                    out.push(entry(MenuAction::GoToSource, i.get_link_two_tone(), "Go to Source"));
                }
            }
            out.extend([
                divided(subtitles_entry(source, &i)),
                later(entry(MenuAction::AddAudio, i.get_audio_two_tone(), "Add Audio Tracks")),
                later(entry(MenuAction::Properties, i.get_info_two_tone(), "Properties & Metadata")),
            ]);
            out
        }
        Kind::Folder(_) => vec![
            entry(MenuAction::Play, i.get_playback_two_tone(), "Play"),
            entry(MenuAction::Shuffle, i.get_shuffle_right_two_tone(), "Shuffle"),
            entry(MenuAction::MarkWatched, i.get_check_selected_two_tone(), "Mark All as Watched"),
            entry(MenuAction::MarkUnwatched, i.get_check_selected_two_tone(), "Mark All as Unwatched"),
            divided(entry(MenuAction::AddToQueue, i.get_queue_two_tone(), "Add to Queue")),
            add_to_playlist,
            divided(entry(MenuAction::ShowInFiles, i.get_folder_two_tone(), "Show in File Explorer")),
            entry(MenuAction::AddSubtitles, i.get_subtitles_two_tone(), "Add Subtitles"),
        ],
        Kind::List(_) => vec![
            entry(MenuAction::Play, i.get_playback_two_tone(), "Play"),
            later(entry(MenuAction::Favorite, i.get_favorite_two_tone(), "Favorite")),
            entry(MenuAction::Shuffle, i.get_shuffle_right_two_tone(), "Shuffle"),
            entry(MenuAction::ExportPlaylist, i.get_upload_up_two_tone(), "Export Playlist"),
            divided(entry(MenuAction::AddToQueue, i.get_queue_two_tone(), "Add to Queue")),
            divided(later(entry(MenuAction::AddSubtitles, i.get_subtitles_two_tone(), "Add Subtitle Tracks"))),
            later(entry(MenuAction::AddAudio, i.get_audio_two_tone(), "Add Audio Tracks")),
            later(entry(MenuAction::Properties, i.get_info_two_tone(), "Properties & Metadata")),
        ],
        Kind::Remote(_) => vec![
            entry(MenuAction::Play, i.get_playback_two_tone(), "Play"),
            entry(MenuAction::AddToQueue, i.get_queue_two_tone(), "Add to Queue"),
        ],
    }
}

/// Add Subtitle Tracks: for files only (not streams).
fn subtitles_entry(source: &Source, i: &Icons) -> MenuEntry {
    let e = entry(MenuAction::AddSubtitles, i.get_subtitles_two_tone(), "Add Subtitle Tracks");
    match source {
        Source::File(path) if !media::is_audio(path) => e,
        _ => later(e),
    }
}

fn page_entries(app: &AppWindow, s: &Session) -> Vec<MenuEntry> {
    let i = Icons::get(app);
    let lib = &s.library;
    let none = page_items(lib).is_empty();
    let dim = |mut e: MenuEntry| {
        e.disabled = none;
        e
    };
    let mut out = vec![
        dim(entry(MenuAction::PlayAll, i.get_playback_two_tone(), "Play All")),
        dim(entry(MenuAction::Shuffle, i.get_shuffle_right_two_tone(), "Shuffle")),
        dim(divided(entry(MenuAction::AddToQueue, i.get_queue_two_tone(), "Add to Queue"))),
        dim(submenu(entry(MenuAction::AddToPlaylist, i.get_playlist_two_tone(), "Add to Playlist"))),
    ];
    let at = lib.at.borrow().clone();
    match at {
        Place::Playlist(_) => out.extend([
            divided(entry(MenuAction::AddFile, i.get_file_two_tone(), "Add File")),
            entry(MenuAction::AddFolder, i.get_new_folder_two_tone(), "Add Folder"),
        ]),
        Place::Section(_, None) | Place::Songs => {
            out.push(divided(entry(MenuAction::AddFolder, i.get_new_folder_two_tone(), "Add Folder")));
        }
        _ => {}
    }
    if page_folder(lib).is_some() {
        out.push(divided(entry(MenuAction::ShowInFiles, i.get_folder_two_tone(), "Show in File Explorer")));
    }
    out
}

/// The Add to Playlist submenu: a new one, then the saved ones (`index`).
fn playlist_entries(app: &AppWindow) -> Vec<MenuEntry> {
    let i = Icons::get(app);
    let mut out = vec![MenuEntry {
        index: -1,
        ..entry(MenuAction::ToPlaylist, i.get_add(), "New Playlist…")
    }];
    out.extend(playlists::list().into_iter().enumerate().map(|(n, list)| MenuEntry {
        index: n as i32,
        divider_before: n == 0,
        ..entry(MenuAction::ToPlaylist, i.get_playlist_two_tone(), &list.name)
    }));
    out
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

fn act(app: &AppWindow, s: &Shared, action: MenuAction, index: i32) {
    let lib = &s.library;
    let menu_for = lib.menu_for.borrow().clone();
    let row = match menu_for {
        Some(MenuFor::Row(i)) => kind(lib, i),
        _ => None,
    };
    let sources = match &row {
        Some(kind) => kind.sources(),
        None => page_items(lib),
    };
    match action {
        MenuAction::Play => match row {
            Some(Kind::File(_, items, at)) => {
                s.queue.borrow_mut().replace(items);
                s.queue.borrow_mut().play_at(at);
                controls::play_current(app, s);
            }
            _ => controls::open(app, s, sources),
        },
        MenuAction::PlayAll => controls::open(app, s, sources),
        MenuAction::Shuffle => {
            let mut sources = media::expand(sources);
            media::shuffle(&mut sources, seed());
            controls::open(app, s, sources);
        }
        MenuAction::AddToQueue => controls::append(app, s, sources),
        MenuAction::ToPlaylist => {
            let sources = media::expand(sources);
            if index < 0 {
                app.invoke_new_playlist(false);
                crate::open::add_to_draft(app, s, sources);
            } else if let Some(list) = playlists::list().get(index as usize) {
                append_to(&list.path, sources);
                refresh(app, s);
            }
        }
        MenuAction::MarkWatched | MenuAction::MarkUnwatched => {
            let files: Vec<Source> = media::expand(sources)
                .into_iter()
                .filter(|src| matches!(src, Source::File(p) if !media::is_audio(p)))
                .collect();
            s.history
                .borrow_mut()
                .set_watched(&files, action == MenuAction::MarkWatched);
            refresh(app, s);
        }
        MenuAction::ShowInFiles => match row {
            Some(Kind::File(Source::File(path), ..)) => show_in_files(&path),
            Some(Kind::Folder(dir)) => show_in_files(&dir),
            _ => {
                if let Some(dir) = page_folder(lib) {
                    show_in_files(&dir);
                }
            }
        },
        MenuAction::GoToSource => {
            if let Some(Kind::File(Source::File(path), ..)) = row
                && let Some(dir) = path.parent()
            {
                lib.go(lib.place_of(dir));
                if !app.get_lib_full() {
                    app.set_dock_open(true);
                }
                refresh(app, s);
            }
        }
        MenuAction::AddSubtitles => match row {
            Some(Kind::File(Source::File(path), ..)) => crate::addsubs::open(app, &path, false),
            Some(Kind::Folder(dir)) => {
                // The folder's first video stands for it.
                let first = media::expand([Source::File(dir)])
                    .into_iter()
                    .find_map(|s| match s {
                        Source::File(p) if !media::is_audio(&p) => Some(p),
                        _ => None,
                    });
                if let Some(first) = first {
                    crate::addsubs::open(app, &first, true);
                }
            }
            _ => {}
        },
        MenuAction::ExportPlaylist => {
            if let Some(Kind::List(path)) = row {
                export(path);
            }
        }
        MenuAction::AddFile => {
            if let Place::Playlist(path) = lib.at.borrow().clone() {
                pick_into(app, s, path, false);
            }
        }
        MenuAction::AddFolder => match lib.at.borrow().clone() {
            Place::Playlist(path) => pick_into(app, s, path, true),
            Place::Section(Section::Music, _) | Place::Songs => add_folder(app, s, Section::Music),
            _ => add_folder(app, s, Section::Videos),
        },
        _ => {}
    }
}

/// Adds `items` to the end of the playlist (or stream list) at `path`.
fn append_to(path: &Path, items: Vec<Source>) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_stem()) else {
        return;
    };
    let mut all = playlists::load(path);
    all.extend(items);
    if let Err(e) = playlists::save_in(dir, &name.to_string_lossy(), &all, None) {
        eprintln!("ferricone: adding to {}: {e}", path.display());
    }
}

/// Asks for files (or a folder) and adds them to the playlist at `path`.
fn pick_into(app: &AppWindow, s: &Shared, path: PathBuf, folder: bool) {
    let (ui, s) = (app.as_weak(), s.clone());
    let task = slint::spawn_local(async move {
        let dialog = rfd::AsyncFileDialog::new();
        let picked: Vec<Source> = if folder {
            let folder = dialog.set_title("Add a Folder to the Playlist").pick_folder().await;
            folder.map(|f| Source::File(f.path().into())).into_iter().collect()
        } else {
            let files = dialog
                .set_title("Add Files to the Playlist")
                .add_filter("Media files", MEDIA_EXTENSIONS)
                .pick_files()
                .await;
            files
                .unwrap_or_default()
                .into_iter()
                .map(|f| Source::File(f.path().into()))
                .collect()
        };
        if picked.is_empty() {
            return;
        }
        append_to(&path, media::expand(picked));
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });
    if let Err(e) = task {
        eprintln!("ferricone: file dialog: {e}");
    }
}

/// Saves a copy of the playlist where the user picks.
fn export(path: PathBuf) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let task = slint::spawn_local(async move {
        let target = rfd::AsyncFileDialog::new()
            .set_title("Export Playlist")
            .set_file_name(name.unwrap_or_default())
            .add_filter("Playlist", playlists::EXTENSIONS)
            .save_file()
            .await;
        if let Some(target) = target
            && let Err(e) = std::fs::copy(&path, target.path())
        {
            eprintln!("ferricone: exporting {}: {e}", path.display());
        }
    });
    if let Err(e) = task {
        eprintln!("ferricone: save dialog: {e}");
    }
}

/// Shows `path` in the file manager: selected in its folder through
/// org.freedesktop.FileManager1 where there is one, else its folder (or
/// itself, for a folder) opened.
fn show_in_files(path: &Path) {
    #[cfg(target_os = "linux")]
    {
        let uri = media::file_uri(path);
        let shown = dbus::blocking::Connection::new_session().ok().is_some_and(|conn| {
            let proxy = conn.with_proxy(
                "org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                std::time::Duration::from_secs(2),
            );
            let method = if path.is_dir() { "ShowFolders" } else { "ShowItems" };
            let result: Result<(), _> =
                proxy.method_call("org.freedesktop.FileManager1", method, (vec![uri], ""));
            result.is_ok()
        });
        if !shown {
            let folder = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
            if let Err(e) = std::process::Command::new("xdg-open").arg(folder).spawn() {
                eprintln!("ferricone: show in file manager: {e}");
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(
            media::file_uri(Path::new("/m/Test Show S01/E01 #1.mkv")),
            "file:///m/Test%20Show%20S01/E01%20%231.mkv"
        );
    }
}
