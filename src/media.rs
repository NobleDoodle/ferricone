//! What the player opens: local files and network locations, and the queue
//! they play from. Command lines, opened folders, drops and typed links all
//! become a list of `Source`s here.

use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    /// Anything else VLC opens by URL: http://, rtsp://, smb://, …
    Location(String),
}

/// Extensions a folder is searched for: VLC's common video and audio formats,
/// not playlists or subtitles.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "3g2", "3gp", "asf", "avi", "divx", "f4v", "flv", "m2t", "m2ts", "m2v", "m4v", "mkv", "mov",
    "mp4", "mpeg", "mpg", "mts", "mxf", "nut", "ogm", "ogv", "rm", "rmvb", "ts", "vob", "webm",
    "wmv", "aac", "ac3", "aif", "aiff", "alac", "amr", "ape", "au", "dts", "flac", "m4a", "m4b",
    "mka", "mp2", "mp3", "mpc", "oga", "ogg", "opus", "spx", "tta", "wav", "wma", "wv",
];
/// Where the audio formats start in `MEDIA_EXTENSIONS`.
const FIRST_AUDIO: usize = 27;

/// Folders are searched this deep for media.
const MAX_DEPTH: usize = 8;

impl Source {
    /// A path, or a URL with a scheme; `file://` URLs become paths.
    pub fn parse(text: &str) -> Option<Source> {
        let text = text.trim();
        if text.is_empty() {
            None
        } else if let Some(path) = file_url_path(text) {
            Some(Source::File(path))
        } else if has_scheme(text) {
            Some(Source::Location(text.into()))
        } else {
            Some(Source::File(text.into()))
        }
    }

    /// The name shown as the title: the file name, or the URL's last segment.
    pub fn title(&self) -> String {
        match self {
            Source::File(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            Source::Location(url) => {
                let path = url_parts(url).1;
                let last = path.rsplit('/').find(|s| !s.is_empty());
                last.map(|s| String::from_utf8_lossy(&percent_decode(s)).into_owned())
                    .unwrap_or_else(|| url.clone())
            }
        }
    }

    /// The breadcrumb: the file's folder, or the URL's host.
    pub fn folder(&self) -> String {
        match self {
            Source::File(path) => path
                .canonicalize()
                .ok()
                .as_deref()
                .and_then(|p| p.parent()?.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Source::Location(url) => url_parts(url).0.into(),
        }
    }
}

/// Replaces folders with the media files in them, in name order, and
/// playlists with their entries (folders in them too, playlists in them
/// not); files and locations stay as they are.
pub fn expand(sources: impl IntoIterator<Item = Source>) -> Vec<Source> {
    let mut out = Vec::new();
    for source in sources {
        match source {
            Source::File(path) if crate::playlists::is_playlist(&path) => {
                for entry in crate::playlists::load(&path) {
                    match entry {
                        Source::File(p) if crate::playlists::is_playlist(&p) => {}
                        Source::File(p) if p.is_dir() => scan(&p, MAX_DEPTH, &mut out),
                        other => out.push(other),
                    }
                }
            }
            Source::File(path) if path.is_dir() => scan(&path, MAX_DEPTH, &mut out),
            other => out.push(other),
        }
    }
    out
}

fn scan(dir: &Path, depth: usize, out: &mut Vec<Source>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    entries.sort_by(|a, b| {
        natural_cmp(
            &a.file_name().to_string_lossy(),
            &b.file_name().to_string_lossy(),
        )
    });
    for entry in entries {
        let path = entry.path();
        // Symlinked folders are not followed, so links can't loop.
        let real_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if real_dir {
            if depth > 0 {
                scan(&path, depth - 1, out);
            }
        } else if is_media(&path) {
            out.push(Source::File(path));
        }
    }
}

/// The media file after `file` in its folder, in name order: the next
/// episode. Hidden files and subfolders are skipped.
pub fn next_in_folder(file: &Path) -> Option<PathBuf> {
    let name = file.file_name()?.to_string_lossy().into_owned();
    let mut siblings: Vec<String> = std::fs::read_dir(file.parent()?)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.') && is_media(Path::new(n)))
        .collect();
    siblings.sort_by(|a, b| natural_cmp(a, b));
    let next = siblings.iter().find(|n| natural_cmp(n, &name).is_gt())?;
    Some(file.with_file_name(next))
}

pub fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Audio only, by extension: the library shows it under Music.
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA_EXTENSIONS[FIRST_AUDIO..].contains(&e.to_ascii_lowercase().as_str()))
}

