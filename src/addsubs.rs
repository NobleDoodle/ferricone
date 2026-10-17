//! Add Subtitles (ui/addsubs.slint, after the Figma `Add Subtitles to
//! Playlist/Folder` frames): subtitle files for one video, picked from those
//! beside it (or in a `Subs` / `Subtitles` folder, or anywhere through
//! Browse), or for a whole folder, each episode matched to its subtitle by
//! name or episode number and changeable row by row. The choices are kept in
//! `subtitles.toml` in the state folder, not by renaming files, and load
//! whenever that video plays. From OpenSubtitles (one file at a time), the
//! results of a search are ticked and downloaded into the data folder's
//! `subtitles`.

use crate::{
    AppWindow, SubFileRow, SubPairRow,
    controls::{Session, Shared},
    media::{self, Source, natural_cmp},
    state,
};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

pub const SUB_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "smi"];

pub fn is_subtitle(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SUB_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

// The saved choices ----------------------------------------------------------

#[derive(Default, Deserialize, Serialize)]
struct SubtitleFile {
    /// Video path → the subtitle files chosen for it.
    #[serde(default)]
    files: HashMap<String, Vec<String>>,
}

fn store() -> (Option<PathBuf>, SubtitleFile) {
    let path = state::state_path("subtitles.toml");
    let file = state::read(path.as_deref());
    (path, file)
}

/// The subtitle files chosen for `video` that still exist.
pub fn chosen_for(video: &Path) -> Vec<PathBuf> {
    let (_, file) = store();
    file.files
        .get(&*video.to_string_lossy())
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

fn save_choices(choices: Vec<(PathBuf, Vec<PathBuf>)>) {
    let (path, mut file) = store();
    for (video, subs) in choices {
        let key = video.to_string_lossy().into_owned();
        if subs.is_empty() {
            file.files.remove(&key);
        } else {
            file.files.insert(key, subs.iter().map(|p| p.to_string_lossy().into_owned()).collect());
        }
    }
    state::write(path.as_deref(), &file);
}

/// Loads the chosen subtitles into the item just opened.
pub fn load_into(s: &Session, video: &Path) {
    for sub in chosen_for(video) {
        s.with_player(|p| {
            if let Err(e) = p.add_subtitle(&sub) {
                eprintln!("ferricone: {e}");
            }
        });
    }
}

// Finding and matching -------------------------------------------------------

/// Subtitle files in `dir` and in its `Subs` / `Subtitles` folders (two
/// levels, for `Subs/Episode 1/English.srt`), in name order.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                let subs_folder = matches!(name.as_str(), "subs" | "sub" | "subtitles" | "subtitle");
                if depth > 0 && (subs_folder || depth < 2) {
                    walk(&path, depth - 1, out);
                }
            } else if is_subtitle(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    // Into Subs folders, and one level below them.
    walk(dir, 2, &mut out);
    out.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out.dedup();
    out
}

/// "S01E02", "1x02" or "E02" in a name: (season, episode).
pub fn episode(name: &str) -> Option<(Option<u32>, u32)> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let number_at = |i: usize| -> Option<(u32, usize)> {
        let digits: String = lower[i..].chars().take_while(char::is_ascii_digit).collect();
        (!digits.is_empty()).then(|| (digits.parse().ok(), i + digits.len())).and_then(|(n, j)| Some((n?, j)))
    };
    for i in 0..bytes.len() {
        // S01E02
        if bytes[i] == b's'
            && let Some((season, j)) = number_at(i + 1)
            && bytes.get(j) == Some(&b'e')
            && let Some((ep, _)) = number_at(j + 1)
        {
            return Some((Some(season), ep));
        }
        // 1x02
        if (i == 0 || !bytes[i - 1].is_ascii_digit())
            && let Some((season, j)) = number_at(i)
            && bytes.get(j) == Some(&b'x')
            && let Some((ep, _)) = number_at(j + 1)
        {
            return Some((Some(season), ep));
        }
    }
    // E02 or Ep 2 on its own.
    for i in 0..bytes.len() {
        let after_letter = i > 0 && bytes[i - 1].is_ascii_alphabetic();
        if bytes[i] == b'e' && !after_letter {
            let j = if lower[i..].starts_with("ep") { i + 2 } else { i + 1 };
            let j = j + lower[j..].chars().take_while(|c| *c == ' ' || *c == '.').count();
            if let Some((ep, _)) = number_at(j) {
                return Some((None, ep));
            }
        }
    }
    None
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// How well `sub` fits `video`: 0 not at all.
pub fn score(video: &Path, sub: &Path) -> u32 {
    let (v, s) = (stem(video), stem(sub));
    if s == v {
        return 100;
    }
    if s.starts_with(&v) {
        return 90;
    }
    // Inside Subs/<video name>/…
    if sub
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n.to_string_lossy().to_lowercase() == v)
    {
        return 85;
    }
    match (episode(&v), episode(&s)) {
        (Some((vs, ve)), Some((ss, se))) if ve == se => match (vs, ss) {
            (Some(a), Some(b)) if a == b => 80,
            (Some(_), Some(_)) => 0,
            _ => 60,
        },
        _ => 0,
    }
}

