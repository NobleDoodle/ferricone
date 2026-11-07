//! Menus that reach outside the window. Wayland only lets a popup surface do
//! that, and Slint's winit backend has none, so this makes an `xdg_popup` on
//! winit's connection, as `src/drop/wayland.rs` joins it for drops, and draws
//! `MenuWindow` into it with Slint's software renderer. Input goes back through
//! the window's `dispatch_event`. Other windows stay on the winit backend.
//!
//! The queue is drained by a timer: winit's event loop reads the socket, which
//! fills this queue too, and a pointer of its own gives the serial of the press
//! that a popup's grab must name.
//!
//! Open menus form a stack: a submenu is a popup of the menu below it, and they
//! close from the top, as the protocol requires. The compositor dismisses them
//! on a click in another app; a click on our own window is ours to notice.

use crate::{AppWindow, MenuAction, MenuEntry, MenuKind, MenuWindow};
use i_slint_backend_winit::Backend as WinitBackend;
use i_slint_core::graphics::RequestedOpenGLVersion;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::winit_030::winit::platform::wayland::WindowExtWayland;
use slint::winit_030::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, PhysicalSize, Timer, TimerMode, Weak};
use std::{
    cell::RefCell,
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
    rc::Rc,
    time::Duration,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::{Backend, ObjectId},
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_compositor::WlCompositor,
        wl_keyboard::{self, WlKeyboard},
        wl_pointer::{self, WlPointer},
        wl_registry::WlRegistry,
        wl_seat::{self, WlSeat},
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::{
    wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    xdg::shell::client::{
        xdg_popup::{self, XdgPopup},
        xdg_positioner::{Anchor, ConstraintAdjustment, Gravity, XdgPositioner},
        xdg_surface::{self, XdgSurface},
        xdg_wm_base::{self, XdgWmBase},
    },
};

/// Room for the menu's shadow around it, in logical pixels.
const MARGIN: f32 = 24.0;
/// Like the in-window menus.
const fn widths(kind: MenuKind) -> (f32, f32) {
    match kind {
        MenuKind::More => (375.0, 375.0),
        MenuKind::Library => (292.0, 292.0),
        _ => (292.0, 375.0),
    }
}
/// Up to a row's top: the menu's padding, and rows and their dividers.
const MENU_PADDING: f32 = 4.0;
const ROW_HEIGHT: f32 = 48.0;
const DIVIDER_HEIGHT: f32 = 1.0;
// wl_keyboard's key codes (evdev), the same on every layout.
const KEY_ESC: u32 = 1;
const KEY_ENTER: u32 = 28;
const KEY_UP: u32 = 103;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;
const KEY_KP_ENTER: u32 = 96;
/// Between a menu and its submenu, which also starts this far above its row.
const SUBMENU_GAP: i32 = 4;
const IDLE: Duration = Duration::from_millis(100);
const OPEN: Duration = Duration::from_millis(8);

// The platform ----------------------------------------------------------------

thread_local! {
    /// The next window created draws in software, into a popup.
    static SOFTWARE_NEXT: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
    static WAYLAND: RefCell<Option<Wayland>> = const { RefCell::new(None) };
    static TIMER: Timer = Timer::default();
    /// What the menus' rows asked for during input, done once it's delivered:
    /// a menu can't close while it handles its own event.
    static REQUESTS: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
}

enum Request {
    /// `y` is the row's top in the menu's window, margin included.
    Hovered { level: usize, action: MenuAction, y: f32 },
    Activated { level: usize, action: MenuAction, index: i32 },
}

struct Hybrid(WinitBackend);

impl Platform for Hybrid {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        match SOFTWARE_NEXT.with(|next| next.borrow_mut().take()) {
            Some(window) => Ok(window),
            None => self.0.create_window_adapter(),
        }
    }
    fn run_event_loop(&self) -> Result<(), slint::PlatformError> {
        self.0.run_event_loop()
    }
    fn process_events(
        &self,
        timeout: Option<Duration>,
        token: i_slint_core::InternalToken,
    ) -> Result<core::ops::ControlFlow<()>, slint::PlatformError> {
        self.0.process_events(timeout, token)
    }
    fn bind_context(&self, ctx: i_slint_core::SlintContextWeak, token: i_slint_core::InternalToken) {
        self.0.bind_context(ctx, token)
    }
    #[allow(deprecated)]
    fn set_event_loop_quit_on_last_window_closed(&self, quit: bool) {
        self.0.set_event_loop_quit_on_last_window_closed(quit)
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn slint::platform::EventLoopProxy>> {
        self.0.new_event_loop_proxy()
    }
    fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
        self.0.set_clipboard_text(text, clipboard)
    }
    fn clipboard_text(&self, clipboard: slint::platform::Clipboard) -> Option<String> {
        self.0.clipboard_text(clipboard)
    }
    fn click_interval(&self) -> Duration {
        self.0.click_interval()
    }
    fn cursor_flash_cycle(&self) -> Duration {
        self.0.cursor_flash_cycle()
    }
    fn open_url(&self, url: &str) -> Result<(), slint::PlatformError> {
        self.0.open_url(url)
    }
}

