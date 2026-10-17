//! The library: Home, Videos, Music, Network, Playlists and Recent, browsed
//! in the dock beside the video (the title bar's ◧, or its folder crumb for
//! the loaded item's folder).
//!
//! Videos and Music are folders (`library.toml` in the config folder; the
//! system's Videos and Music folders until one is added). Watched progress,
//! Recent and Home come from the play history (`state::History`), the
//! thumbnails from `thumbs`.

use crate::{
    AppWindow, Icons, LibRow, LibSection,
    controls::{self, Session, Shared},
    media::{self, Source, natural_cmp},
    music::{MusicIndex, Tag},
    playlists,
};
use serde::{Deserialize, Serialize};

mod menu;
use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
};

/// A remote playlist that takes longer than this to read plays as a stream.
const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// The most rows shown at once (an IPTV list has thousands); search
/// reaches the rest.
const MAX_ROWS: usize = 1000;
/// Rows per Home section.
const HOME_ROWS: usize = 3;
/// Rows on the Recent page.
const RECENT_ROWS: usize = 100;
/// A breadcrumb longer than this collapses to `… ▶ parent ▶ folder`.
const MAX_CRUMBS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Videos,
    Music,
}

/// Where the library is.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Home,
    /// A section's top (None) or a folder inside one of its roots.
    Section(Section, Option<PathBuf>),
    /// A folder outside the library's roots (the title bar's crumb).
    Folder(PathBuf),
    Network,
    /// A saved list of streams.
    Streams(PathBuf),
    /// A remote playlist (an M3U link), read by libVLC.
    Remote(String),
    Playlists,
    Playlist(PathBuf),
    Recent,
    /// Music's folders.
    Songs,
    /// Music by album, artist or genre: the values, or one's tracks.
    Tag(Tag, Option<String>),
}

impl Place {
    /// The rail's entry: 0 Home, 1 Videos, 2 Music, 3 Network, 4 Playlists,
    /// 5 Recent.
    fn page(&self) -> i32 {
        match self {
            Place::Home => 0,
            Place::Section(Section::Videos, _) | Place::Folder(_) => 1,
            Place::Section(Section::Music, _) | Place::Songs | Place::Tag(..) => 2,
            Place::Network | Place::Streams(_) | Place::Remote(_) => 3,
            Place::Playlists | Place::Playlist(_) => 4,
            Place::Recent => 5,
        }
    }

    /// Inside a section rather than at its top.
    fn deep(&self) -> bool {
        matches!(
            self,
            Place::Section(_, Some(_))
                | Place::Folder(_)
                | Place::Playlist(_)
                | Place::Streams(_)
                | Place::Remote(_)
                | Place::Songs
                | Place::Tag(..)
        )
    }
}

/// What a row does when clicked.
#[derive(Clone, Debug)]
enum Target {
    Go(Place),
    /// Plays `items` from `at`.
    Play(Vec<Source>, usize),
    AddFolder(Section),
    NewPlaylist,
    AddStream,
}

/// How folders are sorted (the Sorting Options popover). Subfolders come
/// first, by name, except under Title Z–A.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    /// The next episode on top; music keeps its name order.
    #[default]
    UnwatchedFirst,
    WatchedFirst,
    /// By the file's modification time.
    NewestFirst,
    OldestFirst,
    TitleAz,
    TitleZa,
}

impl Sort {
    pub const ALL: [Sort; 6] = [
        Sort::UnwatchedFirst,
        Sort::WatchedFirst,
        Sort::NewestFirst,
        Sort::OldestFirst,
        Sort::TitleAz,
        Sort::TitleZa,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sort::UnwatchedFirst => "Unwatched First",
            Sort::WatchedFirst => "Watched First",
            Sort::NewestFirst => "Newest First",
            Sort::OldestFirst => "Oldest First",
            Sort::TitleAz => "Title A–Z",
            Sort::TitleZa => "Title Z–A",
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Serialize)]
struct Roots {
    #[serde(default)]
    videos: Vec<PathBuf>,
    #[serde(default)]
    music: Vec<PathBuf>,
    #[serde(default)]
    sort: Sort,
    /// The full library shows cards rather than rows.
    #[serde(default = "yes")]
    grid: bool,
}

impl Default for Roots {
    /// The system's Videos and Music folders.
    fn default() -> Self {
        let user = directories::UserDirs::new();
        let pick = |dir: Option<&Path>| dir.filter(|d| d.is_dir()).map(Path::to_path_buf);
        Self {
            videos: pick(user.as_ref().and_then(|u| u.video_dir()))
                .into_iter()
                .collect(),
            music: pick(user.as_ref().and_then(|u| u.audio_dir()))
                .into_iter()
                .collect(),
            sort: Sort::default(),
            grid: true,
        }
    }
}

