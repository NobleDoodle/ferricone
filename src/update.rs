//! Updates: Settings › General › Updates, and the title bar's notice.
//!
//! At most once a day (Check for updates, on by default) Ferricone asks
//! GitHub for its newest release; dev builds also see prereleases. One click
//! downloads that release's package for this copy, checks it against
//! SHA256SUMS, whose Ed25519 signature must match the release key built in
//! (packaging/update-key.pub), and installs it the way this copy came:
//!
//! - the AppImage replaces its own file,
//! - the tarball's install.sh replaces ~/.local/opt/ferricone,
//! - a .deb, .rpm or Arch package goes through the package manager, with the
//!   desktop's password prompt (pkexec).
//!
//! A copy built from source only points to the release page. Then Restart
//! starts the new version.

use crate::AppWindow;
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Weak};
use std::{
    cell::RefCell,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const RELEASES: &str = "https://api.github.com/repos/NobleDoodle/ferricone/releases?per_page=20";
const PAGE: &str = "https://github.com/NobleDoodle/ferricone/releases";
const DAY: u64 = 24 * 60 * 60;
/// After start, so the check doesn't compete with opening a file.
const FIRST_CHECK: Duration = Duration::from_secs(10);

/// The release key. A build may name another (a test key) through the
/// FERRICONE_UPDATE_KEY variable when it is compiled; release builds don't.
const KEY_PEM: &str = match option_env!("FERRICONE_UPDATE_KEY") {
    Some(pem) => pem,
    None => include_str!("../packaging/update-key.pub"),
};

pub fn current() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is semver")
}

// What is known ------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Release {
    pub version: semver::Version,
    pub page: String,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone, Debug)]
pub enum Status {
    /// Nothing checked yet in this run.
    Idle,
    Checking,
    /// Up to date, as of this many seconds since the epoch.
    Current(u64),
    Available(Release),
    /// Percent done.
    Downloading(Release, u8),
    Installing(Release),
    Installed(Release),
    Failed(String, Option<Release>),
}

thread_local! {
    static STATUS: RefCell<Status> = const { RefCell::new(Status::Idle) };
}

pub fn status() -> Status {
    STATUS.with(|s| s.borrow().clone())
}

fn set(app: &AppWindow, status: Status) {
    app.set_update_notice(
        match &status {
            Status::Available(r) => format!("Update to {}", r.version),
            Status::Installed(_) => "Restart to update".into(),
            _ => String::new(),
        }
        .into(),
    );
    STATUS.with(|s| *s.borrow_mut() = status);
    crate::preferences::refresh(app);
}

fn set_later(ui: &Weak<AppWindow>, status: Status) {
    let _ = ui.upgrade_in_event_loop(move |app| set(&app, status));
}

/// The last check, kept across starts in update.toml.
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Saved {
    checked: u64,
}