/// The winit backend, as `BackendSelector::require_opengl` picks it, with
/// femtovg named: the software renderer is compiled in too, for the menus.
pub fn select_backend() -> Result<(), slint::PlatformError> {
    let winit = WinitBackend::builder()
        .request_graphics_api(RequestedOpenGLVersion::OpenGL(None).into())
        .with_renderer_name("femtovg")
        .build()?;
    slint::platform::set_platform(Box::new(Hybrid(winit)))
        .map_err(|e| slint::PlatformError::Other(e.to_string()))
}

// Starting --------------------------------------------------------------------

pub fn connect(app: &AppWindow) {
    let ui = app.as_weak();
    app.on_open_native_menu(move |kind, x, y| {
        let Some(app) = ui.upgrade() else { return };
        WAYLAND.with(|wayland| {
            let mut wayland = wayland.borrow_mut();
            let Some(wayland) = wayland.as_mut() else { return };
            // The press that opened this is in the queue by now.
            let _ = wayland.queue.dispatch_pending(&mut wayland.state);
            wayland.state.pressed_outside = false;
            wayland.close_from(0);
            wayland.kind = kind;
            let entries = app.invoke_menu_entries(kind);
            wayland.open_menu(&app, MenuAction::None, Place::At { x, y }, widths(kind).0, entries);
            app.set_native_menu_open(!wayland.state.menus.is_empty());
            let _ = wayland.conn.flush();
        });
        TIMER.with(|timer| timer.set_interval(OPEN));
    });
    // winit makes its window once the event loop runs.
    let ui = app.as_weak();
    TIMER.with(|timer| timer.start(TimerMode::Repeated, IDLE, move || poll(&ui)));
}

/// Draining the queue, and starting up once the window is there.
fn poll(ui: &Weak<AppWindow>) {
    let Some(app) = ui.upgrade() else { return };
    let activated = WAYLAND.with(|wayland| {
        let mut wayland = wayland.borrow_mut();
        if let Some(wayland) = wayland.as_mut() {
            return wayland.pump(&app);
        }
        match Wayland::new(&app) {
            Some(Ok(made)) => {
                *wayland = Some(made);
                app.set_native_menus(true);
            }
            Some(Err(e)) => {
                eprintln!("ferricone: menus stay in the window: {e}");
                TIMER.with(|timer| timer.stop());
            }
            // No window yet.
            None => {}
        }
        None
    });
    if let Some((kind, action, index)) = activated {
        app.invoke_native_activate(kind, action, index);
    }
}

// Wayland ---------------------------------------------------------------------

struct Wayland {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    /// The player window's.
    parent: XdgSurface,
    /// Of the menus open.
    kind: MenuKind,
}

struct State {
    compositor: WlCompositor,
    shm: WlShm,
    wm_base: XdgWmBase,
    viewporter: WpViewporter,
    seat: WlSeat,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    keys: Vec<u32>,
    /// The last press: the serial a grab must name.
    button_serial: u32,
    /// Bottom first.
    menus: Vec<OpenMenu>,
    /// The menu under the pointer, and where on it.
    under_pointer: Option<(usize, f32, f32)>,
    /// A press landed off the menus.
    pressed_outside: bool,
    input: Vec<(usize, WindowEvent)>,
}

