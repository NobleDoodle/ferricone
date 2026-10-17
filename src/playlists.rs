//! Saved playlists: M3U8 files (one path or URL per line) in the user's data
//! folder (`$XDG_DATA_HOME/ferricone/playlists` on Linux), so other players
//! read them too. Saved network streams, the library's Network page, are
//! kept the same way in `streams` beside it. A cover, when chosen, sits beside its playlist as
//! `<name>.<png|jpg|…>`.

use crate::media::Source;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const EXTENSIONS: &[&str] = &["m3u", "m3u8"];

#[derive(Clone, Debug)]
pub struct Playlist {
    pub name: String,
    pub path: PathBuf,
}

pub fn folder() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
        .map(|dirs| dirs.data_dir().join("playlists"))
}

pub fn streams_folder() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
        .map(|dirs| dirs.data_dir().join("streams"))
}

/// The saved playlists, by name.
pub fn list() -> Vec<Playlist> {
    folder().map_or_else(Vec::new, |dir| list_in(&dir))
}

/// The playlists in `dir`, by name.
pub fn list_in(dir: &Path) -> Vec<Playlist> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut lists: Vec<Playlist> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_playlist(p))
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some(Playlist { name, path })
        })
        .collect();
    lists.sort_by_key(|l| l.name.to_lowercase());
    lists
}

pub fn is_playlist(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The entries of an M3U playlist; relative paths are relative to it.
pub fn load(path: &Path) -> Vec<Source> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let base = path.parent().unwrap_or(Path::new(""));
    parse(&text)
        .into_iter()
        .map(|source| match source {
            Source::File(p) if p.is_relative() => Source::File(base.join(p)),
            other => other,
        })
        .collect()
}

fn parse(text: &str) -> Vec<Source> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(Source::parse)
        .collect()
}

/// Saves `items` as playlist `name` (replacing one of that name), with
/// `cover` copied beside it; returns where it went.
pub fn save(name: &str, items: &[Source], cover: Option<&Path>) -> Result<PathBuf, String> {
    save_in(&folder().ok_or("there is no data folder")?, name, items, cover)
}

/// `save`, into `dir`.
pub fn save_in(
    dir: &Path,
    name: &str,
    items: &[Source],
    cover: Option<&Path>,
) -> Result<PathBuf, String> {
    let file_name = safe_name(name).ok_or("the name is empty")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{file_name}.m3u8"));
    fs::write(&path, write(items)).map_err(|e| e.to_string())?;
    if let Some(cover) = cover
        && let Some(ext) = cover.extension()
    {
        let target = dir.join(&file_name).with_extension(ext);
        fs::copy(cover, target).map_err(|e| format!("the cover: {e}"))?;
    }
    Ok(path)
}

fn write(items: &[Source]) -> String {
    let mut text = String::from("#EXTM3U\n");
    for item in items {
        text.push_str(&format!("#EXTINF:-1,{}\n", item.title()));
        match item {
            Source::File(path) => text.push_str(&path.to_string_lossy()),
            Source::Location(url) => text.push_str(url),
        }
        text.push('\n');
    }
    text
}

/// The playlist's name as a file name: path separators and control
/// characters dropped, trimmed; `None` when nothing is left.
fn safe_name(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .collect();
    let name = name.trim().trim_start_matches('.').trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_m3u() {
        let items = vec![
            Source::File("/videos/a b.mkv".into()),
            Source::Location("https://example.com/live.m3u8".into()),
        ];
        let text = write(&items);
        assert!(text.starts_with("#EXTM3U\n#EXTINF:-1,a b.mkv\n/videos/a b.mkv\n"));
        assert_eq!(parse(&text), items);
    }

    #[test]
    fn reads_relative_entries_beside_the_playlist() {
        let dir = std::env::temp_dir().join(format!("ferricone-m3u-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let list = dir.join("season.m3u");
        fs::write(
            &list,
            "S01E01.mkv\r\n# a comment\n\nfile:///abs/S01E02.mkv\n",
        )
        .unwrap();
        assert_eq!(
            load(&list),
            [
                Source::File(dir.join("S01E01.mkv")),
                Source::File("/abs/S01E02.mkv".into())
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_safely() {
        assert_eq!(safe_name(" Cool/Songs: 2 ").as_deref(), Some("CoolSongs 2"));
        assert_eq!(safe_name("../x").as_deref(), Some("x"));
        assert_eq!(safe_name(" \t "), None);
    }
}
