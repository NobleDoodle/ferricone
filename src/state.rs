//! What the player remembers between runs, in the user's state folder
//! (`$XDG_STATE_HOME/ferricone` on Linux, the local data folder elsewhere):
//! where each item was left off (`positions.toml`, for the resume pill),
//! each item's bookmarks (`bookmarks.toml`), and what was played when and
//! how far (`history.toml`, for the library's progress, Recent and Home).

use crate::media::Source;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Positions this close to either end aren't worth resuming (and one this
/// close to the end counts as watched), or a tenth of a shorter item.
const MARGIN: Duration = Duration::from_secs(10);

/// `MARGIN`, scaled down for short items: a 30 s clip left at 21 s is
/// still worth resuming. Unknown lengths (live streams) get the full margin,
/// which keeps them from being resumed at all.
fn margin(length: Duration) -> Duration {
    if length.is_zero() {
        MARGIN
    } else {
        MARGIN.min(length / 10)
    }
}
/// Past this share of the item, it counts as watched.
const WATCHED: f64 = 0.95;
/// The most items remembered; the oldest go first.
const LIMIT: usize = 500;

#[derive(Default, Deserialize, Serialize)]
struct File {
    #[serde(default)]
    positions: HashMap<String, Entry>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct Entry {
    /// Seconds into the item.
    time: f64,
    /// When it was left, in Unix seconds, for dropping the oldest.
    saved: u64,
}

pub struct Positions {
    path: Option<PathBuf>,
    file: File,
}

impl Positions {
    /// Forgets every place (Clear history now).
    pub fn clear(&mut self) {
        self.file.positions.clear();
        self.save();
    }

    pub fn load() -> Self {
        let path = state_path("positions.toml");
        let file = read(path.as_deref());
        Self { path, file }
    }

    /// Where `source` was left off, if it is worth resuming.
    pub fn get(&self, source: &Source) -> Option<Duration> {
        let entry = self.file.positions.get(&key(source))?;
        Some(Duration::from_secs_f64(entry.time))
    }

    /// Notes that `source` was left at `time` of `length`, or forgets it when
    /// that is too near either end; saves when that changed anything.
    pub fn remember(&mut self, source: &Source, time: Duration, length: Duration) {
        let key = key(source);
        let margin = margin(length);
        let resumable = time >= margin
            && length.saturating_sub(time) >= margin
            && (length.is_zero() || time.as_secs_f64() < WATCHED * length.as_secs_f64());
        let changed = if resumable {
            let saved = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let entry = Entry {
                time: time.as_secs_f64(),
                saved,
            };
            self.file.positions.insert(key, entry);
            self.trim();
            true
        } else {
            self.file.positions.remove(&key).is_some()
        };
        if changed {
            self.save();
        }
    }

    fn trim(&mut self) {
        let positions = &mut self.file.positions;
        if positions.len() <= LIMIT {
            return;
        }
        let mut by_age: Vec<(u64, String)> = positions
            .iter()
            .map(|(k, e)| (e.saved, k.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, key) in by_age.into_iter().take(positions.len() - LIMIT) {
            positions.remove(&key);
        }
    }

    fn save(&self) {
        write(self.path.as_deref(), &self.file);
    }
}

/// A named moment in an item.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Bookmark {
    pub name: String,
    /// Seconds into the item.
    pub time: f64,
}

#[derive(Default, Deserialize, Serialize)]
struct BookmarkFile {
    #[serde(default)]
    bookmarks: HashMap<String, Vec<Bookmark>>,
}

pub struct Bookmarks {
    path: Option<PathBuf>,
    file: BookmarkFile,
}

impl Bookmarks {
    pub fn load() -> Self {
        let path = state_path("bookmarks.toml");
        let file = read(path.as_deref());
        Self { path, file }
    }

    /// `source`'s bookmarks, in time order.
    pub fn of(&self, source: &Source) -> &[Bookmark] {
        self.file
            .bookmarks
            .get(&key(source))
            .map_or(&[], Vec::as_slice)
    }

    /// Adds one at `time`, named "Bookmark N"; returns its index.
    pub fn add(&mut self, source: &Source, time: Duration) -> usize {
        let list = self.file.bookmarks.entry(key(source)).or_default();
        let bookmark = Bookmark {
            name: format!("Bookmark {}", list.len() + 1),
            time: time.as_secs_f64(),
        };
        let at = list.partition_point(|b| b.time <= bookmark.time);
        list.insert(at, bookmark);
        self.save();
        at
    }