struct OpenMenu {
    /// The row that opened it, `None` for the first.
    action: MenuAction,
    component: MenuWindow,
    entries: ModelRc<MenuEntry>,
    /// The row chosen with the keyboard, -1 for none.
    selected: i32,
    window: Rc<MinimalSoftwareWindow>,
    surface: WlSurface,
    xdg_surface: XdgSurface,
    popup: XdgPopup,
    viewport: WpViewport,
    pool: WlShmPool,
    buffers: [WlBuffer; 2],
    free: [bool; 2],
    file: File,
    size: (u32, u32),
    width: i32,
    pixels: Vec<PremultipliedRgbaColor>,
    bytes: Vec<u8>,
    configure: Option<u32>,
    configured: bool,
    dismissed: bool,
}

enum Place {
    /// At a point on the player's window.
    At { x: f32, y: f32 },
    /// Beside a row of the menu on top; `y` as in `Request::Hovered`.
    Beside { y: f32 },
}

impl Wayland {
    /// `None` until the window exists; an error where popups can't be made.
    fn new(app: &AppWindow) -> Option<Result<Self, String>> {
        app.window().with_winit_window(|window| {
            let handle = window.display_handle().map_err(|e| e.to_string())?.as_raw();
            let RawDisplayHandle::Wayland(handle) = handle else {
                return Err("not on Wayland".to_string());
            };
            let parent = window.xdg_surface().ok_or("no xdg_surface")?;
            // SAFETY: the display and winit's xdg_surface live as long as the window.
            let backend = unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) };
            let conn = Connection::from_backend(backend);
            let id = unsafe { ObjectId::from_ptr(XdgSurface::interface(), parent.as_ptr().cast()) }
                .map_err(|e| e.to_string())?;
            let parent = XdgSurface::from_id(&conn, id).map_err(|e| e.to_string())?;
            let (globals, mut queue) =
                registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
            let qh = queue.handle();
            let bind_error = |e: wayland_client::globals::BindError| e.to_string();
            let mut state = State {
                compositor: globals.bind(&qh, 4..=6, ()).map_err(bind_error)?,
                shm: globals.bind(&qh, 1..=1, ()).map_err(bind_error)?,
                wm_base: globals.bind(&qh, 1..=3, ()).map_err(bind_error)?,
                viewporter: globals.bind(&qh, 1..=1, ()).map_err(bind_error)?,
                seat: globals.bind(&qh, 1..=7, ()).map_err(bind_error)?,
                pointer: None,
                keyboard: None,
                keys: Vec::new(),
                button_serial: 0,
                menus: Vec::new(),
                under_pointer: None,
                pressed_outside: false,
                input: Vec::new(),
            };
            queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
            Ok(Self { conn, queue, state, parent, kind: MenuKind::None })
        })
    }

    /// What arrived, to the menus and back; returns a row to run.
    fn pump(&mut self, app: &AppWindow) -> Option<(MenuKind, MenuAction, i32)> {
        if let Err(e) = self.queue.dispatch_pending(&mut self.state) {
            eprintln!("ferricone: menu queue: {e}");
        }
        if let Some(level) = self.state.menus.iter().position(|menu| menu.dismissed) {
            self.close_from(level);
        }
        let pressed_outside = std::mem::take(&mut self.state.pressed_outside);
        if pressed_outside {
            self.close_from(0);
        }
        self.state.deliver();
        let mut activated = None;
        for request in REQUESTS.with(|requests| std::mem::take(&mut *requests.borrow_mut())) {
            match request {
                // Only the first menu's rows open anything.
                Request::Hovered { level: 0, action, y } => self.show_submenu(app, action, y),
                Request::Hovered { .. } => {}
                Request::Activated { level, action, index } => {
                    activated = activated.or(self.run_row(app, level, action, index));
                }
            }
        }
        for key in std::mem::take(&mut self.state.keys) {
            activated = activated.or(self.key(app, key));
        }
        self.state.deliver();
        // Not on the tick that closed them for a press: the window may still
        // have that press to handle, and must take it as closing the menu.
        if !pressed_outside {
            app.set_native_menu_open(!self.state.menus.is_empty());
        }
        let _ = self.conn.flush();
        activated
    }

    /// A row opens its submenu, which is already open if it was clicked; or
    /// closes the menus to run.
    fn run_row(&mut self, app: &AppWindow, level: usize, action: MenuAction, index: i32) -> Option<(MenuKind, MenuAction, i32)> {
        if level == 0 && app.invoke_submenu_entries(self.kind, action).row_count() > 0 {
            return None;
        }
        let kind = self.kind;
        self.close_from(0);
        Some((kind, action, index))
    }

    fn key(&mut self, app: &AppWindow, key: u32) -> Option<(MenuKind, MenuAction, i32)> {
        let top = self.state.menus.len().checked_sub(1)?;
        match key {
            KEY_ESC => self.close_from(top),
            KEY_LEFT if top > 0 => self.close_from(top),
            KEY_UP => self.state.menus[top].select_next(-1),
            KEY_DOWN => self.state.menus[top].select_next(1),
            KEY_RIGHT | KEY_ENTER | KEY_KP_ENTER => {
                let menu = &self.state.menus[top];
                let row = usize::try_from(menu.selected).ok()?;
                let entry = menu.entries.row_data(row)?;
                let opens = top == 0 && app.invoke_submenu_entries(self.kind, entry.action).row_count() > 0;
                if opens {
                    let y = menu.row_top(row);
                    self.show_submenu(app, entry.action, y);
                    if let Some(submenu) = self.state.menus.get_mut(1) {
                        submenu.select_next(1);
                    }
                } else if key != KEY_RIGHT {
                    return self.run_row(app, top, entry.action, entry.index);
                }
            }
            _ => {}
        }
        None
    }

    /// Opens what the row under the pointer opens, closing what it doesn't.
    fn show_submenu(&mut self, app: &AppWindow, action: MenuAction, y: f32) {
        if self.state.menus.get(1).is_some_and(|menu| menu.action == action) {
            return;
        }
        self.close_from(1);
        let entries = app.invoke_submenu_entries(self.kind, action);
        if entries.row_count() > 0 && !self.state.menus.is_empty() {
            let width = widths(self.kind).1;
            self.open_menu(app, action, Place::Beside { y }, width, entries);
        }
    }

    /// Closes this menu and those on top of it, from the top.
    fn close_from(&mut self, level: usize) {
        while self.state.menus.len() > level {
            let Some(menu) = self.state.menus.pop() else { break };
            menu.popup.destroy();
            menu.viewport.destroy();
            menu.xdg_surface.destroy();
            menu.surface.destroy();
            for buffer in &menu.buffers {
                buffer.destroy();
            }
            menu.pool.destroy();
            let _ = menu.component.hide();
        }
        self.state.input.retain(|(on, _)| *on < level);
        if self.state.under_pointer.is_some_and(|(on, ..)| on >= level) {
            self.state.under_pointer = None;
        }
        let _ = self.conn.flush();
        if self.state.menus.is_empty() {
            TIMER.with(|timer| timer.set_interval(IDLE));
        }
    }

    fn open_menu(&mut self, app: &AppWindow, action: MenuAction, place: Place, width: f32, entries: ModelRc<MenuEntry>) {
        if let Err(e) = self.try_open_menu(app, action, place, width, entries) {
            eprintln!("ferricone: menu: {e}");
        }
    }

    fn try_open_menu(
        &mut self,
        app: &AppWindow,
        action: MenuAction,
        place: Place,
        width: f32,
        entries: ModelRc<MenuEntry>,
    ) -> Result<(), String> {
        let qh = self.queue.handle();
        let state = &mut self.state;
        let level = state.menus.len();
        let scale = app.window().scale_factor();

        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        SOFTWARE_NEXT.with(|next| *next.borrow_mut() = Some(window.clone()));
        let component = MenuWindow::new().map_err(|e| e.to_string())?;
        crate::theme::copy_to_menu(app, &component);
        component.set_entries(entries.clone());
        component.set_margin(MARGIN);
        component.set_menu_width(width);
        component.on_activated(move |action, index| {
            REQUESTS.with(|requests| requests.borrow_mut().push(Request::Activated { level, action, index }));
        });
        component.on_hovered(move |action, y| {
            REQUESTS.with(|requests| requests.borrow_mut().push(Request::Hovered { level, action, y }));
        });
        component.window().dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        component.show().map_err(|e| e.to_string())?;
        let menu_height = component.get_menu_height().ceil() as i32;
        let logical = (width + 2.0 * MARGIN, menu_height as f32 + 2.0 * MARGIN);
        let size = (
            (logical.0 * scale).ceil() as u32,
            (logical.1 * scale).ceil() as u32,
        );
        window.set_size(PhysicalSize::new(size.0, size.1));

        let stride = size.0 * 4;
        let frame = stride as usize * size.1 as usize;
        let fd = rustix::fs::memfd_create("ferricone-menu", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|e| e.to_string())?;
        let file = File::from(fd);
        file.set_len(frame as u64 * 2).map_err(|e| e.to_string())?;
        let pool = state.shm.create_pool(file.as_fd(), (frame * 2) as i32, &qh, ());
        let buffers = [0, 1].map(|index| {
            pool.create_buffer(
                index * frame as i32,
                size.0 as i32,
                size.1 as i32,
                stride as i32,
                wl_shm::Format::Argb8888,
                &qh,
                (),
            )
        });

        let surface = state.compositor.create_surface(&qh, ());
        let viewport = state.viewporter.get_viewport(&surface, &qh, ());
        viewport.set_destination(logical.0.ceil() as i32, logical.1.ceil() as i32);
        let xdg_surface = state.wm_base.get_xdg_surface(&surface, &qh, ());

        let positioner = state.wm_base.create_positioner(&qh, ());
        positioner.set_size(width as i32, menu_height);
        let parent = match place {
            Place::At { x, y } => {
                positioner.set_anchor_rect(x as i32, y as i32, 1, 1);
                positioner.set_anchor(Anchor::BottomRight);
                positioner.set_gravity(Gravity::BottomRight);
                positioner.set_constraint_adjustment(
                    ConstraintAdjustment::FlipX
                        | ConstraintAdjustment::FlipY
                        | ConstraintAdjustment::SlideX
                        | ConstraintAdjustment::SlideY,
                );
                &self.parent
            }
            Place::Beside { y } => {
                let below = state.menus.last().ok_or("no menu to open beside")?;
                // In the popup's geometry, which leaves out the margin.
                let row = (y - MARGIN) as i32;
                positioner.set_anchor_rect(0, row, below.width, 1);
                positioner.set_anchor(Anchor::TopRight);
                positioner.set_gravity(Gravity::BottomRight);
                positioner.set_offset(SUBMENU_GAP, -SUBMENU_GAP);
                // To the other side where it doesn't fit, like the in-window menu.
                positioner.set_constraint_adjustment(ConstraintAdjustment::FlipX | ConstraintAdjustment::SlideY);
                &below.xdg_surface
            }
        };
        let popup = xdg_surface.get_popup(Some(parent), &positioner, &qh, ());
        positioner.destroy();
        // Only once the surface has its role.
        xdg_surface.set_window_geometry(MARGIN as i32, MARGIN as i32, width as i32, menu_height);
        // A submenu grabs too: the pointer on a popup outside the grab counts
        // as outside. It names the last press, as hovering has no serial.
        popup.grab(&state.seat, state.button_serial);
        surface.commit();

        state.menus.push(OpenMenu {
            action,
            component,
            entries,
            selected: -1,
            window,
            surface,
            xdg_surface,
            popup,
            viewport,
            pool,
            buffers,
            free: [true; 2],
            file,
            size,
            width: width as i32,
            pixels: vec![PremultipliedRgbaColor::default(); (size.0 * size.1) as usize],
            bytes: vec![0; frame],
            configure: None,
            configured: false,
            dismissed: false,
        });
        Ok(())
    }
}

