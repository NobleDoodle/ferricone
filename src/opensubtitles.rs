//! A small client for OpenSubtitles' REST API (api.opensubtitles.com,
//! v1): search by the file's OpenSubtitles hash and a query, and download.
//! VLC's own VLSub is a Lua add-on of its Qt interface, out of libVLC's
//! reach. Each user brings an API key from their opensubtitles.com account;
//! logging in raises the download quota. Calls block, so run them off the
//! UI thread.

use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};

const API: &str = "https://api.opensubtitles.com/api/v1";
/// The API asks for "AppName vVersion".
const USER_AGENT: &str = concat!("Ferricone v", env!("CARGO_PKG_VERSION"));

/// One subtitle file found.
#[derive(Clone, Debug)]
pub struct Found {
    pub file_id: u64,
    pub file_name: String,
    pub language: String,
    pub release: String,
    pub downloads: u64,
    /// Made for this exact file (its hash matched).
    pub exact: bool,
}

pub struct Client {
    agent: ureq::Agent,
    key: String,
    token: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

/// The API's message from an error reply, or the error itself.
fn explain(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(401 | 403) => "OpenSubtitles refused the API key or login".into(),
        ureq::Error::StatusCode(406) => "OpenSubtitles' download limit for today is reached".into(),
        ureq::Error::StatusCode(429) => "OpenSubtitles asks to slow down; try again shortly".into(),
        ureq::Error::StatusCode(code) => format!("OpenSubtitles answered {code}"),
        e => format!("couldn't reach OpenSubtitles: {e}"),
    }
}

impl Client {
    /// A client with `key`, logged in when a username is given.
    pub fn new(key: &str, username: &str, password: &str) -> Result<Self, String> {
        if key.trim().is_empty() {
            return Err("Add your OpenSubtitles API key in Settings › Subtitles first.".into());
        }
        let mut client = Self {
            agent: agent(),
            key: key.trim().into(),
            token: None,
        };
        if !username.trim().is_empty() {
            let body = serde_json::json!({ "username": username.trim(), "password": password });
            let reply = client.post("/login", &body)?;
            client.token = reply["token"].as_str().map(String::from);
        }
        Ok(client)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let mut req = self
            .agent
            .post(format!("{API}{path}"))
            .header("Api-Key", &self.key)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .content_type("application/json");
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let text = req
            .send(body.to_string())
            .map_err(explain)?
            .body_mut()
            .read_to_string()
            .map_err(explain)?;
        serde_json::from_str(&text).map_err(|e| format!("OpenSubtitles sent something unreadable: {e}"))
    }

    /// Subtitles for `video` in `language` (an ISO code, "en"): by its hash
    /// and `query` (the title, "Show S01E02").
    pub fn search(&self, query: &str, language: &str, video: Option<&Path>) -> Result<Vec<Found>, String> {
        let hash = video.and_then(|v| movie_hash(v).ok());
        let mut req = self
            .agent
            .get(format!("{API}/subtitles"))
            .header("Api-Key", &self.key)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .query("languages", language)
            .query("query", query.trim().to_lowercase());
        if let Some(hash) = &hash {
            req = req.query("moviehash", hash);
        }
        let text = req
            .call()
            .map_err(explain)?
            .body_mut()
            .read_to_string()
            .map_err(explain)?;
        let reply: Value =
            serde_json::from_str(&text).map_err(|e| format!("OpenSubtitles sent something unreadable: {e}"))?;
        Ok(parse_search(&reply))
    }

    /// Downloads a file's text: asks for a link, then fetches it.
    pub fn download(&self, file_id: u64) -> Result<Vec<u8>, String> {
        let reply = self.post("/download", &serde_json::json!({ "file_id": file_id }))?;
        let link = reply["link"]
            .as_str()
            .ok_or("OpenSubtitles gave no download link")?;
        let mut response = self.agent.get(link).header("User-Agent", USER_AGENT).call().map_err(explain)?;
        response.body_mut().read_to_vec().map_err(explain)
    }
}

