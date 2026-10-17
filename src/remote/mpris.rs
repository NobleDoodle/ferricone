//! MPRIS (`org.mpris.MediaPlayer2`) on the session bus, on a thread of its
//! own that sleeps until the bus or the player has something: a method call,
//! a property read, or a change to announce. Position is read from the
//! player's clock when asked, so it is never stale; seeks send `Seeked`.

use super::{CONTROLS, Command, METADATA, Remote, Report, STATUS, Status, VOLUME};
use dbus::{
    Message, Path,
    arg::{PropMap, RefArg, Variant},
    blocking::{Connection, stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged},
    channel::{BusType, Channel, MatchingReceiver, Sender},
    message::{MatchRule, SignalArgs},
};
use dbus_crossroads::{Crossroads, IfaceBuilder, MethodErr};
use rustix::{
    event::{PollFd, PollFlags, poll},
    io::Errno,
    pipe::{PipeFlags, pipe_with},
};
use std::{
    os::fd::{BorrowedFd, OwnedFd},
    time::Duration,
};

const NAME: &str = "org.mpris.MediaPlayer2.ferricone";
const OBJECT: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// Wakes the server thread: a byte down a non-blocking pipe.
pub(super) struct Waker(OwnedFd);

impl Waker {
    pub(super) fn new() -> Result<(Self, OwnedFd), String> {
        let (read, write) =
            pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK).map_err(|e| e.to_string())?;
        Ok((Self(write), read))
    }

    pub(super) fn wake(&self) {
        // A full pipe already holds a wake-up.
        let _ = rustix::io::write(&self.0, &[1]);
    }
}

struct Server {
    remote: &'static Remote,
    commands: Box<dyn Fn(Command) + Send>,
}

impl Server {
    fn report(&self) -> std::sync::MutexGuard<'_, Report> {
        self.remote.report.lock().unwrap()
    }
}

pub(super) fn serve(
    remote: &'static Remote,
    woken: OwnedFd,
    commands: Box<dyn Fn(Command) + Send>,
) {
    let started = std::thread::Builder::new()
        .name("ferricone-mpris".into())
        .spawn(move || {
            if let Err(e) = run(Server { remote, commands }, woken) {
                eprintln!("ferricone: media controls: {e}");
            }
        });
    if let Err(e) = started {
        eprintln!("ferricone: media controls: {e}");
    }
}

fn run(server: Server, woken: OwnedFd) -> Result<(), String> {
    let mut channel = Channel::get_private(BusType::Session).map_err(|e| e.to_string())?;
    channel.set_watch_enabled(true);
    let conn = Connection::from(channel);
    // A second player takes an instance name, as MPRIS asks.
    let owned = |name: String| {
        use dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply::PrimaryOwner;
        conn.request_name(name, false, false, true)
            .is_ok_and(|reply| reply == PrimaryOwner)
    };
    if !owned(NAME.into()) && !owned(format!("{NAME}.instance{}", std::process::id())) {
        return Err("the session bus refused the player's name".into());
    }

    let remote = server.remote;
    let mut cr = Crossroads::new();
    let root = cr.register(ROOT, root_interface);
    let player = cr.register(PLAYER, player_interface);
    cr.insert(OBJECT, &[root, player], server);
    conn.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |msg, conn| {
            let _ = cr.handle_message(msg, conn);
            true
        }),
    );

    let socket = conn.channel().watch().fd;
    loop {
        while conn.process(Duration::ZERO).map_err(|e| e.to_string())? {}
        announce(&conn, remote);
        conn.channel().flush();

        // SAFETY: the connection owns the socket and outlives this loop.
        let socket = unsafe { BorrowedFd::borrow_raw(socket) };
        let mut fds = [
            PollFd::new(&socket, PollFlags::IN),
            PollFd::new(&woken, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) | Err(Errno::INTR) => {}
            Err(e) => return Err(e.to_string()),
        }
        if !fds[1].revents().is_empty() {
            let mut buf = [0; 64];
            while rustix::io::read(&woken, &mut buf).is_ok_and(|n| n > 0) {}
        }
    }
}

/// Sends `PropertiesChanged` for what changed, and `Seeked` after a seek.
fn announce(conn: &Connection, remote: &Remote) {
    let mut report = remote.report.lock().unwrap();
    let (changed, seeked) = (report.changed, report.seeked.take());
    report.changed = 0;
    let mut props = PropMap::new();
    let mut put = |name: &str, value: Box<dyn RefArg>| {
        props.insert(name.into(), Variant(value));
    };
    if changed & STATUS != 0 {
        put("PlaybackStatus", Box::new(status(&report).to_string()));
    }
    if changed & METADATA != 0 {
        put("Metadata", Box::new(metadata(&report)));
    }
    if changed & VOLUME != 0 {
        put("Volume", Box::new(report.volume));
    }
    if changed & CONTROLS != 0 {
        put("CanGoNext", Box::new(report.can_next));
        put("CanGoPrevious", Box::new(report.can_previous));
        put("CanSeek", Box::new(report.can_seek));
        put("CanPlay", Box::new(report.track_id > 0));
        put("CanPause", Box::new(report.track_id > 0));
    }
    drop(report);
    let path = Path::new(OBJECT).unwrap();
    if !props.is_empty() {
        let signal = PropertiesPropertiesChanged {
            interface_name: PLAYER.into(),
            changed_properties: props,
            invalidated_properties: Vec::new(),
        };
        let _ = conn.send(signal.to_emit_message(&path));
    }
    if let Some(time) = seeked
        && let Ok(signal) = Message::new_signal(OBJECT, PLAYER, "Seeked")
    {
        let _ = conn.send(signal.append1(time.as_micros() as i64));
    }
}

