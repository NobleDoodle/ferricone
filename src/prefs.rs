//! App-wide preferences, kept in `app.toml` in the config folder: the
//! appearance (theme.rs), whether the introduction was seen, and the
//! Settings window's choices (preferences.rs). Per-folder playback choices
//! stay in presets.toml. One copy lives on the UI thread; `update` changes
//! and saves it.

use crate::media::Source;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, path::PathBuf};

/// Light, Dark, or the desktop's preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    #[default]
    Dark,
    System,
}

/// How much libVLC writes to the terminal and the log file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Nothing,
    #[default]
    Errors,
    Warnings,
    Everything,
}

/// When the bars hide over the video.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoHide {
    #[default]
    Fullscreen,
    Never,
}

/// Continuing an item where it was left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Resume {
    /// The resume pill offers it.
    #[default]
    Ask,
    Always,
    Never,
}

/// When the last item in the queue ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Finish {
    /// The Up next pill offers the next file in the folder.
    #[default]
    OfferNext,
    PlayNext,
    Stop,
    Quit,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Prefs {
    pub onboarded: bool,
    // Appearance
    pub mode: Mode,
    /// "#rrggbb".
    pub accent: String,
    /// Take Omarchy's colors (only offered where it is installed).
    pub omarchy: bool,
    // General: diagnostics (libVLC options, so from the next start)
    pub log: bool,
    pub log_file: String,
    pub log_level: LogLevel,
    /// More libVLC options, as on its command line.
    pub extra_args: String,
    // Interface
    pub title_shows_file: bool,
    pub auto_hide: AutoHide,
    /// Seconds without moving the pointer before the bars (and the pointer)
    /// hide over the video.
    pub hide_after: f32,
    // Privacy
    pub save_history: bool,
    pub remember_position: bool,
    /// Comma-separated words; items whose path or address contains one are
    /// not remembered.
    pub never_remember: String,
    pub fetch_art: bool,
    // Playback
    pub resume: Resume,
    pub autoplay: bool,
    pub stop_after_each: bool,
    pub on_finish: Finish,
    pub precise_seek: bool,
    pub scrub_preview: bool,
    // System
    /// Milliseconds libVLC buffers network streams.
    pub network_caching: u32,
    pub hardware_decoding: bool,
    /// Where recordings go; empty is libVLC's choice (the Videos folder).
    pub record_dir: String,
    /// Changed keys, by action (shortcuts.rs); "" leaves one unassigned.
    pub keys: std::collections::BTreeMap<String, String>,
    // OpenSubtitles (opensubtitles.rs): the user's API key and optional
    // login, and the language searched for.
    pub os_key: String,
    pub os_username: String,
    pub os_password: String,
    pub os_language: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            onboarded: false,
            mode: Mode::Dark,
            accent: "#d37918".into(),
            omarchy: true,
            log: false,
            log_file: default_log_file(),
            log_level: LogLevel::Errors,
            extra_args: String::new(),
            title_shows_file: true,
            auto_hide: AutoHide::Fullscreen,
            hide_after: 2.0,
            save_history: true,
            remember_position: true,
            never_remember: String::new(),
            fetch_art: false,
            resume: Resume::Ask,
            autoplay: true,
            stop_after_each: false,
            on_finish: Finish::OfferNext,
            precise_seek: true,
            scrub_preview: true,
            network_caching: NETWORK_CACHING,
            hardware_decoding: true,
            record_dir: String::new(),
            keys: Default::default(),
            os_key: String::new(),
            os_username: String::new(),
            os_password: String::new(),
            os_language: "en".into(),
        }
    }
}

/// libVLC's own default.
pub const NETWORK_CACHING: u32 = 1000;

fn default_log_file() -> String {
    crate::state::state_path("ferricone.log")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")
        .map(|d| d.config_dir().join("app.toml"))
}

thread_local! {
    static PREFS: RefCell<Prefs> = RefCell::new(crate::state::read(path().as_deref()));
}

/// The preferences now.
pub fn get() -> Prefs {
    PREFS.with(|p| p.borrow().clone())
}

/// Changes the preferences and saves them.
pub fn update(change: impl FnOnce(&mut Prefs)) {
    PREFS.with(|p| {
        let mut prefs = p.borrow_mut();
        change(&mut prefs);
        let path = path();
        crate::state::write(path.as_deref(), &*prefs);
        // It can hold an OpenSubtitles password: only the user may read it.
        #[cfg(unix)]
        if let Some(path) = &path {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    });
}

/// Back to the defaults, keeping that the introduction was seen.
pub fn reset() {
    update(|p| {
        *p = Prefs {
            onboarded: p.onboarded,
            ..Prefs::default()
        };
    });
}

impl Prefs {
    /// `source` is one the user asked never to remember.
    pub fn excluded(&self, source: &Source) -> bool {
        let text = match source {
            Source::File(path) => path.to_string_lossy().to_lowercase(),
            Source::Location(url) => url.to_lowercase(),
        };
        self.never_remember
            .split(',')
            .map(|w| w.trim().to_lowercase())
            .any(|w| !w.is_empty() && text.contains(&w))
    }

    /// The libVLC options the preferences make, before FERRICONE_VLC_ARGS.
    pub fn vlc_args(&self) -> Vec<String> {
        let mut args: Vec<String> = match self.log_level {
            LogLevel::Nothing => vec!["--quiet".into()],
            LogLevel::Errors => vec!["--verbose=0".into()],
            LogLevel::Warnings => vec!["--verbose=1".into()],
            LogLevel::Everything => vec!["--verbose=2".into()],
        };
        if self.log && !self.log_file.is_empty() {
            args.push("--file-logging".into());
            args.push(format!("--logfile={}", self.log_file));
        }
        if self.fetch_art {
            args.push("--metadata-network-access".into());
        }
        if self.network_caching != NETWORK_CACHING {
            args.push(format!("--network-caching={}", self.network_caching));
        }
        if !self.hardware_decoding {
            args.push("--avcodec-hw=none".into());
        }
        args.extend(self.extra_args.split_whitespace().map(String::from));
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_remember_matches_words_in_paths() {
        let prefs = Prefs {
            never_remember: "Private, .tmp ,".into(),
            ..Prefs::default()
        };
        assert!(prefs.excluded(&Source::File("/home/a/private/x.mkv".into())));
        assert!(prefs.excluded(&Source::File("/a/clip.TMP".into())));
        assert!(!prefs.excluded(&Source::File("/a/show.mkv".into())));
    }

    #[test]
    fn preferences_make_vlc_options() {
        let prefs = Prefs {
            log: true,
            log_file: "/tmp/f.log".into(),
            log_level: LogLevel::Nothing,
            extra_args: "--no-osd  --x".into(),
            ..Prefs::default()
        };
        assert_eq!(
            prefs.vlc_args(),
            ["--quiet", "--file-logging", "--logfile=/tmp/f.log", "--no-osd", "--x"]
        );
    }

    #[test]
    fn old_files_keep_their_appearance() {
        let prefs: Prefs = toml::from_str("onboarded = true\nmode = \"light\"\n").unwrap();
        assert!(prefs.onboarded && prefs.mode == Mode::Light && prefs.save_history);
    }
}