impl State {
    /// Acknowledges configures, hands on the pointer, and draws what changed.
    fn deliver(&mut self) {
        for (level, event) in std::mem::take(&mut self.input) {
            if let Some(menu) = self.menus.get_mut(level) {
                // The pointer takes over from the keyboard.
                if matches!(event, WindowEvent::PointerMoved { .. }) && menu.selected >= 0 {
                    menu.selected = -1;
                    menu.component.set_selected(-1);
                }
                menu.component.window().dispatch_event(event);
            }
        }
        for menu in &mut self.menus {
            if let Some(serial) = menu.configure.take() {
                menu.xdg_surface.ack_configure(serial);
                menu.configured = true;
            }
            if menu.configured {
                menu.draw();
            }
        }
    }

    fn level_of(&self, surface: &WlSurface) -> Option<usize> {
        self.menus.iter().position(|menu| &menu.surface == surface)
    }
}

impl OpenMenu {
    /// The next row that isn't disabled, around the ends.
    fn select_next(&mut self, step: i32) {
        let rows = self.entries.row_count() as i32;
        let mut row = if self.selected < 0 && step < 0 { rows } else { self.selected };
        for _ in 0..rows {
            row = (row + step).rem_euclid(rows);
            if self.entries.row_data(row as usize).is_some_and(|entry| !entry.disabled) {
                self.selected = row;
                self.component.set_selected(row);
                return;
            }
        }
    }