fn saved_path() -> Option<PathBuf> {
    crate::state::state_path("update.toml")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// Starting and checking ----------------------------------------------------

pub fn start(app: &AppWindow) {
    install();
    let ui = app.as_weak();
    slint::Timer::single_shot(FIRST_CHECK, move || {
        if let Some(app) = ui.upgrade() {
            check_if_due(&app);
        }
    });
}

/// Checks when automatic checks are on and the last was a day ago.
pub fn check_if_due(app: &AppWindow) {
    let saved: Saved = crate::state::read(saved_path().as_deref());
    if crate::prefs::get().check_updates
        && now().saturating_sub(saved.checked) >= DAY
        && matches!(status(), Status::Idle | Status::Current(_) | Status::Failed(..))
    {
        check(app);
    }
}

pub fn check(app: &AppWindow) {
    if matches!(status(), Status::Checking | Status::Downloading(..) | Status::Installing(_)) {
        return;
    }
    set(app, Status::Checking);
    let ui = app.as_weak();
    std::thread::spawn(move || {
        let result = newest_release(&current());
        let checked = now();
        crate::state::write(saved_path().as_deref(), &Saved { checked });
        set_later(
            &ui,
            match result {
                Ok(Some(release)) => Status::Available(release),
                Ok(None) => Status::Current(checked),
                Err(e) => Status::Failed(e, None),
            },
        );
    });
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(timeout))
        .user_agent(format!(
            "Ferricone/{} (+https://github.com/NobleDoodle/ferricone)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .into()
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

fn newest_release(current: &semver::Version) -> Result<Option<Release>, String> {
    // A test server can stand in for GitHub; what it serves must still be
    // signed with the built-in key.
    let url = std::env::var("FERRICONE_UPDATE_URL").unwrap_or_else(|_| RELEASES.into());
    let text = agent(Duration::from_secs(30))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("couldn't reach GitHub: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("couldn't reach GitHub: {e}"))?;
    let releases: Vec<GitHubRelease> =
        serde_json::from_str(&text).map_err(|e| format!("GitHub's answer was unreadable: {e}"))?;
    Ok(newest(releases, current))
}

/// The newest release above `current`; prereleases only for a prerelease.
fn newest(releases: Vec<GitHubRelease>, current: &semver::Version) -> Option<Release> {
    releases
        .into_iter()
        .filter(|r| !r.draft && (!r.prerelease || !current.pre.is_empty()))
        .filter_map(|r| {
            let version = semver::Version::parse(r.tag_name.strip_prefix('v')?).ok()?;
            (version > *current && (version.pre.is_empty() || !current.pre.is_empty())).then(|| {
                Release {
                    version,
                    page: r.html_url,
                    assets: r.assets,
                }
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

// How this copy was installed ----------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// The AppImage file ($APPIMAGE).
    AppImage(PathBuf),
    /// The tarball's ~/.local/opt/ferricone.
    Tarball(PathBuf),
    Deb,
    Rpm,
    Pacman,
    /// Built from source, or a layout this doesn't know.
    Other,
}

/// How this copy was installed, as it was at start: once an update replaces
/// the running files, this process's own path reads as deleted.
pub fn install() -> Install {
    static INSTALL: std::sync::OnceLock<Install> = std::sync::OnceLock::new();
    INSTALL.get_or_init(detect).clone()
}

fn detect() -> Install {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_default();
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match classify(&exe, appimage, home.as_deref()) {
        Place::AppImage(file) => Install::AppImage(file),
        Place::Tarball(dir) => Install::Tarball(dir),
        Place::System => system_owner(),
        Place::Other => Install::Other,
    }
}

#[derive(Debug, PartialEq)]
enum Place {
    AppImage(PathBuf),
    Tarball(PathBuf),
    System,
    Other,
}

fn classify(exe: &Path, appimage: Option<PathBuf>, home: Option<&Path>) -> Place {
    if let Some(file) = appimage.filter(|f| f.is_absolute()) {
        return Place::AppImage(file);
    }
    if exe.starts_with("/opt/ferricone") {
        return Place::System;
    }
    match home.map(|h| h.join(".local/opt/ferricone")) {
        Some(dir) if exe.starts_with(&dir) => Place::Tarball(dir),
        _ => Place::Other,
    }
}

/// Which package manager installed /opt/ferricone.
fn system_owner() -> Install {
    if Path::new("/var/lib/dpkg/info/ferricone.list").exists() {
        return Install::Deb;
    }
    let pacman = std::fs::read_dir("/var/lib/pacman/local").is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("ferricone-"))
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
    });
    if pacman {
        return Install::Pacman;
    }
    let rpm = std::process::Command::new("rpm")
        .args(["-q", "--quiet", "ferricone"])
        .status()
        .is_ok_and(|s| s.success());
    if rpm { Install::Rpm } else { Install::Other }
}

/// The release file for this install on this processor.
fn asset_suffix(install: &Install, arch: &str) -> Option<String> {
    let deb_arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(match install {
        Install::AppImage(_) => format!("-{arch}.AppImage"),
        Install::Tarball(_) => format!("-{arch}.tar.gz"),
        Install::Deb => format!("_{deb_arch}.deb"),
        Install::Rpm => format!(".{arch}.rpm"),
        Install::Pacman => format!("-{arch}.pkg.tar.zst"),
        Install::Other => return None,
    })
}

/// Whether Update can install here, or only open the release page.
pub fn can_install() -> bool {
    asset_suffix(&install(), std::env::consts::ARCH).is_some()
}

// Updating -----------------------------------------------------------------

/// The button next to the status: check, update, or restart.
pub fn act(app: &AppWindow) {
    match status() {
        Status::Available(release) | Status::Failed(_, Some(release)) => {
            if can_install() {
                update(app, release);
            } else {
                open_page(&release.page);
            }
        }
        Status::Installed(_) => restart(app),
        Status::Idle | Status::Current(_) | Status::Failed(_, None) => check(app),
        Status::Checking | Status::Downloading(..) | Status::Installing(_) => {}
    }
}

pub fn open_page(url: &str) {
    #[cfg(target_os = "linux")]
    if let Err(e) = std::process::Command::new("xdg-open").arg(url).spawn() {
        eprintln!("ferricone: opening {url}: {e}");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = url;
}

/// The newer version found, if any.
pub fn news() -> Option<String> {
    match status() {
        Status::Available(r)
        | Status::Downloading(r, _)
        | Status::Installing(r)
        | Status::Installed(r)
        | Status::Failed(_, Some(r)) => Some(r.version.to_string()),
        _ => None,
    }
}

pub fn release_page() -> String {
    match status() {
        Status::Available(r)
        | Status::Downloading(r, _)
        | Status::Installing(r)
        | Status::Installed(r)
        | Status::Failed(_, Some(r)) => r.page,
        _ => PAGE.into(),
    }
}

fn update(app: &AppWindow, release: Release) {
    set(app, Status::Downloading(release.clone(), 0));
    let ui = app.as_weak();
    std::thread::spawn(move || {
        let install = install();
        let result = download_and_install(&ui, &install, &release);
        set_later(
            &ui,
            match result {
                Ok(()) => Status::Installed(release),
                Err(e) => Status::Failed(e, Some(release)),
            },
        );
    });
}

fn download_and_install(ui: &Weak<AppWindow>, install: &Install, release: &Release) -> Result<(), String> {
    let suffix = asset_suffix(install, std::env::consts::ARCH).ok_or("no package for this copy")?;
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(&suffix))
        .ok_or_else(|| format!("the release has no {suffix} file"))?;
    let (sums, signature) = (
        find("SHA256SUMS").ok_or("the release has no SHA256SUMS")?,
        find("SHA256SUMS.sig").ok_or("the release isn't signed")?,
    );
    let sums = fetch(&sums.browser_download_url)?;
    let signature = fetch(&signature.browser_download_url)?;
    verify_signature(&sums, &signature, &release_key()?)?;
    let expected = expected_hash(&String::from_utf8_lossy(&sums), &asset.name)
        .ok_or_else(|| format!("SHA256SUMS doesn't list {}", asset.name))?;

    // The AppImage is renamed over itself, so it downloads next to itself.
    let dir = match install {
        Install::AppImage(file) => file.parent().map(Path::to_path_buf).ok_or("the AppImage has no folder")?,
        _ => cache_dir().ok_or("no cache folder")?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!(".{}.part", asset.name));
    // apt, dnf, zypper and pacman tell a package by its name's ending.
    let verified = match install {
        Install::AppImage(_) => part.clone(),
        _ => dir.join(&asset.name),
    };
    let result = (|| {
        download(ui, release, &asset.browser_download_url, &part, &expected)?;
        std::fs::rename(&part, &verified).map_err(|e| format!("{}: {e}", verified.display()))?;
        set_later(ui, Status::Installing(release.clone()));
        install_file(install, &verified, &dir)
    })();
    let _ = std::fs::remove_file(&part);
    let _ = std::fs::remove_file(&verified);
    result
}

fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone").map(|d| d.cache_dir().join("update"))
}

/// A small file. Each download has its own agent, so none reuses a connection
/// the server has closed (an HTTP/1.0 server closes after every reply).
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut body = agent(Duration::from_secs(60))
        .get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?
        .into_body();
    body.with_config()
        .limit(1 << 20)
        .read_to_vec()
        .map_err(|e| format!("download failed: {e}"))
}

/// Into `file`, reporting progress, refusing anything but `expected`.
fn download(ui: &Weak<AppWindow>, release: &Release, url: &str, file: &Path, expected: &[u8; 32]) -> Result<(), String> {
    let response = agent(Duration::from_secs(60 * 60))
        .get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let mut reader = response.into_body().into_with_config().limit(u64::MAX).reader();
    let mut out = std::fs::File::create(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    let (mut done, mut shown) = (0u64, 0u8);
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = reader.read(&mut buffer).map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        out.write_all(&buffer[..n]).map_err(|e| format!("{}: {e}", file.display()))?;
        done += n as u64;
        if let Some(total) = total.filter(|&t| t > 0) {
            let percent = (done * 100 / total).min(100) as u8;
            if percent >= shown + 2 {
                shown = percent;
                set_later(ui, Status::Downloading(release.clone(), percent));
            }
        }
    }
    out.sync_all().map_err(|e| format!("{}: {e}", file.display()))?;
    if hash.finish().as_ref() != expected {
        return Err("the download doesn't match its checksum".into());
    }
    Ok(())
}

// Checking what was downloaded ---------------------------------------------

/// The raw Ed25519 key in a PEM public key (SubjectPublicKeyInfo).
fn release_key() -> Result<[u8; 32], String> {
    use base64::Engine;
    let body: String = KEY_PEM
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|_| "the built-in release key is unreadable")?;
    // SEQUENCE { SEQUENCE { OID 1.3.101.112 (Ed25519) }, BIT STRING { key } }
    const PREFIX: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
    match der.split_at_checked(PREFIX.len()) {
        Some((prefix, key)) if prefix == PREFIX => key.try_into().map_err(|_| "the built-in release key is unreadable".into()),
        _ => Err("the built-in release key isn't an Ed25519 key".into()),
    }
}

fn verify_signature(sums: &[u8], signature: &[u8], key: &[u8; 32]) -> Result<(), String> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(sums, signature)
        .map_err(|_| "the release's signature doesn't match Ferricone's release key".into())
}

/// The SHA-256 that `sha256sum` wrote for `name`.
fn expected_hash(sums: &str, name: &str) -> Option<[u8; 32]> {
    sums.lines().find_map(|line| {
        let (hex, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim_start();
        let file = file.strip_prefix('*').unwrap_or(file);
        if file != name || hex.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(out)
    })
}

// Installing ---------------------------------------------------------------

fn install_file(install: &Install, file: &Path, dir: &Path) -> Result<(), String> {
    match install {
        Install::AppImage(target) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("{}: {e}", file.display()))?;
            }
            std::fs::rename(file, target).map_err(|e| format!("replacing {}: {e}", target.display()))
        }
        Install::Tarball(_) => {
            let unpacked = dir.join("unpacked");
            let _ = std::fs::remove_dir_all(&unpacked);
            let archive = std::fs::File::open(file).map_err(|e| e.to_string())?;
            tar::Archive::new(flate2::read::GzDecoder::new(archive))
                .unpack(&unpacked)
                .map_err(|e| format!("unpacking: {e}"))?;
            let script = std::fs::read_dir(&unpacked)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path().join("install.sh"))
                .find(|p| p.is_file())
                .ok_or("the tarball has no install.sh")?;
            let result = run(std::process::Command::new("bash").arg(&script));
            let _ = std::fs::remove_dir_all(&unpacked);
            result
        }
        Install::Deb | Install::Rpm | Install::Pacman => {
            let mut command = package_command(install, file)?;
            if find_program("pkexec").is_none() {
                return Err(format!(
                    "pkexec isn't installed. Install the update with: sudo {}",
                    shown(&command)
                ));
            }
            let manual = shown(&command);
            let mut pkexec = std::process::Command::new("pkexec");
            pkexec.arg(command.get_program()).args(command.get_args());
            command = pkexec;
            run(&mut command).map_err(|e| match e.as_str() {
                "exit status: 126" => "the password prompt was cancelled".into(),
                "exit status: 127" => format!("no permission to install. Install the update with: sudo {manual}"),
                _ => format!("{e}. Install the update with: sudo {manual}"),
            })
        }
        Install::Other => Err("this copy wasn't installed from a release".into()),
    }
}