/// The best subtitle for each video, if any fits; each used once.
pub fn best_matches(videos: &[PathBuf], subs: &[PathBuf]) -> Vec<Option<usize>> {
    let mut used = vec![false; subs.len()];
    videos
        .iter()
        .map(|v| {
            let best = subs
                .iter()
                .enumerate()
                .filter(|(i, _)| !used[*i])
                .map(|(i, s)| (score(v, s), i))
                .filter(|(score, _)| *score > 0)
                .max_by_key(|(score, i)| (*score, std::cmp::Reverse(*i)));
            best.map(|(_, i)| {
                used[i] = true;
                i
            })
        })
        .collect()
}

// The dialog --------------------------------------------------------------------

#[derive(Default)]
struct Dialog {
    /// The video (file mode) or every video of the folder (folder mode).
    videos: Vec<PathBuf>,
    folder_mode: bool,
    /// Where subtitles were looked for.
    location: PathBuf,
    candidates: Vec<PathBuf>,
    /// File mode: which candidates are ticked.
    ticked: Vec<bool>,
    /// Folder mode: each video's candidate.
    picks: Vec<Option<usize>>,
    /// From OpenSubtitles rather than the computer.
    online: bool,
    query: String,
    language: usize,
    results: Vec<crate::opensubtitles::Found>,
    picked: Vec<bool>,
    /// Searching or downloading; what the note says meanwhile.
    busy: Option<String>,
    /// The last search's or download's problem.
    problem: Option<String>,
}