    /// As `Request::Hovered`'s `y`.
    fn row_top(&self, row: usize) -> f32 {
        let dividers = self.entries.iter().take(row + 1).filter(|entry| entry.divider_before).count();
        MARGIN + MENU_PADDING + row as f32 * ROW_HEIGHT + dividers as f32 * DIVIDER_HEIGHT
    }

    fn draw(&mut self) {
        let Some(index) = self.free.iter().position(|free| *free) else { return };
        let (width, height) = self.size;
        let pixels = &mut self.pixels;
        let drawn = self.window.draw_if_needed(|renderer| {
            renderer.render(pixels, width as usize);
        });
        if !drawn {
            return;
        }
        // wl_shm's ARGB8888 is B, G, R, A in memory.
        let (chunks, _) = self.bytes.as_chunks_mut::<4>();
        for (bytes, pixel) in chunks.iter_mut().zip(self.pixels.iter()) {
            *bytes = [pixel.blue, pixel.green, pixel.red, pixel.alpha];
        }
        let offset = index as u64 * self.bytes.len() as u64;
        if let Err(e) = self.file.write_all_at(&self.bytes, offset) {
            eprintln!("ferricone: menu buffer: {e}");
            return;
        }
        self.free[index] = false;
        self.surface.attach(Some(&self.buffers[index]), 0, 0);
        self.surface.damage_buffer(0, 0, width as i32, height as i32);
        self.surface.commit();
    }
}