fn package_command(install: &Install, file: &Path) -> Result<std::process::Command, String> {
    let program = |name: &str| find_program(name).ok_or(format!("{name} isn't installed"));
    let mut command;
    match install {
        Install::Deb => {
            command = std::process::Command::new(program("apt-get")?);
            command.args(["install", "-y"]).arg(file);
        }
        Install::Rpm => {
            if let Some(dnf) = find_program("dnf") {
                command = std::process::Command::new(dnf);
                command.args(["install", "-y"]).arg(file);
            } else {
                command = std::process::Command::new(program("zypper")?);
                command
                    .args(["--non-interactive", "install", "--allow-unsigned-rpm"])
                    .arg(file);
            }
        }
        Install::Pacman => {
            command = std::process::Command::new(program("pacman")?);
            command.args(["-U", "--noconfirm"]).arg(file);
        }
        _ => return Err("not a package".into()),
    }
    Ok(command)
}

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .chain(["/usr/bin".into(), "/usr/sbin".into(), "/bin".into()])
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn shown(command: &std::process::Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run(command: &mut std::process::Command) -> Result<(), String> {
    let output = command
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", shown(command)))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("ferricone: {} failed: {}\n{stderr}", shown(command), output.status);
    Err(output.status.to_string())
}

/// Starts the installed version and quits this one.
fn restart(app: &AppWindow) {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    let program = match install() {
        Install::AppImage(file) => file,
        Install::Tarball(dir) => dir.join("bin/ferricone"),
        _ => PathBuf::from("/opt/ferricone/bin/ferricone"),
    };
    // A moment later, so this copy has let go of the media keys' name.
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", "sleep 1; exec \"$0\""])
        .arg(&program)
        .stdin(std::process::Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let started = command.spawn();
    match started {
        Ok(_) => app.invoke_quit(),
        Err(e) => set(app, Status::Failed(format!("couldn't start {}: {e}", program.display()), None)),
    }
}