/// The search reply's files, the exact ones and the most downloaded first.
fn parse_search(reply: &Value) -> Vec<Found> {
    let mut out = Vec::new();
    for item in reply["data"].as_array().into_iter().flatten() {
        let a = &item["attributes"];
        for file in a["files"].as_array().into_iter().flatten() {
            let Some(file_id) = file["file_id"].as_u64() else { continue };
            out.push(Found {
                file_id,
                file_name: file["file_name"].as_str().unwrap_or_default().into(),
                language: a["language"].as_str().unwrap_or_default().into(),
                release: a["release"].as_str().unwrap_or_default().into(),
                downloads: a["download_count"].as_u64().unwrap_or(0),
                exact: a["moviehash_match"].as_bool().unwrap_or(false),
            });
        }
    }
    out.sort_by(|a, b| b.exact.cmp(&a.exact).then(b.downloads.cmp(&a.downloads)));
    out
}

/// OpenSubtitles' hash: the file size plus the 64-bit little-endian sums of
/// its first and last 64 KiB, as 16 hex digits.
pub fn movie_hash(path: &Path) -> std::io::Result<String> {
    const CHUNK: u64 = 64 * 1024;
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size < CHUNK {
        return Err(std::io::Error::other("too small to hash"));
    }
    let mut hash = size;
    let mut buffer = vec![0u8; CHUNK as usize];
    for start in [0, size - CHUNK] {
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer)?;
        for word in buffer.as_chunks::<8>().0 {
            hash = hash.wrapping_add(u64::from_le_bytes(*word));
        }
    }
    Ok(format!("{hash:016x}"))
}

/// The languages offered, as OpenSubtitles codes and names.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt-pt", "Portuguese"),
    ("pt-br", "Portuguese (Brazil)"),
    ("nl", "Dutch"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("no", "Norwegian"),
    ("fi", "Finnish"),
    ("pl", "Polish"),
    ("cs", "Czech"),
    ("hu", "Hungarian"),
    ("ro", "Romanian"),
    ("el", "Greek"),
    ("tr", "Turkish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("ar", "Arabic"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh-cn", "Chinese (Simplified)"),
    ("zh-tw", "Chinese (Traditional)"),
    ("vi", "Vietnamese"),
    ("th", "Thai"),
    ("id", "Indonesian"),
];

/// "Show Name S01E02" from "Show.Name.S01E02.1080p.mkv": separators to
/// spaces, up to the episode code or the first quality tag.
pub fn query_for(video: &Path) -> String {
    let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let spaced: String = stem
        .chars()
        .map(|c| if matches!(c, '.' | '_' | '-') { ' ' } else { c })
        .collect();
    let mut words = Vec::new();
    for word in spaced.split_whitespace() {
        let lower = word.to_lowercase();
        let tag = ["1080p", "720p", "2160p", "480p", "x264", "x265", "hevc", "bluray", "web", "webrip", "hdtv"]
            .contains(&lower.as_str());
        if tag || word.starts_with('[') || word.starts_with('(') && !word.trim_matches(['(', ')']).chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        words.push(word.to_string());
        if crate::addsubs::episode(word).is_some_and(|(season, _)| season.is_some()) {
            break;
        }
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_drop_release_tags() {
        assert_eq!(query_for(Path::new("/v/The.Wand.S01E05.1080p.WEB.mkv")), "The Wand S01E05");
        assert_eq!(query_for(Path::new("/v/Some Film (2014) [1080p].mkv")), "Some Film (2014)");
    }

    #[test]
    fn reads_search_replies() {
        let reply = serde_json::json!({ "data": [
            { "attributes": { "language": "en", "release": "WEB", "download_count": 10,
                "moviehash_match": false, "files": [{ "file_id": 1, "file_name": "a.srt" }] } },
            { "attributes": { "language": "en", "release": "BluRay", "download_count": 3,
                "moviehash_match": true, "files": [{ "file_id": 2, "file_name": "b.srt" }] } },
        ]});
        let found = parse_search(&reply);
        assert_eq!(found.iter().map(|f| f.file_id).collect::<Vec<_>>(), [2, 1]);
        assert!(found[0].exact);
    }

    #[test]
    fn hashes_like_opensubtitles() {
        // 128 KiB of zeros: the sum is just the size.
        let dir = std::env::temp_dir().join(format!("ferricone-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("zeros.bin");
        std::fs::write(&file, vec![0u8; 128 * 1024]).unwrap();
        assert_eq!(movie_hash(&file).unwrap(), format!("{:016x}", 128 * 1024));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