impl Roots {
    fn of(&self, section: Section) -> &[PathBuf] {
        match section {
            Section::Videos => &self.videos,
            Section::Music => &self.music,
        }
    }

    /// The section and root `dir` is in, if any.
    fn find(&self, dir: &Path) -> Option<(Section, &Path)> {
        [Section::Videos, Section::Music]
            .into_iter()
            .flat_map(|s| self.of(s).iter().map(move |r| (s, r)))
            .find(|(_, root)| dir.starts_with(root))
            .map(|(s, r)| (s, r.as_path()))
    }
}

impl Roots {
    /// Where `dir` is shown: in its section when under a root.
    fn place_of(&self, dir: &Path) -> Place {
        match self.find(dir) {
            // Music's top has its categories; its one root is Songs.
            Some((Section::Music, root)) if dir == root && self.music.len() == 1 => Place::Songs,
            Some((section, root)) if dir == root && self.of(section).len() == 1 => {
                Place::Section(section, None)
            }
            Some((section, _)) => Place::Section(section, Some(dir.into())),
            None => Place::Folder(dir.into()),
        }
    }
}

fn roots_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
        .map(|d| d.config_dir().join("library.toml"))
}

/// A remote playlist's entries, as far as they are known.
enum Remote {
    Loading,
    Read(Vec<(Source, String)>),
    /// Not a playlist (or unreadable): it plays as a stream.
    Failed,
}

/// What the expander's thread hands back.
type Expanded = Arc<Mutex<Vec<(String, Option<Vec<crate::vlc::Entry>>)>>>;

pub struct Library {
    pub thumbs: crate::thumbs::Thumbs,
    vlc: Rc<crate::vlc::Vlc>,
    expander: RefCell<Option<crate::vlc::Expander>>,
    remote: RefCell<HashMap<String, Remote>>,
    expanded: Expanded,
    music: RefCell<MusicIndex>,
    /// What the open right-click or ⋯ menu is for.
    menu_for: RefCell<Option<menu::MenuFor>>,
    roots: RefCell<Roots>,
    at: RefCell<Place>,
    back: RefCell<Vec<Place>>,
    forward: RefCell<Vec<Place>>,
    search: RefCell<String>,
    /// The shown rows' targets, and the breadcrumb's places.
    targets: RefCell<Vec<Option<Target>>>,
    crumbs: RefCell<Vec<Place>>,
}

impl Library {
    pub fn new(vlc: Rc<crate::vlc::Vlc>) -> Self {
        let roots = match roots_path() {
            Some(path) if path.exists() => crate::state::read(Some(&path)),
            _ => Roots::default(),
        };
        Self {
            thumbs: crate::thumbs::Thumbs::new(vlc.clone()),
            vlc,
            expander: RefCell::default(),
            remote: RefCell::default(),
            expanded: Arc::default(),
            music: RefCell::new(MusicIndex::load()),
            menu_for: RefCell::default(),
            roots: RefCell::new(roots),
            at: RefCell::new(Place::Home),
            back: RefCell::default(),
            forward: RefCell::default(),
            search: RefCell::default(),
            targets: RefCell::default(),
            crumbs: RefCell::default(),
        }
    }

    fn go(&self, place: Place) {
        let previous = self.at.replace(place.clone());
        if previous != place {
            self.back.borrow_mut().push(previous);
            self.forward.borrow_mut().clear();
            self.search.borrow_mut().clear();
            self.thumbs.cancel_waiting();
        }
    }

    fn add_root(&self, section: Section, dir: PathBuf) {
        let mut roots = self.roots.borrow_mut();
        let list = match section {
            Section::Videos => &mut roots.videos,
            Section::Music => &mut roots.music,
        };
        if !list.contains(&dir) {
            list.push(dir);
            crate::state::write(roots_path().as_deref(), &*roots);
        }
    }

    /// Where `dir` is shown: in its section when under a root.
    fn place_of(&self, dir: &Path) -> Place {
        self.roots.borrow().place_of(dir)
    }
}

/// Asks for a folder for Videos (or Music) — Settings › Library.
pub fn ask_folder(app: &AppWindow, s: &Shared, music: bool) {
    add_folder(app, s, if music { Section::Music } else { Section::Videos });
}

/// The first video folder, for the introduction to offer.
pub fn first_video_folder(s: &Session) -> Option<PathBuf> {
    s.library.roots.borrow().videos.first().cloned()
}

/// Adds `dir` to Videos (the introduction's media folder).
pub fn add_video_folder(app: &AppWindow, s: &Session, dir: PathBuf) {
    s.library.add_root(Section::Videos, dir);
    refresh(app, s);
}