thread_local! {
    static DIALOG: RefCell<Dialog> = RefCell::default();
    /// For finishing downloads back on the UI thread.
    static SESSION: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

/// The last folders with " / " between them, as the Figma frame shows the
/// place ("… / Shows / The Wand / S01").
fn shown(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let tail = parts[parts.len().saturating_sub(3)..].join(" / ");
    if parts.len() > 3 { format!("… / {tail}") } else { tail }
}

fn name_in(location: &Path, path: &Path) -> String {
    path.strip_prefix(location)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Opens the dialog for `video`, or for its whole folder.
pub fn open(app: &AppWindow, video: &Path, folder_mode: bool) {
    let Some(dir) = video.parent().map(Path::to_path_buf) else { return };
    let videos = if folder_mode {
        media::expand([Source::File(dir.clone())])
            .into_iter()
            .filter_map(|s| match s {
                Source::File(p) if p.parent() == Some(&dir) && !media::is_audio(&p) => Some(p),
                _ => None,
            })
            .collect()
    } else {
        vec![video.to_path_buf()]
    };
    let prefs = crate::prefs::get();
    let language = crate::opensubtitles::LANGUAGES
        .iter()
        .position(|(c, _)| *c == prefs.os_language)
        .unwrap_or(0);
    DIALOG.with(|d| {
        let mut d = d.borrow_mut();
        *d = Dialog {
            videos,
            folder_mode,
            query: crate::opensubtitles::query_for(video),
            language,
            ..Default::default()
        };
        look_in(&mut d, dir);
    });
    show(app);
    app.set_addsubs_open(true);
}

/// Looks for subtitles in `dir`, choosing what was chosen before, or the
/// best matches.
fn look_in(d: &mut Dialog, dir: PathBuf) {
    let mut candidates = find(&dir);
    // Ones chosen before, from elsewhere, stay offered.
    for v in &d.videos {
        for sub in chosen_for(v) {
            if !candidates.contains(&sub) {
                candidates.push(sub);
            }
        }
    }
    if d.folder_mode {
        let best = best_matches(&d.videos, &candidates);
        d.picks = d
            .videos
            .iter()
            .zip(best)
            .map(|(v, best)| {
                let before = chosen_for(v).first().and_then(|s| candidates.iter().position(|c| c == s));
                before.or(best)
            })
            .collect();
    } else {
        let video = &d.videos[0];
        // Closest first.
        candidates.sort_by_key(|c| std::cmp::Reverse(score(video, c)));
        let before = chosen_for(video);
        d.ticked = candidates.iter().map(|c| before.contains(c)).collect();
        // Nothing chosen yet: tick the one that clearly belongs.
        if before.is_empty()
            && let Some(first) = candidates.first()
            && score(video, first) >= 80
        {
            d.ticked[0] = true;
        }
    }
    d.candidates = candidates;
    d.location = dir;
}

fn show(app: &AppWindow) {
    DIALOG.with(|d| {
        let d = d.borrow();
        app.set_addsubs_folder_mode(d.folder_mode);
        app.set_addsubs_location(shown(&d.location).into());
        let names: Vec<SharedString> = d.candidates.iter().map(|c| name_in(&d.location, c).into()).collect();
        app.set_addsubs_files(ModelRc::new(VecModel::from(
            names
                .iter()
                .zip(&d.ticked)
                .map(|(name, &chosen)| SubFileRow { name: name.clone(), chosen, ..Default::default() })
                .collect::<Vec<_>>(),
        )));
        let mut options = vec![SharedString::from("None")];
        options.extend(names);
        let options = ModelRc::new(VecModel::from(options));
        app.set_addsubs_pairs(ModelRc::new(VecModel::from(
            d.videos
                .iter()
                .zip(&d.picks)
                .map(|(v, pick)| SubPairRow {
                    video: v.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into(),
                    options: options.clone(),
                    selected: pick.map_or(0, |i| i as i32 + 1),
                })
                .collect::<Vec<_>>(),
        )));
        app.set_addsubs_online(d.online);
        app.set_addsubs_query(d.query.clone().into());
        app.set_addsubs_language(d.language as i32);
        app.set_addsubs_busy(d.busy.is_some());
        app.set_addsubs_results(ModelRc::new(VecModel::from(
            d.results
                .iter()
                .zip(&d.picked)
                .map(|(f, &chosen)| SubFileRow {
                    name: f.file_name.clone().into(),
                    detail: format!(
                        "{}{} · {} downloads{}",
                        f.language,
                        if f.release.is_empty() { String::new() } else { format!(" · {}", f.release) },
                        f.downloads,
                        if f.exact { " · made for this file" } else { "" }
                    )
                    .into(),
                    chosen,
                })
                .collect::<Vec<_>>(),
        )));
        let note = if let Some(busy) = &d.busy {
            busy.clone()
        } else if let Some(problem) = &d.problem {
            problem.clone()
        } else if d.online {
            if d.results.is_empty() { "Search to see subtitles from opensubtitles.com.".into() } else { String::new() }
        } else if d.candidates.is_empty() {
            "No subtitle files here. Browse to look in another folder or pick files.".into()
        } else {
            String::new()
        };
        app.set_addsubs_note(note.into());
    });
}

pub fn connect(app: &AppWindow, session: &Shared) {
    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    app.set_addsubs_languages(ModelRc::new(VecModel::from(
        crate::opensubtitles::LANGUAGES
            .iter()
            .map(|(_, name)| SharedString::from(*name))
            .collect::<Vec<_>>(),
    )));
    // The settings sidebar's Subtitles tab: for the item playing.
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_addsubs_for_loaded(move || {
        let Some(app) = ui.upgrade() else { return };
        if let Some(Source::File(path)) = s.loaded.borrow().clone() {
            open(&app, &path, false);
        }
    });
    let ui = app.as_weak();
    app.on_addsubs_toggle(move |i| {
        DIALOG.with(|d| {
            if let Some(t) = d.borrow_mut().ticked.get_mut(i as usize) {
                *t = !*t;
            }
        });
        if let Some(app) = ui.upgrade() {
            show(&app);
        }
    });
    let ui = app.as_weak();
    app.on_addsubs_pick(move |row, option| {
        DIALOG.with(|d| {
            if let Some(p) = d.borrow_mut().picks.get_mut(row as usize) {
                *p = (option > 0).then(|| option as usize - 1);
            }
        });
        if let Some(app) = ui.upgrade() {
            show(&app);
        }
    });
    let ui = app.as_weak();
    app.on_addsubs_set_folder_mode(move |folder| {
        let Some(app) = ui.upgrade() else { return };
        let video = DIALOG.with(|d| d.borrow().videos.first().cloned());
        if let Some(video) = video {
            open(&app, &video, folder);
        }
    });
    let ui = app.as_weak();
    app.on_addsubs_browse(move || browse(&ui));
    let ui = app.as_weak();
    app.on_addsubs_set_online(move |online| {
        DIALOG.with(|d| {
            let mut d = d.borrow_mut();
            d.online = online;
            d.problem = None;
            // OpenSubtitles works one file at a time.
            if online && d.folder_mode {
                d.folder_mode = false;
                d.videos.truncate(1);
                let dir = d.location.clone();
                look_in(&mut d, dir);
            }
        });
        if let Some(app) = ui.upgrade() {
            show(&app);
        }
    });
    let ui = app.as_weak();
    app.on_addsubs_search(move |query, language| {
        DIALOG.with(|d| {
            let mut d = d.borrow_mut();
            d.query = query.to_string();
            d.language = language.max(0) as usize;
        });
        search(&ui);
    });
    let ui = app.as_weak();
    app.on_addsubs_toggle_result(move |i| {
        DIALOG.with(|d| {
            if let Some(t) = d.borrow_mut().picked.get_mut(i as usize) {
                *t = !*t;
            }
        });
        if let Some(app) = ui.upgrade() {
            show(&app);
        }
    });
    let (ui, s) = (app.as_weak(), session.clone());
    app.on_addsubs_save(move || {
        let Some(app) = ui.upgrade() else { return };
        if DIALOG.with(|d| d.borrow().online) {
            download(&app);
            return;
        }
        let choices = DIALOG.with(|d| {
            let d = d.borrow();
            if d.folder_mode {
                d.videos
                    .iter()
                    .zip(&d.picks)
                    .map(|(v, pick)| (v.clone(), pick.and_then(|i| d.candidates.get(i).cloned()).into_iter().collect()))
                    .collect::<Vec<_>>()
            } else {
                let chosen: Vec<PathBuf> = d
                    .candidates
                    .iter()
                    .zip(&d.ticked)
                    .filter(|(_, t)| **t)
                    .map(|(c, _)| c.clone())
                    .collect();
                vec![(d.videos[0].clone(), chosen)]
            }
        });
        // The item playing gets its new ones now.
        let loaded = match &*s.loaded.borrow() {
            Some(Source::File(p)) => Some(p.clone()),
            _ => None,
        };
        let before = loaded.as_deref().map(chosen_for).unwrap_or_default();
        let now: Vec<PathBuf> = choices
            .iter()
            .filter(|(v, _)| Some(v) == loaded.as_ref())
            .flat_map(|(_, subs)| subs.clone())
            .collect();
        save_choices(choices);
        add_new(&s, &before, &now);
        app.set_addsubs_open(false);
    });
}

/// Loads into the item playing the subtitles in `now` it didn't have.
fn add_new(s: &Session, before: &[PathBuf], now: &[PathBuf]) {
    for sub in now.iter().filter(|s| !before.contains(s)) {
        s.with_player(|p| {
            if let Err(e) = p.add_subtitle(sub) {
                eprintln!("ferricone: {e}");
            }
        });
    }
}

/// Searches OpenSubtitles off the UI thread.
fn search(ui: &slint::Weak<AppWindow>) {
    let (query, language, video) = DIALOG.with(|d| {
        let mut d = d.borrow_mut();
        d.busy = Some("Searching OpenSubtitles…".into());
        d.problem = None;
        let code = crate::opensubtitles::LANGUAGES.get(d.language).map_or("en", |(c, _)| *c);
        (d.query.clone(), code.to_string(), d.videos.first().cloned())
    });
    if let Some(app) = ui.upgrade() {
        show(&app);
    }
    let p = crate::prefs::get();
    let ui = ui.clone();
    std::thread::spawn(move || {
        let found = crate::opensubtitles::Client::new(&p.os_key, &p.os_username, &p.os_password)
            .and_then(|c| c.search(&query, &language, video.as_deref()));
        let _ = slint::invoke_from_event_loop(move || {
            DIALOG.with(|d| {
                let mut d = d.borrow_mut();
                d.busy = None;
                match found {
                    Ok(results) => {
                        d.problem = results.is_empty().then(|| "OpenSubtitles found nothing; try other words or another language.".into());
                        // The one made for this file comes ticked.
                        d.picked = results.iter().enumerate().map(|(i, f)| i == 0 && f.exact).collect();
                        d.results = results;
                    }
                    Err(e) => d.problem = Some(e),
                }
            });
            if let Some(app) = ui.upgrade() {
                show(&app);
            }
        });
    });
}

/// Where downloaded subtitles are kept.
fn download_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
        .map(|d| d.data_dir().join("subtitles"))
}

/// Downloads the ticked results, then attaches them to the video.
fn download(app: &AppWindow) {
    let (wanted, video) = DIALOG.with(|d| {
        let mut d = d.borrow_mut();
        let wanted: Vec<_> = d
            .results
            .iter()
            .zip(&d.picked)
            .filter(|(_, p)| **p)
            .map(|(f, _)| f.clone())
            .collect();
        if !wanted.is_empty() {
            d.busy = Some("Downloading…".into());
            d.problem = None;
        }
        (wanted, d.videos.first().cloned())
    });
    let (Some(video), false) = (video, wanted.is_empty()) else {
        app.set_addsubs_open(false);
        return;
    };
    show(app);
    let p = crate::prefs::get();
    let ui = app.as_weak();
    let dir = download_dir();
    let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<PathBuf>, String> {
            let dir = dir.ok_or("there is no data folder")?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let client = crate::opensubtitles::Client::new(&p.os_key, &p.os_username, &p.os_password)?;
            let mut saved = Vec::new();
            for found in wanted {
                let bytes = client.download(found.file_id)?;
                let ext = Path::new(&found.file_name)
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "srt".into());
                let path = dir.join(format!("{stem}.{}.{}.{ext}", found.language, found.file_id));
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                saved.push(path);
            }
            Ok(saved)
        })();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = ui.upgrade() else { return };
            match result {
                Ok(saved) => {
                    let before = chosen_for(&video);
                    let mut all = before.clone();
                    all.extend(saved.iter().cloned());
                    save_choices(vec![(video.clone(), all)]);
                    SESSION.with(|s| {
                        if let Some(s) = &*s.borrow()
                            && matches!(&*s.loaded.borrow(), Some(Source::File(p)) if *p == video)
                        {
                            add_new(s, &before, &saved);
                        }
                    });
                    DIALOG.with(|d| d.borrow_mut().busy = None);
                    app.set_addsubs_open(false);
                }
                Err(e) => {
                    DIALOG.with(|d| {
                        let mut d = d.borrow_mut();
                        d.busy = None;
                        d.problem = Some(e);
                    });
                    show(&app);
                }
            }
        });
    });
}

