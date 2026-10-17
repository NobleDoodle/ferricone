//! File drops on Wayland. winit 0.30 has no drag and drop there, so this joins
//! winit's connection to the compositor with an event queue of its own, binds
//! a `wl_data_device` and takes `text/uri-list` drops on a thread of its own.

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    pipe::{PipeFlags, pipe_with},
};
use slint::winit_030::winit::{
    raw_window_handle::{HasDisplayHandle, RawDisplayHandle},
    window::Window,
};
use std::{
    io::Read,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    sync::Mutex,
    thread::JoinHandle,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    backend::{Backend, WaylandError},
    delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_offer::{self, WlDataOffer},
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
    },
};

use crate::diag::trace;

const URI_LIST: &str = "text/uri-list";
/// How long a drop source may take to send its list.
const READ_TIMEOUT: Timespec = Timespec {
    tv_sec: 3,
    tv_nsec: 0,
};

/// Stops and joins the listener thread when dropped.
pub struct Listener {
    stop: Option<OwnedFd>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        drop(self.stop.take()); // closing the pipe wakes the thread
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Listens for drops on `window`'s Wayland connection; `None` elsewhere.
/// `dropped` gets each drop's URI list, on the listener's thread.
pub fn listen(
    window: &Window,
    dropped: impl Fn(String) + Send + 'static,
) -> Result<Option<Listener>, String> {
    let handle = window.display_handle().map_err(|e| e.to_string())?;
    let RawDisplayHandle::Wayland(handle) = handle.as_raw() else {
        return Ok(None);
    };
    // SAFETY: winit keeps the display connected until the process exits, and
    // `Listener` stops this thread before the event loop ends.
    let backend = unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let (globals, queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let manager: WlDataDeviceManager = globals.bind(&qh, 3..=3, ()).map_err(|e| e.to_string())?;
    let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
    let device = manager.get_data_device(&seat, &qh, ());
    conn.flush().map_err(|e| e.to_string())?;

    let (stop_read, stop_write) = pipe_with(PipeFlags::CLOEXEC).map_err(|e| e.to_string())?;
    let state = State {
        drag: None,
        selection: None,
        dropped: Box::new(dropped),
        stop: stop_read,
    };
    let thread = std::thread::Builder::new()
        .name("ferricone-drops".into())
        .spawn(move || {
            let mut state = state;
            let mut queue = queue;
            if let Err(e) = run(&mut queue, &mut state) {
                eprintln!("ferricone: file drops: {e}");
            }
            // The objects have no destructor requests to send; leaving the
            // queue alive keeps libwayland from warning about them at exit.
            std::mem::forget((queue, manager, seat, device));
        })
        .map_err(|e| e.to_string())?;
    Ok(Some(Listener {
        stop: Some(stop_write),
        thread: Some(thread),
    }))
}

/// Dispatches our queue until the stop pipe closes, reading the socket
/// alongside winit through libwayland's prepare/read protocol.
fn run(queue: &mut EventQueue<State>, state: &mut State) -> Result<(), String> {
    loop {
        queue.dispatch_pending(state).map_err(|e| e.to_string())?;
        match queue.flush() {
            Ok(()) | Err(WaylandError::Io(_)) => {}
            Err(e) => return Err(e.to_string()),
        }
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let socket = guard.connection_fd();
        let mut fds = [
            PollFd::new(&socket, PollFlags::IN),
            PollFd::new(&state.stop, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.to_string()),
        }
        if !fds[1].revents().is_empty() {
            return Ok(());
        }
        if !fds[0].revents().is_empty() {
            match guard.read() {
                Ok(_) | Err(WaylandError::Io(_)) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

struct State {
    /// The offer being dragged over the window, kept after `leave` until the
    /// next drag: some compositors send `leave` just before `drop`.
    drag: Option<WlDataOffer>,
    /// The clipboard's offer, which arrives on the same device; only freed.
    selection: Option<WlDataOffer>,
    dropped: Box<dyn Fn(String) + Send>,
    stop: OwnedFd,
}

/// What an offer said about itself.
#[derive(Default)]
struct Offer {
    mime_types: Vec<String>,
    action: Option<DndAction>,
}

fn offer(offer: &WlDataOffer) -> &Mutex<Offer> {
    offer
        .data::<Mutex<Offer>>()
        .expect("offers carry Offer data")
}

impl State {
    /// The dropped URI list, read from the drag source through a pipe.
    fn receive(&self, offer: &WlDataOffer, conn: &Connection) -> Option<String> {
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).ok()?;
        offer.receive(URI_LIST.into(), write.as_fd());
        conn.flush().ok()?;
        drop(write); // the source holds the other copy; EOF once it closes it
        read_all(read, self.stop.as_fd())
    }
}

/// Reads until EOF, giving up when the stop pipe closes or the source stalls.
fn read_all(fd: OwnedFd, stop: BorrowedFd) -> Option<String> {
    let mut file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let mut fds = [
            PollFd::new(&file, PollFlags::IN),
            PollFd::new(&stop, PollFlags::IN),
        ];
        if poll(&mut fds, Some(&READ_TIMEOUT)).ok()? == 0 || !fds[1].revents().is_empty() {
            return None;
        }
        match file.read(&mut chunk).ok()? {
            0 => return String::from_utf8(bytes).ok(),
            n => bytes.extend_from_slice(&chunk[..n]),
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                id: Some(o),
                ..
            } => {
                let takes = {
                    let data = offer(&o).lock().unwrap();
                    trace!("drag enters, offering {:?}", data.mime_types);
                    data.mime_types.iter().any(|m| m == URI_LIST)
                };
                o.accept(serial, takes.then(|| URI_LIST.into()));
                if takes {
                    o.set_actions(DndAction::Copy, DndAction::Copy);
                }
                if let Some(old) = state.drag.replace(o) {
                    old.destroy();
                }
            }
            wl_data_device::Event::Leave => trace!("drag leaves"),
            wl_data_device::Event::Drop => {
                let Some(o) = state.drag.take() else { return };
                let (takes, action) = {
                    let data = offer(&o).lock().unwrap();
                    let takes = data.mime_types.iter().any(|m| m == URI_LIST);
                    (takes, data.action)
                };
                trace!("drop, action {action:?}");
                if takes {
                    let uris = state.receive(&o, conn);
                    trace!("dropped {uris:?}");
                    // Allowed only once an action was agreed on, which not
                    // every compositor and drag source does.
                    if action == Some(DndAction::Copy) {
                        o.finish();
                    }
                    if let Some(uris) = uris {
                        (state.dropped)(uris);
                    }
                }
                o.destroy();
            }
            wl_data_device::Event::Selection { id } => {
                if let Some(old) = std::mem::replace(&mut state.selection, id) {
                    old.destroy();
                }
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Mutex::new(Offer::default())),
    ]);
}

impl Dispatch<WlDataOffer, Mutex<Offer>> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        data: &Mutex<Offer>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut data = data.lock().unwrap();
        match event {
            wl_data_offer::Event::Offer { mime_type } => data.mime_types.push(mime_type),
            wl_data_offer::Event::Action { dnd_action } => {
                data.action = dnd_action.into_result().ok()
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlDataDeviceManager);
delegate_noop!(State: ignore WlSeat);