    pub fn remove(&mut self, source: &Source, index: usize) {
        let key = key(source);
        if let Some(list) = self.file.bookmarks.get_mut(&key)
            && index < list.len()
        {
            list.remove(index);
            if list.is_empty() {
                self.file.bookmarks.remove(&key);
            }
            self.save();
        }
    }

    fn save(&self) {
        write(self.path.as_deref(), &self.file);
    }
}

/// What the library knows of one played item.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Played {
    /// How far it got, 0–1 of its length.
    pub progress: f32,
    /// Seen to the end (or near enough).
    pub watched: bool,
    /// When it was last opened, in Unix seconds.
    pub at: u64,
    /// How often it was opened.
    pub plays: u32,
}

#[derive(Default, Deserialize, Serialize)]
struct HistoryFile {
    #[serde(default)]
    played: HashMap<String, Played>,
}

/// Every item played, by `key`; the library's watched progress, Recent and
/// Home come from it.
pub struct History {
    path: Option<PathBuf>,
    file: HistoryFile,
}

/// The most items the history keeps; the least recent go first.
const HISTORY_LIMIT: usize = 2000;

impl History {
    pub fn load() -> Self {
        let path = state_path("history.toml");
        let file = read(path.as_deref());
        Self { path, file }
    }

    /// Forgets everything played (Settings › Privacy › Clear history now).
    pub fn clear(&mut self) {
        self.file.played.clear();
        self.save();
    }

    /// Marks `source` watched (bar full) or not (no bar), whether or not it
    /// was ever played; the library's Mark as Watched / Unwatched.
    pub fn set_watched(&mut self, sources: &[Source], watched: bool) {
        for source in sources {
            let entry = self.file.played.entry(key(source)).or_default();
            entry.watched = watched;
            entry.progress = if watched { 1.0 } else { 0.0 };
        }
        self.trim();
        self.save();
    }

    /// `source` was opened.
    pub fn started(&mut self, source: &Source) {
        let entry = self.file.played.entry(key(source)).or_default();
        entry.at = now();
        entry.plays += 1;
        self.trim();
        self.save();
    }

    /// `source` got to `time` of `length` (zero when unknown, as for
    /// streams).
    pub fn note(&mut self, source: &Source, time: Duration, length: Duration) {
        let Some(entry) = self.file.played.get_mut(&key(source)) else {
            return;
        };
        if length.is_zero() {
            return;
        }
        let progress = (time.as_secs_f64() / length.as_secs_f64()).clamp(0.0, 1.0);
        let near_end = length.saturating_sub(time) < margin(length) || progress >= WATCHED;
        entry.watched |= near_end;
        // Watched items start over; the bar shows how far the rewatch got.
        entry.progress = if near_end { 1.0 } else { progress as f32 };
        self.save();
    }

    pub fn get(&self, source: &Source) -> Option<&Played> {
        self.file.played.get(&key(source))
    }

    /// Played items, the latest first (not ones only marked watched).
    pub fn recent(&self) -> Vec<(Source, &Played)> {
        let mut items: Vec<_> = self
            .file
            .played
            .iter()
            .filter(|(_, p)| p.plays > 0)
            .map(|(k, p)| (from_key(k), p))
            .collect();
        items.sort_by_key(|a| std::cmp::Reverse(a.1.at));
        items
    }

    /// The folders of played files, the most played first.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut by_folder: HashMap<PathBuf, (u32, u64)> = HashMap::new();
        for (k, p) in &self.file.played {
            if let Source::File(path) = from_key(k)
                && let Some(parent) = path.parent()
            {
                let e = by_folder.entry(parent.to_path_buf()).or_default();
                e.0 += p.plays;
                e.1 = e.1.max(p.at);
            }
        }
        let mut folders: Vec<_> = by_folder.into_iter().collect();
        folders.sort_by_key(|a| std::cmp::Reverse(a.1));
        folders.into_iter().map(|(f, _)| f).collect()
    }

    fn trim(&mut self) {
        let played = &mut self.file.played;
        if played.len() <= HISTORY_LIMIT {
            return;
        }
        let mut by_age: Vec<(u64, String)> = played.iter().map(|(k, p)| (p.at, k.clone())).collect();
        by_age.sort_unstable();
        for (_, key) in by_age.into_iter().take(played.len() - HISTORY_LIMIT) {
            played.remove(&key);
        }
    }

    fn save(&self) {
        write(self.path.as_deref(), &self.file);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `key`'s inverse: paths are absolute, anything else a location.
fn from_key(key: &str) -> Source {
    if key.starts_with('/') || std::path::Path::new(key).is_absolute() {
        Source::File(key.into())
    } else {
        Source::Location(key.into())
    }
}

pub(crate) fn state_path(name: &str) -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
        .map(|dirs| dirs.state_dir().unwrap_or(dirs.data_local_dir()).join(name))
}