/// File mode: pick subtitle files anywhere (ticked). Folder mode: pick the
/// folder to look in.
fn browse(ui: &slint::Weak<AppWindow>) {
    let folder_mode = DIALOG.with(|d| d.borrow().folder_mode);
    let start = DIALOG.with(|d| d.borrow().location.clone());
    let ui = ui.clone();
    let task = slint::spawn_local(async move {
        let dialog = rfd::AsyncFileDialog::new().set_directory(&start);
        if folder_mode {
            let Some(dir) = dialog.set_title("Look for Subtitles In").pick_folder().await else { return };
            DIALOG.with(|d| look_in(&mut d.borrow_mut(), dir.path().to_path_buf()));
        } else {
            let Some(files) = dialog
                .set_title("Choose Subtitle Files")
                .add_filter("Subtitles", SUB_EXTENSIONS)
                .pick_files()
                .await
            else {
                return;
            };
            DIALOG.with(|d| {
                let mut d = d.borrow_mut();
                for f in files {
                    let path = f.path().to_path_buf();
                    match d.candidates.iter().position(|c| *c == path) {
                        Some(i) => d.ticked[i] = true,
                        None => {
                            d.candidates.insert(0, path);
                            d.ticked.insert(0, true);
                        }
                    }
                }
            });
        }
        if let Some(app) = ui.upgrade() {
            show(&app);
        }
    });
    if let Err(e) = task {
        eprintln!("ferricone: file dialog: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_episode_numbers() {
        assert_eq!(episode("The_Wand_S01E05"), Some((Some(1), 5)));
        assert_eq!(episode("show.1x12.720p"), Some((Some(1), 12)));
        assert_eq!(episode("Anime - Ep 07 [1080p]"), Some((None, 7)));
        assert_eq!(episode("Movie (2014)"), None);
    }

    #[test]
    fn matches_episodes_to_subtitles() {
        let videos: Vec<PathBuf> = ["/s/Show S01E01.mkv", "/s/Show S01E02.mkv", "/s/Show S01E03.mkv"]
            .map(PathBuf::from)
            .to_vec();
        let subs: Vec<PathBuf> = [
            "/s/Subs/show.s01e02.en.srt",
            "/s/Show S01E01.srt",
            "/s/Subs/Show S01E03/English.srt",
            "/s/other.srt",
        ]
        .map(PathBuf::from)
        .to_vec();
        assert_eq!(best_matches(&videos, &subs), [Some(1), Some(0), Some(2)]);
        assert_eq!(score(&videos[0], &subs[3]), 0);
    }
}