pub fn start(app: &AppWindow, s: &Session) {
    let ui = app.as_weak();
    s.library.thumbs.start(move || {
        let _ = ui.upgrade_in_event_loop(|app| app.invoke_lib_thumbs_arrived());
    });
    let ui = app.as_weak();
    s.library.music.borrow_mut().start(&s.library.vlc, move || {
        let _ = ui.upgrade_in_event_loop(|app| app.invoke_lib_tags_arrived());
    });
    let (ui, expanded) = (app.as_weak(), s.library.expanded.clone());
    match s.library.vlc.expander(REMOTE_TIMEOUT, move |url, entries| {
        expanded.lock().unwrap().push((url, entries));
        let _ = ui.upgrade_in_event_loop(|app| app.invoke_lib_remote_arrived());
    }) {
        Ok(e) => *s.library.expander.borrow_mut() = Some(e),
        Err(e) => eprintln!("ferricone: remote playlists: {e}"),
    }
}

/// Starts reading `url`'s entries unless that is under way or done.
fn want_remote(lib: &Library, url: &str) {
    if lib.remote.borrow().contains_key(url) {
        return;
    }
    let started = lib
        .expander
        .borrow()
        .as_ref()
        .map(|e| e.expand(url));
    let state = match started {
        Some(Ok(())) => Remote::Loading,
        _ => Remote::Failed,
    };
    lib.remote.borrow_mut().insert(url.into(), state);
}

/// Links that may be playlists rather than streams; HLS's `.m3u8` can be
/// either, so the expander decides.
fn may_be_playlist(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    [".m3u", ".m3u8", ".pls", ".xspf"].iter().any(|e| path.ends_with(e))
}

/// Saves typed links as a stream list named `name` (or after the first
/// link); returns where it went.
fn save_streams(links: &str, name: &str) -> Result<PathBuf, String> {
    let items = media::from_lines(links);
    let first = items.first().ok_or("there are no links")?;
    let name = match name.trim() {
        "" => first.title(),
        name => name.to_string(),
    };
    let dir = playlists::streams_folder().ok_or("there is no data folder")?;
    playlists::save_in(&dir, &name, &items, None)
}