/// The file's contents, or empty ones when it is missing or unreadable.
pub(crate) fn read<T: DeserializeOwned + Default>(path: Option<&std::path::Path>) -> T {
    let Some(path) = path else {
        return T::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return T::default();
    };
    toml::from_str(&text).unwrap_or_else(|e| {
        eprintln!("ferricone: ignoring unreadable {}: {e}", path.display());
        T::default()
    })
}

/// Writes the file whole, through a temporary file, so a crash can't leave
/// half of it.
pub(crate) fn write<T: Serialize>(path: Option<&std::path::Path>, contents: &T) {
    let Some(path) = path else { return };
    let result = (|| -> Result<(), String> {
        let text = toml::to_string(contents).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, text).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        eprintln!("ferricone: saving {}: {e}", path.display());
    }
}

/// Files by their full path, locations by their URL.
fn key(source: &Source) -> String {
    match source {
        Source::File(path) => path
            .canonicalize()
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .into_owned(),
        Source::Location(url) => url.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Positions {
        Positions {
            path: None,
            file: File::default(),
        }
    }

    #[test]
    fn keeps_only_resumable_positions() {
        let mut p = store();
        let a = Source::Location("http://h/a".into());
        let min = |m: u64| Duration::from_secs(m * 60);
        p.remember(&a, min(5), min(20));
        assert_eq!(p.get(&a), Some(min(5)));
        // Near the start, near the end, or watched: forgotten.
        p.remember(&a, Duration::from_secs(3), min(20));
        assert_eq!(p.get(&a), None);
        p.remember(&a, min(5), min(20));
        p.remember(&a, min(20) - Duration::from_secs(5), min(20));
        assert_eq!(p.get(&a), None);
        p.remember(&a, min(5), min(20));
        p.remember(&a, Duration::from_secs(19 * 60 + 10), min(20));
        assert_eq!(p.get(&a), None);
        // A 30 s clip: a tenth (3 s) is the margin, so 21 s is kept.
        let secs = Duration::from_secs;
        p.remember(&a, secs(21), secs(30));
        assert_eq!(p.get(&a), Some(secs(21)));
        p.remember(&a, secs(28), secs(30));
        assert_eq!(p.get(&a), None);
        // A live stream (no length) isn't resumed.
        p.remember(&a, min(5), Duration::ZERO);
        assert_eq!(p.get(&a), None);
    }

    #[test]
    fn history_marks_items_watched_near_the_end() {
        let mut h = History {
            path: None,
            file: HistoryFile::default(),
        };
        let a = Source::File("/v/a.mkv".into());
        let b = Source::Location("https://example.com/live".into());
        let min = |m: u64| Duration::from_secs(m * 60);
        h.note(&a, min(5), min(20));
        assert!(h.get(&a).is_none(), "only opened items are noted");
        h.started(&a);
        h.note(&a, min(5), min(20));
        assert_eq!(h.get(&a).unwrap().progress, 0.25);
        h.note(&a, min(20) - Duration::from_secs(5), min(20));
        assert!(h.get(&a).unwrap().watched);
        h.started(&b);
        h.note(&b, min(5), Duration::ZERO);
        assert_eq!(h.get(&b).unwrap().progress, 0.0);
        assert_eq!(h.recent().len(), 2);
        assert_eq!(h.folders(), [PathBuf::from("/v")]);
    }

    #[test]
    fn keeps_bookmarks_in_time_order() {
        let mut b = Bookmarks {
            path: None,
            file: BookmarkFile::default(),
        };
        let a = Source::Location("http://h/a".into());
        assert_eq!(b.add(&a, Duration::from_secs(40)), 0);
        assert_eq!(b.add(&a, Duration::from_secs(10)), 0);
        let names: Vec<&str> = b.of(&a).iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Bookmark 2", "Bookmark 1"]);
        b.remove(&a, 0);
        b.remove(&a, 0);
        assert!(b.of(&a).is_empty());
    }

    #[test]
    fn round_trips_through_toml() {
        let mut p = store();
        let a = Source::Location("http://h/a b".into());
        p.remember(&a, Duration::from_secs(90), Duration::from_secs(600));
        let text = toml::to_string(&p.file).unwrap();
        let file: File = toml::from_str(&text).unwrap();
        assert_eq!(file.positions["http://h/a b"].time, 90.0);
    }
}
