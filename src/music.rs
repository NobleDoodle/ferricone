//! The Music page's tags: every audio file under the music folders, with its
//! title, artist, album, genre and track number as libVLC reads them. Kept
//! in the cache folder (`music.toml`) by modification time, so only new or
//! changed files are read again.

use crate::{
    media::{self, Source, natural_cmp},
    vlc::{Tagger, Tags, Vlc},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

/// What the Albums, Artists and Genres pages group by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Album,
    Artist,
    Genre,
}

impl Tag {
    pub fn plural(self) -> &'static str {
        match self {
            Tag::Album => "Albums",
            Tag::Artist => "Artists",
            Tag::Genre => "Genres",
        }
    }

    fn unknown(self) -> &'static str {
        match self {
            Tag::Album => "Unknown Album",
            Tag::Artist => "Unknown Artist",
            Tag::Genre => "Unknown Genre",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Track {
    /// The file's modification time when read, in Unix seconds.
    modified: u64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
}

impl Track {
    fn value(&self, tag: Tag) -> &str {
        let value = match tag {
            Tag::Album => &self.album,
            Tag::Artist => &self.artist,
            Tag::Genre => &self.genre,
        };
        value.as_deref().filter(|v| !v.trim().is_empty()).unwrap_or(tag.unknown())
    }
}

#[derive(Default, Deserialize, Serialize)]
struct IndexFile {
    #[serde(default)]
    tracks: HashMap<String, Track>,
}

type Arrived = Arc<Mutex<Vec<(PathBuf, Option<Tags>)>>>;

pub struct MusicIndex {
    path: Option<PathBuf>,
    file: IndexFile,
    tagger: Option<Tagger>,
    /// Files being read, with their modification times.
    pending: HashMap<PathBuf, u64>,
    arrived: Arrived,
    scanned: bool,
}

impl MusicIndex {
    pub fn load() -> Self {
        let path = directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
            .map(|d| d.cache_dir().join("music.toml"));
        Self {
            file: crate::state::read(path.as_deref()),
            path,
            tagger: None,
            pending: HashMap::new(),
            arrived: Arc::default(),
            scanned: false,
        }
    }

    /// Starts the tag reader; `ready` runs on its threads after each file,
    /// and should get the UI thread to call `collect`.
    pub fn start(&mut self, vlc: &Vlc, ready: impl Fn() + Send + Sync + 'static) {
        let arrived = self.arrived.clone();
        match vlc.tagger(move |path, tags| {
            arrived.lock().unwrap().push((path, tags));
            ready();
        }) {
            Ok(t) => self.tagger = Some(t),
            Err(e) => eprintln!("ferricone: music tags: {e}"),
        }
    }

    /// Files still being read.
    pub fn reading(&self) -> usize {
        self.pending.len()
    }

    /// Walks `roots` once per run (or again when `again`), reading the tags
    /// of new and changed files and forgetting deleted ones.
    pub fn scan(&mut self, roots: &[PathBuf], again: bool) {
        if self.scanned && !again {
            return;
        }
        self.scanned = true;
        let files = media::expand(roots.iter().map(|r| Source::File(r.clone())));
        let mut seen = HashSet::new();
        for source in files {
            let Source::File(path) = source else { continue };
            if !media::is_audio(&path) {
                continue;
            }
            let key = path.to_string_lossy().into_owned();
            let modified = modified(&path);
            seen.insert(key.clone());
            let fresh = self.file.tracks.get(&key).is_some_and(|t| t.modified == modified);
            if fresh || self.pending.contains_key(&path) {
                continue;
            }
            if let Some(tagger) = &self.tagger
                && tagger.read(&path).is_ok()
            {
                self.pending.insert(path, modified);
            }
        }
        let before = self.file.tracks.len();
        self.file
            .tracks
            .retain(|k, _| seen.contains(k) || !roots.iter().any(|r| Path::new(k).starts_with(r)));
        if self.file.tracks.len() != before && self.pending.is_empty() {
            self.save();
        }
    }

    /// Takes in what was read; true when anything changed.
    pub fn collect(&mut self) -> bool {
        let arrived: Vec<_> = self.arrived.lock().unwrap().drain(..).collect();
        let any = !arrived.is_empty();
        for (path, tags) in arrived {
            let Some(modified) = self.pending.remove(&path) else {
                continue;
            };
            let tags = tags.unwrap_or_default();
            self.file.tracks.insert(
                path.to_string_lossy().into_owned(),
                Track {
                    modified,
                    title: tags.title,
                    artist: tags.artist,
                    album: tags.album,
                    genre: tags.genre,
                    track: tags.track,
                    disc: tags.disc,
                },
            );
        }
        if any && self.pending.is_empty() {
            self.save();
        }
        any
    }

    /// A tag's values and how many tracks have each, by name.
    pub fn values(&self, tag: Tag) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for track in self.file.tracks.values() {
            *counts.entry(track.value(tag).to_string()).or_default() += 1;
        }
        let mut values: Vec<_> = counts.into_iter().collect();
        values.sort_by(|a, b| {
            // Unknown last.
            (a.0 == tag.unknown())
                .cmp(&(b.0 == tag.unknown()))
                .then_with(|| natural_cmp(&a.0, &b.0))
        });
        values
    }

    /// The tracks with `tag` = `value`, in album, disc and track order.
    pub fn tracks(&self, tag: Tag, value: &str) -> Vec<(PathBuf, &Track)> {
        let mut tracks: Vec<_> = self
            .file
            .tracks
            .iter()
            .filter(|(_, t)| t.value(tag) == value)
            .map(|(k, t)| (PathBuf::from(k), t))
            .collect();
        tracks.sort_by(|a, b| {
            natural_cmp(a.1.value(Tag::Album), b.1.value(Tag::Album))
                .then(a.1.disc.unwrap_or(0).cmp(&b.1.disc.unwrap_or(0)))
                .then(a.1.track.unwrap_or(u32::MAX).cmp(&b.1.track.unwrap_or(u32::MAX)))
                .then_with(|| natural_cmp(&a.0.to_string_lossy(), &b.0.to_string_lossy()))
        });
        tracks
    }

    fn save(&self) {
        crate::state::write(self.path.as_deref(), &self.file);
    }
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(album: &str, artist: &str, n: Option<u32>) -> Track {
        Track {
            album: Some(album.into()),
            artist: Some(artist.into()),
            track: n,
            ..Default::default()
        }
    }

    #[test]
    fn groups_tracks_by_tag() {
        let mut index = MusicIndex {
            path: None,
            file: IndexFile::default(),
            tagger: None,
            pending: HashMap::new(),
            arrived: Arc::default(),
            scanned: true,
        };
        let t = &mut index.file.tracks;
        t.insert("/m/b2.mp3".into(), track("Beta", "Ann", Some(2)));
        t.insert("/m/b1.mp3".into(), track("Beta", "Ann", Some(1)));
        t.insert("/m/a1.mp3".into(), track("Alpha", "Bob", Some(1)));
        t.insert("/m/x.mp3".into(), Track::default());
        assert_eq!(
            index.values(Tag::Album),
            [("Alpha".into(), 1), ("Beta".into(), 2), ("Unknown Album".into(), 1)]
        );
        let beta: Vec<_> = index.tracks(Tag::Album, "Beta").into_iter().map(|(p, _)| p).collect();
        assert_eq!(beta, [PathBuf::from("/m/b1.mp3"), PathBuf::from("/m/b2.mp3")]);
        assert_eq!(index.tracks(Tag::Artist, "Unknown Artist").len(), 1);
    }
}