// What Settings shows ------------------------------------------------------

/// The status line, and its button's label ("" for none).
pub fn describe(now: u64) -> (String, &'static str) {
    let current = current();
    let install_label = if can_install() { "Update" } else { "Download" };
    match status() {
        Status::Idle => (format!("Ferricone {current}"), "Check now"),
        Status::Checking => ("Checking for updates…".into(), ""),
        Status::Current(at) => (format!("Ferricone {current} is up to date. Checked {}.", ago(now, at)), "Check now"),
        Status::Available(r) => (format!("Ferricone {} is available. You have {current}.", r.version), install_label),
        Status::Downloading(r, percent) => (format!("Downloading Ferricone {}… {percent} %", r.version), ""),
        Status::Installing(r) => (
            match install() {
                Install::Deb | Install::Rpm | Install::Pacman => {
                    format!("Installing Ferricone {}: confirm with your password…", r.version)
                }
                _ => format!("Installing Ferricone {}…", r.version),
            },
            "",
        ),
        Status::Installed(r) => (format!("Ferricone {} is installed. Restart to use it.", r.version), "Restart"),
        Status::Failed(e, Some(_)) => (format!("Couldn't update: {e}."), "Try again"),
        Status::Failed(e, None) => (format!("Couldn't check for updates: {e}."), "Check now"),
    }
}

fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        s if s < 60 => "just now".into(),
        s if s < 2 * 60 * 60 => format!("{} minutes ago", (s / 60).max(1)),
        s if s < 2 * DAY => format!("{} hours ago", s / 3600),
        s => format!("{} days ago", s / DAY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(tag: &str, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            html_url: format!("{PAGE}/tag/{tag}"),
            draft: false,
            prerelease,
            assets: vec![],
        }
    }

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn stable_builds_skip_prereleases() {
        let list = || vec![gh("v1.1.0", false), gh("v1.3.0-dev.1", true), gh("v1.2.0", false)];
        assert_eq!(newest(list(), &v("1.1.0")).unwrap().version, v("1.2.0"));
        assert!(newest(list(), &v("1.2.0")).is_none());
    }

    #[test]
    fn dev_builds_see_prereleases_and_the_release_they_lead_to() {
        let list = || vec![gh("v1.2.0-dev.2", true), gh("v1.2.0-dev.1", true), gh("v1.1.0", false)];
        assert_eq!(newest(list(), &v("1.2.0-dev.1")).unwrap().version, v("1.2.0-dev.2"));
        let mut with_release = list();
        with_release.push(gh("v1.2.0", false));
        assert_eq!(newest(with_release, &v("1.2.0-dev.2")).unwrap().version, v("1.2.0"));
    }

    #[test]
    fn drafts_and_odd_tags_are_ignored() {
        let mut draft = gh("v9.0.0", false);
        draft.draft = true;
        assert!(newest(vec![draft, gh("nightly", false)], &v("1.0.0")).is_none());
    }

    #[test]
    fn places() {
        let home = Path::new("/home/a");
        assert_eq!(
            classify(Path::new("/tmp/.mount_x/usr/bin/ferricone"), Some("/home/a/F.AppImage".into()), Some(home)),
            Place::AppImage("/home/a/F.AppImage".into())
        );
        assert_eq!(classify(Path::new("/opt/ferricone/bin/ferricone"), None, Some(home)), Place::System);
        assert_eq!(
            classify(Path::new("/home/a/.local/opt/ferricone/bin/ferricone"), None, Some(home)),
            Place::Tarball("/home/a/.local/opt/ferricone".into())
        );
        assert_eq!(classify(Path::new("/home/a/src/target/release/ferricone"), None, Some(home)), Place::Other);
    }

    #[test]
    fn release_file_names() {
        // As packaging/ names them (1.1.0's release).
        let names = [
            ("ferricone_1.1.0_amd64.deb", Install::Deb, "x86_64"),
            ("ferricone_1.1.0_arm64.deb", Install::Deb, "aarch64"),
            ("ferricone-1.1.0-1.x86_64.rpm", Install::Rpm, "x86_64"),
            ("ferricone-1.1.0-1.aarch64.rpm", Install::Rpm, "aarch64"),
            ("ferricone-1.1.0-1-x86_64.pkg.tar.zst", Install::Pacman, "x86_64"),
            ("Ferricone_Player-1.1.0-aarch64.AppImage", Install::AppImage("/a".into()), "aarch64"),
            ("ferricone-1.1.0-x86_64.tar.gz", Install::Tarball("/t".into()), "x86_64"),
        ];
        for (name, install, arch) in names {
            let suffix = asset_suffix(&install, arch).unwrap();
            assert!(name.ends_with(&suffix), "{name} vs {suffix}");
            let other = if arch == "x86_64" { "aarch64" } else { "x86_64" };
            assert!(!name.ends_with(&asset_suffix(&install, other).unwrap()), "{name}");
        }
        assert!(asset_suffix(&Install::Other, "x86_64").is_none());
    }

    #[test]
    fn checksums_and_signatures() {
        use ring::signature::KeyPair;
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        let hash = "ab".repeat(32);
        let sums = format!("{hash}  ferricone_1.2.0_amd64.deb\n{}  other\n", "cd".repeat(32));
        let signature = pair.sign(sums.as_bytes());
        assert!(verify_signature(sums.as_bytes(), signature.as_ref(), &key).is_ok());
        let tampered = sums.replace("ab", "ac");
        assert!(verify_signature(tampered.as_bytes(), signature.as_ref(), &key).is_err());
        assert_eq!(expected_hash(&sums, "ferricone_1.2.0_amd64.deb"), Some([0xab; 32]));
        assert_eq!(expected_hash(&sums, "missing"), None);
    }

    #[test]
    fn the_built_in_key_reads() {
        release_key().unwrap();
    }
}