/// Fisher–Yates on xorshift64: shuffling needs no stronger randomness.
pub fn shuffle<T>(items: &mut [T], mut seed: u64) {
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// `file://` with the path percent-encoded, for libVLC and D-Bus.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Typed or pasted locations, one per line.
pub fn from_lines(text: &str) -> Vec<Source> {
    text.lines().filter_map(Source::parse).collect()
}

/// A drop's `text/uri-list` (RFC 2483): one URI per line, `#` comments.
pub fn from_uri_list(text: &str) -> Vec<Source> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(Source::parse)
        .collect()
}

/// Name order with numbers compared by value: "Episode 2" before "Episode 10".
/// Case only breaks ties.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    natural_cmp_folded(a, b).then_with(|| a.cmp(b))
}

fn natural_cmp_folded(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(d) = it.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    digits
                };
                let (x, y) = (number(&mut a), number(&mut b));
                let (xt, yt) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let by_value = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if by_value != Ordering::Equal {
                    return by_value;
                }
            }
            (Some(x), Some(y)) => {
                let by_letter = x.to_lowercase().cmp(y.to_lowercase());
                if by_letter != Ordering::Equal {
                    return by_letter;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// `scheme:` per RFC 3986, at least two letters so `C:\` stays a path.
fn has_scheme(text: &str) -> bool {
    text.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

/// (host, path) of `scheme://host/path?query`.
fn url_parts(url: &str) -> (&str, &str) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    rest.split_once('/')
        .map_or((rest, ""), |(host, path)| (host, path))
}

/// The local path of a `file://` URL on this machine.
fn file_url_path(text: &str) -> Option<PathBuf> {
    let rest = text
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .map(|_| &text[7..])?;
    let (host, path) = rest.split_once('/')?;
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    let bytes = percent_decode(&format!("/{path}"));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        // file:///C:/dir → C:/dir
        let path = String::from_utf8_lossy(&bytes).into_owned();
        Some(PathBuf::from(path.strip_prefix('/').unwrap_or(&path)))
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

/// What happens when an item ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    /// Stop after the last item.
    #[default]
    Off,
    /// Start over after the last item.
    All,
    /// Play the same item again.
    One,
}

/// The play order: what was opened, which item is playing, and repeat.
#[derive(Default)]
pub struct Queue {
    items: Vec<Source>,
    current: usize,
    pub repeat: Repeat,
}

impl Queue {
    /// Replaces the queue; returns the first item.
    pub fn replace(&mut self, items: Vec<Source>) -> Option<&Source> {
        (self.items, self.current) = (items, 0);
        self.current()
    }

    /// Adds items at the end.
    pub fn append(&mut self, items: Vec<Source>) {
        self.items.extend(items);
    }

    pub fn items(&self) -> &[Source] {
        &self.items
    }

    pub fn current(&self) -> Option<&Source> {
        self.items.get(self.current)
    }

    pub fn current_index(&self) -> Option<usize> {
        (self.current < self.items.len()).then_some(self.current)
    }

    /// The item to play when the current one ends, as repeat says.
    pub fn advance(&mut self) -> Option<&Source> {
        match self.repeat {
            Repeat::One => self.current(),
            _ => self.next(),
        }
    }

    /// Moves to the next item; after the last, to the first when repeating.
    pub fn next(&mut self) -> Option<&Source> {
        if self.current + 1 < self.items.len() {
            self.current += 1;
        } else if self.repeat == Repeat::All && !self.items.is_empty() {
            self.current = 0;
        } else {
            return None;
        }
        self.current()
    }

    /// Moves to the previous item; before the first, to the last when
    /// repeating.
    pub fn previous(&mut self) -> Option<&Source> {
        if self.current > 0 {
            self.current -= 1;
        } else if self.repeat == Repeat::All && self.items.len() > 1 {
            self.current = self.items.len() - 1;
        } else {
            return None;
        }
        self.current()
    }

    /// Makes item `index` the current one.
    pub fn play_at(&mut self, index: usize) -> Option<&Source> {
        if index >= self.items.len() {
            return None;
        }
        self.current = index;
        self.current()
    }

    pub fn has_next(&self) -> bool {
        self.current + 1 < self.items.len() || (self.repeat == Repeat::All && self.items.len() > 1)
    }

    pub fn has_previous(&self) -> bool {
        self.current > 0 || (self.repeat == Repeat::All && self.items.len() > 1)
    }

    /// Removes item `index`; true when that was the one playing (the next
    /// one, if any, is current then).
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.items.remove(index);
        if index < self.current {
            self.current -= 1;
        }
        let was_current = index == self.current;
        if self.current >= self.items.len() {
            self.current = self.items.len().saturating_sub(1);
        }
        was_current
    }

    /// Moves item `from` to `to`; the playing item stays current.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        self.current = if self.current == from {
            to
        } else if from < self.current && to >= self.current {
            self.current - 1
        } else if from > self.current && to <= self.current {
            self.current + 1
        } else {
            self.current
        };
    }

    /// Sorts by title, numbers by value; the playing item stays current.
    pub fn sort_by_title(&mut self) {
        self.reorder(|items| items.sort_by(|a, b| natural_cmp(&a.1.title(), &b.1.title())));
    }

    /// Shuffles what comes after the playing item, from `seed`.
    pub fn shuffle_rest(&mut self, seed: u64) {
        let rest = (self.current + 1).min(self.items.len());
        shuffle(&mut self.items[rest..], seed);
    }

    /// Reorders the items while keeping the playing one current.
    fn reorder(&mut self, f: impl FnOnce(&mut Vec<(usize, Source)>)) {
        let mut tagged: Vec<(usize, Source)> = self.items.drain(..).enumerate().collect();
        f(&mut tagged);
        let current = self.current;
        self.current = tagged.iter().position(|(i, _)| *i == current).unwrap_or(0);
        self.items = tagged.into_iter().map(|(_, s)| s).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths_and_urls() {
        assert_eq!(
            Source::parse("file:///home/a/My%20Clip.mkv"),
            Some(Source::File("/home/a/My Clip.mkv".into()))
        );
        assert_eq!(
            Source::parse("file://localhost/tmp/x.mp4"),
            Some(Source::File("/tmp/x.mp4".into()))
        );
        assert_eq!(
            Source::parse(" https://example.com/stream.avi "),
            Some(Source::Location("https://example.com/stream.avi".into()))
        );
        assert_eq!(
            Source::parse("clips/a.mp4"),
            Some(Source::File("clips/a.mp4".into()))
        );
        assert_eq!(
            Source::parse(r"C:\a.mp4"),
            Some(Source::File(r"C:\a.mp4".into()))
        );
        assert_eq!(Source::parse("  "), None);
    }

    #[test]
    fn reads_uri_lists() {
        let list = "# from a file manager\r\nfile:///a/b%23c.mp4\r\nhttp://h/x.ts\r\n";
        assert_eq!(
            from_uri_list(list),
            [
                Source::File("/a/b#c.mp4".into()),
                Source::Location("http://h/x.ts".into())
            ]
        );
    }

    #[test]
    fn titles_locations() {
        let url = Source::Location("https://example.com/live/My%20Show.m3u8?token=1".into());
        assert_eq!(url.title(), "My Show.m3u8");
        assert_eq!(url.folder(), "example.com");
        assert_eq!(
            Source::Location("rtsp://cam.local".into()).title(),
            "rtsp://cam.local"
        );
    }

    #[test]
    fn sorts_numbers_by_value() {
        let mut names = [
            "Ep 10.mkv",
            "ep 2.mkv",
            "Ep 1.mkv",
            "Ep 02b.mkv",
            "Extra.mkv",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "Ep 1.mkv",
                "ep 2.mkv",
                "Ep 02b.mkv",
                "Ep 10.mkv",
                "Extra.mkv"
            ]
        );
    }

    fn files(names: &[&str]) -> Vec<Source> {
        names.iter().map(|n| Source::File((*n).into())).collect()
    }

    fn titles(q: &Queue) -> Vec<String> {
        q.items().iter().map(Source::title).collect()
    }

    #[test]
    fn repeats_and_wraps() {
        let mut q = Queue::default();
        q.replace(files(&["a", "b"]));
        q.repeat = Repeat::One;
        assert_eq!(q.advance().map(Source::title).as_deref(), Some("a"));
        assert_eq!(q.next().map(Source::title).as_deref(), Some("b"));
        q.repeat = Repeat::All;
        assert!(q.has_next());
        assert_eq!(q.advance().map(Source::title).as_deref(), Some("a"));
        assert_eq!(q.previous().map(Source::title).as_deref(), Some("b"));
    }

    #[test]
    fn edits_keep_the_playing_item() {
        let mut q = Queue::default();
        q.replace(files(&["e10", "e2", "e1", "e3"]));
        q.play_at(1); // e2
        q.sort_by_title();
        assert_eq!(titles(&q), ["e1", "e2", "e3", "e10"]);
        assert_eq!(q.current_index(), Some(1));
        q.move_item(3, 0); // e10 to the front
        assert_eq!(q.current().map(Source::title).as_deref(), Some("e2"));
        q.move_item(2, 3); // e2 to the end
        assert_eq!(q.current_index(), Some(3));
        assert!(!q.remove(0));
        assert_eq!(q.current().map(Source::title).as_deref(), Some("e2"));
        assert!(q.remove(2)); // the playing one, last: the one before is current
        assert_eq!(q.current_index(), Some(1));
        q.append(files(&["x", "y", "z"]));
        q.play_at(0);
        q.shuffle_rest(42);
        assert_eq!(q.items().len(), 5);
        assert_eq!(q.current_index(), Some(0));
    }

    #[test]
    fn finds_the_next_episode() {
        let dir = std::env::temp_dir().join(format!("ferricone-next-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Extras.mkv")).unwrap();
        for name in [
            "S01E10.mkv",
            "S01E2.mkv",
            "S01E1.mkv",
            "notes.txt",
            ".S01E3.mkv",
        ] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let next =
            |n: &str| next_in_folder(&dir.join(n)).map(|p| p.file_name().unwrap().to_owned());
        assert_eq!(next("S01E1.mkv").unwrap(), "S01E2.mkv");
        assert_eq!(next("S01E2.mkv").unwrap(), "S01E10.mkv");
        assert_eq!(next("S01E10.mkv"), None); // the folder and the hidden file don't count
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn steps_through_the_queue() {
        let mut q = Queue::default();
        assert_eq!(q.replace(vec![]), None);
        q.replace(vec![Source::File("a".into()), Source::File("b".into())]);
        assert!(q.has_next() && !q.has_previous());
        assert_eq!(q.next(), Some(&Source::File("b".into())));
        assert_eq!(q.next(), None);
        assert_eq!(q.current(), Some(&Source::File("b".into())));
        assert_eq!(q.previous(), Some(&Source::File("a".into())));
        assert_eq!(q.previous(), None);
    }
}