// Events ----------------------------------------------------------------------

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(qh, ()));
        }
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && state.keyboard.is_none()
        {
            state.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(state: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_pointer::Event::Enter { surface, surface_x, surface_y, .. } => {
                if let Some(level) = state.level_of(&surface) {
                    state.input.push((level, moved(surface_x, surface_y)));
                    state.under_pointer = Some((level, surface_x as f32, surface_y as f32));
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if let Some(level) = state.level_of(&surface) {
                    state.input.push((level, WindowEvent::PointerExited));
                    state.under_pointer = None;
                }
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                if let Some((level, ..)) = state.under_pointer {
                    state.input.push((level, moved(surface_x, surface_y)));
                    state.under_pointer = Some((level, surface_x as f32, surface_y as f32));
                }
            }
            // Every press names the serial; only a menu gets the input.
            wl_pointer::Event::Button { serial, button, state: pressed, .. } => {
                state.button_serial = serial;
                if state.under_pointer.is_none()
                    && !state.menus.is_empty()
                    && matches!(pressed, WEnum::Value(wl_pointer::ButtonState::Pressed))
                {
                    state.pressed_outside = true;
                }
                if let Some((level, x, y)) = state.under_pointer {
                    let position = LogicalPosition::new(x, y);
                    let button = if button == 0x110 { PointerEventButton::Left } else { PointerEventButton::Right };
                    state.input.push((level, match pressed {
                        WEnum::Value(wl_pointer::ButtonState::Pressed) => WindowEvent::PointerPressed { position, button },
                        _ => WindowEvent::PointerReleased { position, button },
                    }));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(state: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_keyboard::Event::Key { key, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. } = event
            && !state.menus.is_empty()
        {
            state.keys.push(key);
        }
    }
}

fn moved(x: f64, y: f64) -> WindowEvent {
    WindowEvent::PointerMoved { position: LogicalPosition::new(x as f32, y as f32) }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(_: &mut Self, base: &XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(state: &mut Self, surface: &XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(menu) = state.menus.iter_mut().find(|menu| &menu.xdg_surface == surface)
        {
            menu.configure = Some(serial);
        }
    }
}

impl Dispatch<XdgPopup, ()> for State {
    fn event(state: &mut Self, popup: &XdgPopup, event: xdg_popup::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_popup::Event::PopupDone = event
            && let Some(menu) = state.menus.iter_mut().find(|menu| &menu.popup == popup)
        {
            menu.dismissed = true;
        }
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(state: &mut Self, buffer: &WlBuffer, event: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event {
            for menu in &mut state.menus {
                if let Some(index) = menu.buffers.iter().position(|b| b == buffer) {
                    menu.free[index] = true;
                }
            }
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WpViewporter);
delegate_noop!(State: ignore WpViewport);
delegate_noop!(State: ignore XdgPositioner);