pub fn connect(app: &AppWindow, session: &Shared) {
    menu::connect(app, session);
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_refresh(move || {
        if let Some(app) = ui.upgrade() {
            refresh(&app, &s);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_thumbs_arrived(move || {
        if let Some(app) = ui.upgrade()
            && s.library.thumbs.collect()
        {
            refresh(&app, &s);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_remote_arrived(move || {
        let Some(app) = ui.upgrade() else { return };
        let arrived: Vec<_> = s.library.expanded.lock().unwrap().drain(..).collect();
        for (url, entries) in arrived {
            let state = match entries {
                Some(entries) => Remote::Read(
                    entries
                        .into_iter()
                        .filter_map(|e| {
                            let source = Source::parse(&e.mrl)?;
                            let title = e.title.unwrap_or_else(|| source.title());
                            Some((source, title))
                        })
                        .collect(),
                ),
                None => Remote::Failed,
            };
            s.library.remote.borrow_mut().insert(url, state);
        }
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_tags_arrived(move || {
        let Some(app) = ui.upgrade() else { return };
        let mut music = s.library.music.borrow_mut();
        // Redraw every 25 files, and when done.
        let changed = music.collect();
        let redraw = changed && (music.reading() % 25 == 0);
        drop(music);
        if redraw && matches!(*s.library.at.borrow(), Place::Tag(..)) {
            refresh(&app, &s);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_save_streams(move |links, name| {
        let Some(app) = ui.upgrade() else { return false };
        match save_streams(&links, &name) {
            Ok(path) => {
                s.library.go(Place::Streams(path));
                if !app.get_lib_full() {
                    app.set_dock_open(true);
                }
                refresh(&app, &s);
                true
            }
            Err(e) => {
                eprintln!("ferricone: saving streams: {e}");
                false
            }
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_go_page(move |page| {
        let Some(app) = ui.upgrade() else { return };
        let place = match page {
            1 => Place::Section(Section::Videos, None),
            2 => Place::Section(Section::Music, None),
            3 => Place::Network,
            4 => Place::Playlists,
            5 => Place::Recent,
            _ => Place::Home,
        };
        s.library.go(place);
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_crumb(move |index| {
        let Some(app) = ui.upgrade() else { return };
        let place = s.library.crumbs.borrow().get(index as usize).cloned();
        if let Some(place) = place {
            s.library.go(place);
            refresh(&app, &s);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_back(move || {
        let Some(app) = ui.upgrade() else { return };
        let lib = &s.library;
        let Some(place) = lib.back.borrow_mut().pop() else {
            return;
        };
        let current = lib.at.replace(place);
        lib.forward.borrow_mut().push(current);
        lib.search.borrow_mut().clear();
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_forward(move || {
        let Some(app) = ui.upgrade() else { return };
        let lib = &s.library;
        let Some(place) = lib.forward.borrow_mut().pop() else {
            return;
        };
        let current = lib.at.replace(place);
        lib.back.borrow_mut().push(current);
        lib.search.borrow_mut().clear();
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_search(move |text| {
        let Some(app) = ui.upgrade() else { return };
        *s.library.search.borrow_mut() = text.trim().to_lowercase();
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_reveal(move || {
        let Some(app) = ui.upgrade() else { return };
        let place = match s.loaded.borrow().as_ref() {
            Some(Source::File(path)) => path
                .canonicalize()
                .ok()
                .and_then(|p| p.parent().map(|d| s.library.place_of(d))),
            Some(Source::Location(_)) => Some(Place::Network),
            None => None,
        };
        if let Some(place) = place {
            s.library.go(place);
        }
        if !app.get_lib_full() {
            app.set_dock_open(true);
        }
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_add_folder(move || {
        if let Some(app) = ui.upgrade() {
            let section = match *s.library.at.borrow() {
                Place::Section(Section::Music, _) => Section::Music,
                _ => Section::Videos,
            };
            add_folder(&app, &s, section);
        }
    });
    app.set_lib_sort_labels(ModelRc::new(VecModel::from(
        Sort::ALL.iter().map(|m| SharedString::from(m.label())).collect::<Vec<_>>(),
    )));
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_set_sort(move |index| {
        let Some(app) = ui.upgrade() else { return };
        if let Some(&sort) = Sort::ALL.get(index as usize) {
            let mut roots = s.library.roots.borrow_mut();
            roots.sort = sort;
            crate::state::write(roots_path().as_deref(), &*roots);
        }
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_set_grid(move |grid| {
        let Some(app) = ui.upgrade() else { return };
        let mut roots = s.library.roots.borrow_mut();
        roots.grid = grid;
        crate::state::write(roots_path().as_deref(), &*roots);
        drop(roots);
        refresh(&app, &s);
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_lib_open(move |index| {
        let Some(app) = ui.upgrade() else { return };
        let target = s
            .library
            .targets
            .borrow()
            .get(index as usize)
            .cloned()
            .flatten();
        match target {
            Some(Target::Go(place)) => {
                s.library.go(place);
                refresh(&app, &s);
            }
            Some(Target::Play(items, at)) => {
                s.queue.borrow_mut().replace(items);
                s.queue.borrow_mut().play_at(at);
                controls::play_current(&app, &s);
            }
            Some(Target::AddFolder(section)) => add_folder(&app, &s, section),
            Some(Target::NewPlaylist) => app.invoke_new_playlist(false),
            Some(Target::AddStream) => app.invoke_show_save_streams_dialog(),
            None => {}
        }
    });
}

/// Asks for a folder and adds it to `section`.
fn add_folder(app: &AppWindow, s: &Shared, section: Section) {
    let (ui, s) = (app.as_weak(), s.clone());
    let title = match section {
        Section::Videos => "Add a Video Folder",
        Section::Music => "Add a Music Folder",
    };
    let task = slint::spawn_local(async move {
        let folder = rfd::AsyncFileDialog::new().set_title(title).pick_folder().await;
        if let (Some(folder), Some(app)) = (folder, ui.upgrade()) {
            let dir = folder.path().to_path_buf();
            s.library.add_root(section, dir.clone());
            if section == Section::Music {
                let roots = s.library.roots.borrow().music.clone();
                s.library.music.borrow_mut().scan(&roots, true);
            }
            s.library.go(s.library.place_of(&dir));
            refresh(&app, &s);
        }
    });
    if let Err(e) = task {
        eprintln!("ferricone: folder dialog: {e}");
    }
}

/// One shown row and what it does.
struct Row {
    row: LibRow,
    target: Option<Target>,
}

/// Rebuilds what the dock and the full library show, when one shows.
pub fn refresh(app: &AppWindow, s: &Session) {
    if !app.get_dock_open() && !app.get_lib_full() {
        return;
    }
    let lib = &s.library;
    let place = lib.at.borrow().clone();
    let mut rows = rows(app, s, &place);

    // Search narrows to what can be played or entered.
    let search = lib.search.borrow().clone();
    if !search.is_empty() {
        rows.retain(|r| {
            !r.row.header
                && !r.row.action
                && r.row.title.to_lowercase().contains(&search)
        });
    }
    let count = rows.iter().filter(|r| !r.row.header && !r.row.action).count();
    rows.truncate(MAX_ROWS);
    app.set_lib_count(
        match count {
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        }
        .into(),
    );

    let (names, places) = crumbs(&lib.roots.borrow(), &place);
    *lib.crumbs.borrow_mut() = places;
    app.set_lib_crumbs(ModelRc::new(VecModel::from(names)));
    app.set_lib_page(place.page());
    app.set_lib_deep(place.deep());
    app.set_lib_can_back(!lib.back.borrow().is_empty());
    app.set_lib_can_forward(!lib.forward.borrow().is_empty());

    let (mut shown, targets): (Vec<LibRow>, Vec<_>) =
        rows.into_iter().map(|r| (r.row, r.target)).unzip();
    for (i, row) in shown.iter_mut().enumerate() {
        row.index = i as i32;
    }
    *lib.targets.borrow_mut() = targets;

    // The full library: the page's first action goes to the toolbar's +,
    // and Home's headers split the rest into sections.
    let action = shown
        .iter()
        .zip(lib.targets.borrow().iter())
        .find(|(r, t)| r.action && t.is_some())
        .map(|(r, _)| r);
    app.set_lib_action(action.map_or(-1, |r| r.index));
    app.set_lib_action_label(action.map_or_else(SharedString::new, |r| r.title.clone()));
    let mut sections: Vec<LibSection> = Vec::new();
    let mut items: Vec<LibRow> = Vec::new();
    let mut header: Option<&LibRow> = None;
    let mut close = |header: Option<&LibRow>, items: &mut Vec<LibRow>| {
        if header.is_some() || !items.is_empty() {
            sections.push(LibSection {
                title: header.map_or_else(SharedString::new, |h| h.title.clone()),
                header: header.map_or(-1, |h| h.index),
                items: ModelRc::new(VecModel::from(std::mem::take(items))),
            });
        }
    };
    for row in &shown {
        if row.header {
            close(header, &mut items);
            header = Some(row);
        } else if !row.action {
            items.push(row.clone());
        }
    }
    close(header, &mut items);
    app.set_lib_sections(ModelRc::new(VecModel::from(sections)));
    let roots = lib.roots.borrow();
    app.set_lib_sort(Sort::ALL.iter().position(|&m| m == roots.sort).unwrap_or(0) as i32);
    app.set_lib_grid(roots.grid);
    drop(roots);
    app.set_lib_rows(ModelRc::new(VecModel::from(shown)));
}

fn rows(app: &AppWindow, s: &Session, place: &Place) -> Vec<Row> {
    let icons = Icons::get(app);
    let lib = &s.library;
    let loaded = s.loaded.borrow().clone();
    let file_row = |source: &Source, target: Target| -> Row {
        let history = s.history.borrow();
        let played = history.get(source);
        let (thumb, icon) = match source {
            Source::File(path) if media::is_audio(path) => (None, icons.get_music()),
            Source::File(path) => (lib.thumbs.get(path), icons.get_video()),
            Source::Location(_) => (None, icons.get_streaming()),
        };
        Row {
            row: LibRow {
                title: source.title().into(),
                icon,
                has_thumb: thumb.is_some(),
                thumb: thumb.unwrap_or_default(),
                progress: played.map_or(0.0, |p| p.progress),
                watched: played.is_some_and(|p| p.watched),
                playing: loaded.as_ref() == Some(source),
                ..Default::default()
            },
            target: Some(target),
        }
    };
    let link_row = |title: &str, icon, target: Target| Row {
        row: LibRow {
            title: title.into(),
            icon,
            ..Default::default()
        },
        target: Some(target),
    };
    let action_row = |title: &str, target: Target| Row {
        row: LibRow {
            title: title.into(),
            icon: icons.get_add(),
            action: true,
            ..Default::default()
        },
        target: Some(target),
    };
    let header_row = |title: &str, target: Place| Row {
        row: LibRow {
            title: title.into(),
            header: true,
            ..Default::default()
        },
        target: Some(Target::Go(target)),
    };
    let folder_row = |dir: &Path| {
        let name = dir.file_name().map_or_else(
            || dir.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        link_row(&name, icons.get_folder(), Target::Go(lib.place_of(dir)))
    };

    match place {
        Place::Home => {
            let history = s.history.borrow();
            let recent = history.recent();
            let exists = |src: &Source| match src {
                Source::File(p) => p.exists(),
                Source::Location(_) => true,
            };
            let continuing: Vec<Source> = recent
                .iter()
                .filter(|(src, p)| {
                    matches!(src, Source::File(f) if !media::is_audio(f))
                        && !p.watched
                        && p.progress > 0.0
                        && exists(src)
                })
                .map(|(src, _)| src.clone())
                .take(HOME_ROWS)
                .collect();
            let listened: Vec<Source> = recent
                .iter()
                .filter(|(src, _)| matches!(src, Source::File(f) if media::is_audio(f)) && exists(src))
                .map(|(src, _)| src.clone())
                .take(HOME_ROWS)
                .collect();
            let streams: Vec<Source> = recent
                .iter()
                .filter(|(src, _)| matches!(src, Source::Location(_)))
                .map(|(src, _)| src.clone())
                .take(HOME_ROWS)
                .collect();
            let folders: Vec<PathBuf> = history
                .folders()
                .into_iter()
                .filter(|d| d.is_dir())
                .take(HOME_ROWS)
                .collect();
            drop(history);
            let lists = playlists::list();
            let mut out = Vec::new();
            let mut section = |title: &str, to: Place, rows: Vec<Row>| {
                if !rows.is_empty() {
                    out.push(header_row(title, to));
                    out.extend(rows);
                }
            };
            let play_one = |src: &Source| file_row(src, Target::Play(vec![src.clone()], 0));
            section("Continue Watching", Place::Recent, continuing.iter().map(play_one).collect());
            section("Listen Again", Place::Recent, listened.iter().map(play_one).collect());
            section(
                "Frequently Visited Folders",
                Place::Section(Section::Videos, None),
                folders.iter().map(|d| folder_row(d)).collect(),
            );
            section(
                "Playlists",
                Place::Playlists,
                lists
                    .iter()
                    .take(HOME_ROWS)
                    .map(|l| link_row(&l.name, icons.get_playlist(), Target::Go(Place::Playlist(l.path.clone()))))
                    .collect(),
            );
            section("Latest Streams", Place::Network, streams.iter().map(play_one).collect());
            if out.is_empty() {
                out.push(action_row("Add a video folder", Target::AddFolder(Section::Videos)));
                out.push(action_row("Add a music folder", Target::AddFolder(Section::Music)));
            }
            out
        }
        Place::Section(Section::Music, None) => vec![
            action_row("Add", Target::AddFolder(Section::Music)),
            link_row("Songs", icons.get_songs(), Target::Go(Place::Songs)),
            link_row("Albums", icons.get_album(), Target::Go(Place::Tag(Tag::Album, None))),
            link_row("Artists", icons.get_artist(), Target::Go(Place::Tag(Tag::Artist, None))),
            link_row("Genres", icons.get_genre(), Target::Go(Place::Tag(Tag::Genre, None))),
        ],
        Place::Songs => {
            let roots = lib.roots.borrow().music.clone();
            match roots.as_slice() {
                [] => vec![action_row("Add a music folder", Target::AddFolder(Section::Music))],
                [only] => folder_rows(only, &mut |d| folder_row(d), &file_row, s),
                many => many.iter().map(|r| folder_row(r)).collect(),
            }
        }
        Place::Tag(tag, value) => {
            let roots = lib.roots.borrow().music.clone();
            lib.music.borrow_mut().scan(&roots, false);
            let music = lib.music.borrow();
            let mut out = Vec::new();
            if music.reading() > 0 {
                out.push(Row {
                    row: LibRow {
                        title: format!("Reading tags… {} left", music.reading()).into(),
                        icon: icons.get_info(),
                        action: true,
                        ..Default::default()
                    },
                    target: None,
                });
            }
            let icon = match tag {
                Tag::Album => icons.get_album(),
                Tag::Artist => icons.get_artist(),
                Tag::Genre => icons.get_genre(),
            };
            match value {
                None => out.extend(music.values(*tag).into_iter().map(|(name, _)| {
                    link_row(&name, icon.clone(), Target::Go(Place::Tag(*tag, Some(name.clone()))))
                })),
                Some(value) => {
                    let tracks = music.tracks(*tag, value);
                    let sources: Vec<Source> =
                        tracks.iter().map(|(p, _)| Source::File(p.clone())).collect();
                    out.extend(tracks.iter().enumerate().map(|(i, (path, track))| {
                        let mut row = file_row(&Source::File(path.clone()), Target::Play(sources.clone(), i));
                        if let Some(title) = &track.title {
                            row.row.title = title.as_str().into();
                        }
                        row
                    }));
                }
            }
            out
        }
        Place::Section(section, None) => {
            let mut out = vec![action_row("Add", Target::AddFolder(*section))];
            let roots = lib.roots.borrow().of(*section).to_vec();
            match roots.as_slice() {
                [only] => out.extend(folder_rows(only, &mut |d| folder_row(d), &file_row, s)),
                many => out.extend(many.iter().map(|r| folder_row(r))),
            }
            out
        }
        Place::Section(_, Some(dir)) | Place::Folder(dir) => {
            folder_rows(dir, &mut |d| folder_row(d), &file_row, s)
        }
        Place::Network => {
            let history = s.history.borrow();
            let streams: Vec<Source> = history
                .recent()
                .into_iter()
                .filter(|(src, _)| matches!(src, Source::Location(_)))
                .map(|(src, _)| src)
                .take(RECENT_ROWS)
                .collect();
            drop(history);
            let mut out = vec![action_row("Add Network Stream", Target::AddStream)];
            let saved = playlists::streams_folder().map_or_else(Vec::new, |d| playlists::list_in(&d));
            out.extend(saved.into_iter().map(|l| {
                link_row(&l.name, icons.get_streaming(), Target::Go(Place::Streams(l.path)))
            }));
            out.extend(
                streams
                    .iter()
                    .map(|src| file_row(src, Target::Play(vec![src.clone()], 0))),
            );
            out
        }
        Place::Streams(path) => {
            let items = playlists::load(path);
            items
                .iter()
                .enumerate()
                .map(|(i, src)| match src {
                    Source::Location(url) if may_be_playlist(url) => {
                        link_row(&src.title(), icons.get_playlist(), Target::Go(Place::Remote(url.clone())))
                    }
                    _ => file_row(src, Target::Play(items.clone(), i)),
                })
                .collect()
        }
        Place::Remote(url) => {
            want_remote(lib, url);
            let remote = lib.remote.borrow();
            let note = |text: &str| Row {
                row: LibRow {
                    title: text.into(),
                    icon: icons.get_info(),
                    action: true,
                    ..Default::default()
                },
                target: None,
            };
            match remote.get(url) {
                Some(Remote::Read(entries)) => {
                    let sources: Vec<Source> = entries.iter().map(|(s, _)| s.clone()).collect();
                    entries
                        .iter()
                        .enumerate()
                        .map(|(i, (src, title))| {
                            let mut row = file_row(src, Target::Play(sources.clone(), i));
                            row.row.title = title.as_str().into();
                            row
                        })
                        .collect()
                }
                Some(Remote::Failed) => {
                    let source = Source::Location(url.clone());
                    vec![file_row(&source, Target::Play(vec![source.clone()], 0))]
                }
                _ => vec![note("Reading the playlist…")],
            }
        }
        Place::Playlists => {
            let mut out = vec![action_row("Create playlist", Target::NewPlaylist)];
            out.extend(playlists::list().into_iter().map(|l| {
                link_row(&l.name, icons.get_playlist(), Target::Go(Place::Playlist(l.path)))
            }));
            out
        }
        Place::Playlist(path) => {
            let items = playlists::load(path);
            items
                .iter()
                .enumerate()
                .map(|(i, src)| file_row(src, Target::Play(items.clone(), i)))
                .collect()
        }
        Place::Recent => {
            let history = s.history.borrow();
            let items: Vec<Source> = history
                .recent()
                .into_iter()
                .map(|(src, _)| src)
                .filter(|src| match src {
                    Source::File(p) => p.exists(),
                    Source::Location(_) => true,
                })
                .take(RECENT_ROWS)
                .collect();
            drop(history);
            items
                .iter()
                .map(|src| file_row(src, Target::Play(vec![src.clone()], 0)))
                .collect()
        }
    }
}

/// A folder's subfolders, then its media in the chosen order (by default
/// unwatched videos before watched ones, so the next episode is on top). A
/// file plays the folder from there, in that order.
fn folder_rows(
    dir: &Path,
    folder_row: &mut dyn FnMut(&Path) -> Row,
    file_row: &dyn Fn(&Source, Target) -> Row,
    s: &Session,
) -> Vec<Row> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            dirs.push((name, path));
        } else if media::is_media(&path) {
            files.push((name, path));
        }
    }
    let sort = s.library.roots.borrow().sort;
    dirs.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    if sort == Sort::TitleZa {
        dirs.reverse();
    }
    sort_files(&mut files, sort, &s.history.borrow());
    let sources: Vec<Source> = files.into_iter().map(|(_, p)| Source::File(p)).collect();
    let mut out: Vec<Row> = dirs.iter().map(|(_, d)| folder_row(d)).collect();
    out.extend(
        sources
            .iter()
            .enumerate()
            .map(|(i, src)| file_row(src, Target::Play(sources.clone(), i))),
    );
    out
}

/// Sorts `(name, path)` files by `sort`; ties go by name.
fn sort_files(files: &mut [(String, PathBuf)], sort: Sort, history: &crate::state::History) {
    let watched = |path: &Path| history.get(&Source::File(path.into())).is_some_and(|p| p.watched);
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let by_name = |a: &(String, PathBuf), b: &(String, PathBuf)| natural_cmp(&a.0, &b.0);
    match sort {
        // Music isn't "watched"; it keeps its track order.
        Sort::UnwatchedFirst | Sort::WatchedFirst => {
            let key = |p: &Path| !media::is_audio(p) && watched(p);
            files.sort_by(|a, b| {
                let order = key(&a.1).cmp(&key(&b.1));
                let order = if sort == Sort::WatchedFirst { order.reverse() } else { order };
                order.then_with(|| by_name(a, b))
            });
        }
        Sort::NewestFirst => files.sort_by(|a, b| modified(&b.1).cmp(&modified(&a.1)).then_with(|| by_name(a, b))),
        Sort::OldestFirst => files.sort_by(|a, b| modified(&a.1).cmp(&modified(&b.1)).then_with(|| by_name(a, b))),
        Sort::TitleAz => files.sort_by(by_name),
        Sort::TitleZa => files.sort_by(|a, b| by_name(b, a)),
    }
}

/// The breadcrumb's names and places; one name (the page's) at a page's top.
fn crumbs(roots: &Roots, place: &Place) -> (Vec<SharedString>, Vec<Place>) {
    let mut out: Vec<(String, Place)> = Vec::new();
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
    };
    match place {
        Place::Home => out.push(("Home".into(), Place::Home)),
        Place::Network => out.push(("Network".into(), Place::Network)),
        Place::Streams(path) => {
            out.push(("Network".into(), Place::Network));
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
            out.push((stem.unwrap_or_default(), place.clone()));
        }
        Place::Remote(url) => {
            out.push(("Network".into(), Place::Network));
            out.push((Source::Location(url.clone()).title(), place.clone()));
        }
        Place::Recent => out.push(("Recent".into(), Place::Recent)),
        Place::Playlists => out.push(("Playlists".into(), Place::Playlists)),
        Place::Playlist(path) => {
            out.push(("Playlists".into(), Place::Playlists));
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
            out.push((stem.unwrap_or_default(), place.clone()));
        }
        Place::Songs => {
            out.push(("Music".into(), Place::Section(Section::Music, None)));
            out.push(("Songs".into(), Place::Songs));
        }
        Place::Tag(tag, value) => {
            out.push(("Music".into(), Place::Section(Section::Music, None)));
            out.push((tag.plural().into(), Place::Tag(*tag, None)));
            if let Some(v) = value {
                out.push((v.clone(), place.clone()));
            }
        }
        Place::Section(section, dir) => {
            let top = Place::Section(*section, None);
            out.push((
                match section {
                    Section::Videos => "Videos".into(),
                    Section::Music => "Music".into(),
                },
                top,
            ));
            if let Some(dir) = dir {
                let root = roots
                    .of(*section)
                    .iter()
                    .find(|r| dir.starts_with(r))
                    .cloned();
                let single = roots.of(*section).len() == 1;
                // Music's folders are under Songs.
                if *section == Section::Music {
                    out.push(("Songs".into(), Place::Songs));
                }
                if let Some(root) = root {
                    // With one root, the section (or Songs) stands for it.
                    let mut at = if single { None } else { Some(root.clone()) };
                    if let Some(r) = &at {
                        out.push((name(r), Place::Section(*section, Some(r.clone()))));
                    }
                    for part in dir.strip_prefix(&root).into_iter().flat_map(|p| p.components()) {
                        let next = at.clone().unwrap_or_else(|| root.clone()).join(part);
                        out.push((name(&next), Place::Section(*section, Some(next.clone()))));
                        at = Some(next);
                    }
                }
            }
        }
        Place::Folder(dir) => {
            // From the home folder when inside it, else from the top.
            let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
            let start = home.filter(|h| dir.starts_with(h) && dir != h);
            let mut chain: Vec<PathBuf> = dir.ancestors().map(Path::to_path_buf).collect();
            chain.reverse();
            if let Some(h) = &start {
                chain.retain(|p| p.starts_with(h) && p != h);
            }
            out.extend(chain.into_iter().map(|p| (name(&p), Place::Folder(p))));
        }
    }
    if out.len() > MAX_CRUMBS {
        let tail = out.split_off(out.len() - 2);
        let before = out.pop().map(|(_, p)| p).unwrap_or(Place::Home);
        out = vec![("…".into(), before)];
        out.extend(tail);
    }
    out.into_iter().map(|(n, p)| (SharedString::from(n), p)).unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(videos: &[&str]) -> Roots {
        Roots {
            videos: videos.iter().map(PathBuf::from).collect(),
            music: Vec::new(),
            sort: Sort::default(),
            grid: true,
        }
    }

    #[test]
    fn breadcrumbs_follow_the_roots() {
        let r = roots(&["/m/Videos"]);
        let at = r.place_of(Path::new("/m/Videos/Shows/The Wand/S01"));
        assert_eq!(
            at,
            Place::Section(Section::Videos, Some("/m/Videos/Shows/The Wand/S01".into()))
        );
        let (names, places) = crumbs(&r, &at);
        assert_eq!(names, ["Videos", "Shows", "The Wand", "S01"]);
        assert_eq!(
            places[2],
            Place::Section(Section::Videos, Some("/m/Videos/Shows/The Wand".into()))
        );
        assert_eq!(r.place_of(Path::new("/m/Videos")), Place::Section(Section::Videos, None));

        let deeper = r.place_of(Path::new("/m/Videos/a/b/c/d"));
        let (names, places) = crumbs(&r, &deeper);
        assert_eq!(names, ["…", "c", "d"]);
        assert_eq!(places[0], Place::Section(Section::Videos, Some("/m/Videos/a/b".into())));

        let two = roots(&["/m/Videos", "/n/Films"]);
        let (names, _) = crumbs(&two, &two.place_of(Path::new("/n/Films/Old")));
        assert_eq!(names, ["Videos", "Films", "Old"]);
        assert_eq!(two.place_of(Path::new("/x/y")), Place::Folder("/x/y".into()));
    }
}