fn status(report: &Report) -> &'static str {
    match report.status {
        Status::Playing => "Playing",
        Status::Paused => "Paused",
        Status::Stopped => "Stopped",
    }
}

fn track_path(report: &Report) -> Path<'static> {
    if report.track_id == 0 {
        return Path::new(NO_TRACK).unwrap();
    }
    Path::new(format!(
        "/io/github/nobledoodle/ferricone/track/{}",
        report.track_id
    ))
    .unwrap()
}

fn metadata(report: &Report) -> PropMap {
    let mut map = PropMap::new();
    let mut put = |name: &str, value: Box<dyn RefArg>| {
        map.insert(name.into(), Variant(value));
    };
    put("mpris:trackid", Box::new(track_path(report)));
    if report.track_id == 0 {
        return map;
    }
    let track = &report.track;
    put("xesam:title", Box::new(track.title.clone()));
    if let Some(length) = report.length_us() {
        put("mpris:length", Box::new(length));
    }
    if let Some(artist) = &track.artist {
        put("xesam:artist", Box::new(vec![artist.clone()]));
    }
    if let Some(album) = &track.album {
        put("xesam:album", Box::new(album.clone()));
    }
    if let Some(art) = &track.art_url {
        put("mpris:artUrl", Box::new(art.clone()));
    }
    if let Some(url) = &track.url {
        put("xesam:url", Box::new(url.clone()));
    }
    map
}

fn send(server: &Server, command: Command) -> Result<(), MethodErr> {
    (server.commands)(command);
    Ok(())
}

fn root_interface(b: &mut IfaceBuilder<Server>) {
    b.property("Identity")
        .get(|_, _| Ok("Ferricone Player".to_string()));
    b.property("CanQuit").get(|_, _| Ok(true));
    b.property("CanRaise").get(|_, _| Ok(true));
    b.property("HasTracklist").get(|_, _| Ok(false));
    b.property("SupportedUriSchemes").get(|_, _| {
        Ok([
            "file", "http", "https", "rtsp", "rtp", "mms", "smb", "ftp", "sftp",
        ]
        .map(String::from)
        .to_vec())
    });
    b.property("SupportedMimeTypes")
        .get(|_, _| Ok(Vec::<String>::new()));
    b.method("Raise", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Raise)
    });
    b.method("Quit", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Quit)
    });
}

fn player_interface(b: &mut IfaceBuilder<Server>) {
    b.method("Next", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Next)
    });
    b.method("Previous", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Previous)
    });
    b.method("Pause", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Pause)
    });
    b.method("PlayPause", (), (), |_, s: &mut Server, ()| {
        send(s, Command::PlayPause)
    });
    b.method("Stop", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Stop)
    });
    b.method("Play", (), (), |_, s: &mut Server, ()| {
        send(s, Command::Play)
    });
    b.method(
        "Seek",
        ("Offset",),
        (),
        |_, s: &mut Server, (offset,): (i64,)| send(s, Command::Seek(offset as f64 / 1e6)),
    );
    b.method(
        "SetPosition",
        ("TrackId", "Position"),
        (),
        |_, s: &mut Server, (track, position): (Path<'static>, i64)| {
            // Requests for an earlier item, or past the end, are ignored, as
            // MPRIS asks.
            let (current, length) = {
                let report = s.report();
                (track_path(&report), report.length_us())
            };
            match length {
                Some(length) if track == current && (0..=length).contains(&position) => {
                    send(s, Command::SetPosition(position as f64 / length as f64))
                }
                _ => Ok(()),
            }
        },
    );
    b.method(
        "OpenUri",
        ("Uri",),
        (),
        |_, s: &mut Server, (uri,): (String,)| send(s, Command::Open(uri)),
    );
    b.signal::<(i64,), _>("Seeked", ("Position",));

    b.property("PlaybackStatus")
        .get(|_, s| Ok(status(&s.report()).to_string()));
    b.property("Rate").get(|_, _| Ok(1.0));
    b.property("MinimumRate").get(|_, _| Ok(1.0));
    b.property("MaximumRate").get(|_, _| Ok(1.0));
    b.property("Metadata").get(|_, s| Ok(metadata(&s.report())));
    b.property("Volume")
        .get(|_, s| Ok(s.report().volume))
        .set(|_, s, volume: f64| {
            send(s, Command::SetVolume(volume.max(0.0)))?;
            Ok(None) // the player reports the volume it took
        });
    b.property("Position")
        .get(|_, s| Ok(s.report().position_us()))
        .emits_changed_false();
    b.property("CanGoNext").get(|_, s| Ok(s.report().can_next));
    b.property("CanGoPrevious")
        .get(|_, s| Ok(s.report().can_previous));
    b.property("CanPlay")
        .get(|_, s| Ok(s.report().track_id > 0));
    b.property("CanPause")
        .get(|_, s| Ok(s.report().track_id > 0));
    b.property("CanSeek").get(|_, s| Ok(s.report().can_seek));
    b.property("CanControl")
        .get(|_, _| Ok(true))
        .emits_changed_const();
}
