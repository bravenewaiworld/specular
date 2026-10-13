//! Specular's UI: draws the core's screens with Slint and carries out the app's effects. All
//! slow work happens off the UI thread: files on an I/O thread (and the library scan on its
//! own), hardware on a hardware thread, box art on the art service's threads (`art`); their
//! results come back as messages to the app, or as calls on the UI thread.

mod art;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, VecModel};
use specular_core::app::{App, Effect, Msg};
use specular_core::device::{Device, DeviceProfile, Hardware, Rgb};
use specular_core::input::{Button, ButtonEvent};
use specular_core::list;
use specular_core::list::Rect;
use specular_core::model::{ImageRef, Layout, Row, RowKind, ScreenView};
use specular_core::nav::Transition;
use specular_core::settings::{Layer, builtin, store};

slint::include_modules!();

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Installs a device's Slint platform (how it draws and how its buttons arrive). It is given the
/// callback for button events, which the platform must call on the UI thread.
pub type InstallPlatform = Box<dyn FnOnce(Box<dyn FnMut(ButtonEvent)>) -> Result<(), Error>>;

/// The accent colour (also the light bar's); the UI's theme takes it from here.
const ACCENT: Rgb = Rgb(0x2e, 0xc4, 0xb6);

/// Now, on the monotonic clock button times use.
pub fn now() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Where Specular's data lives: the SD card's folders.
#[derive(Clone, Debug)]
pub struct DataRoot(pub PathBuf);

impl DataRoot {
    pub fn roms(&self) -> PathBuf { self.0.join("roms") }
    pub fn configs(&self) -> PathBuf { self.0.join("configs") }
    pub fn cache(&self) -> PathBuf { self.0.join("cache") }
}

/// Runs Specular on `device` until the event loop ends. `install` sets up the device's platform;
/// without one, Slint's default window is used and keyboard keys act as buttons (see `KEYS`).
pub fn run(device: &dyn Device, data: DataRoot, install: Option<InstallPlatform>) -> Result<(), Error> {
    let profile = device.profile();
    let keyboard = install.is_none();
    if let Some(install) = install {
        install(Box::new(|e| post(Msg::Button(e))))?;
    }
    let window = AppWindow::new()?;
    let theme = window.global::<Theme>();
    theme.set_u(profile.ppi as f32 / 400.0);
    let Rgb(r, g, b) = ACCENT;
    theme.set_accent(slint::Color::from_rgb_u8(r, g, b));
    theme.set_tint(slint::Color::from_rgb_u8(r, g, b));
    window.window().set_size(slint::PhysicalSize::new(profile.width, profile.height));

    let (io, io_thread) = io_thread(data.clone());
    let systems = specular_core::library::builtin_systems();
    let art_worker = art::worker(data.roms(), data.cache(), systems.clone(), vec![Box::new(art::LibretroThumbnails::new())],
        |rom| on_ui(move |r| r.art_stored(rom)));
    let art_loader = art::loader(data.roms(), data.cache(),
        |rom, picture| on_ui(move |r| r.art_loaded(rom, picture.map(slint::Image::from_rgba8))));
    let runner = Rc::new(Runner {
        app: RefCell::new(App::new(systems, builtin::settings())),
        window: window.as_weak(),
        rows: Rc::new(VecModel::default()),
        hints: Rc::new(VecModel::default()),
        profile,
        hardware: hardware_thread(device.hardware()),
        io,
        configs: data.configs(),
        repeat: slint::Timer::default(),
        clock: slint::Timer::default(),
        ignore_power_until: Cell::new(Duration::ZERO),
        shown_layout: Cell::new(None),
        repeating: Cell::new(false),
        art: RefCell::default(),
        picture: RefCell::default(),
        picture_timer: slint::Timer::default(),
        art_worker,
        art_loader,
        on_screen: RefCell::default(),
        requested: RefCell::default(),
    });
    window.set_rows(runner.rows.clone().into());
    window.set_hints(runner.hints.clone().into());
    RUNNER.with(|r| *r.borrow_mut() = Some(runner.clone()));
    if keyboard {
        window.on_key(|text, pressed| match key_button(&text) {
            Some(button) => { post(Msg::Button(ButtonEvent { button, pressed, time: now() })); true }
            None => false,
        });
    }
    runner.tick_clock();
    runner.refresh(None);
    let _script = std::env::var("SPECULAR_SCRIPT").ok().map(|s| script(&window, &s));
    window.run()?;
    // Dropping the runner closes the I/O thread's queue; wait for its last saves.
    RUNNER.with(|r| r.borrow_mut().take());
    drop(runner);
    let _ = io_thread.join();
    Ok(())
}

thread_local! {
    /// The running UI, reachable from messages posted by other threads.
    static RUNNER: RefCell<Option<Rc<Runner>>> = RefCell::default();
}

/// Runs `f` with the running UI on the UI thread, from any thread.
fn on_ui(f: impl FnOnce(&Rc<Runner>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(r) = RUNNER.with(|r| r.borrow().clone()) { f(&r); }
    });
}

/// Delivers a message to the app on the UI thread, from any thread.
fn post(msg: Msg) { on_ui(move |r| r.handle(msg)); }

enum HwCmd { Brightness(u8), Lights(Vec<Rgb>), Sleep }

/// How often the battery is read (on some devices each read is a slow bus transfer).
const BATTERY_INTERVAL: Duration = Duration::from_secs(10);

/// The hardware thread: carries out commands, and reads the battery every `BATTERY_INTERVAL`.
fn hardware_thread(hw: Hardware) -> mpsc::Sender<HwCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut next_battery = Instant::now();
        loop {
            if Instant::now() >= next_battery {
                if let Ok(b) = hw.power.battery() { post(Msg::Battery(b)); }
                next_battery = Instant::now() + BATTERY_INTERVAL;
            }
            let result = match rx.recv_timeout(next_battery.saturating_duration_since(Instant::now())) {
                Ok(HwCmd::Brightness(l)) => hw.display.set_brightness(l),
                Ok(HwCmd::Lights(c)) => hw.lights.show(&c),
                Ok(HwCmd::Sleep) => {
                    let r = hw.power.sleep();
                    post(Msg::Woke);
                    next_battery = Instant::now(); // it may have changed a lot
                    r
                }
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            if let Err(e) = result { eprintln!("hardware: {e}"); }
        }
    });
    tx
}

/// A settings file to write.
struct Save(PathBuf, serde_json::Map<String, serde_json::Value>);

/// The I/O thread: loads the settings at start (the library scan runs on its own thread), then
/// saves settings. Saves queued together are merged, so holding a button that changes a value
/// writes the file once rather than once per step.
fn io_thread(data: DataRoot) -> (mpsc::Sender<Save>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Save>();
    let handle = std::thread::spawn(move || {
        // A corrupt file has been moved aside; either way, start from what could be read.
        let values = store::load_layer(&Layer::Global.path(&data.configs())).unwrap_or_else(|e| { eprintln!("settings: {e:?}"); Default::default() });
        post(Msg::SettingsLoaded(Layer::Global, values));
        let roms = data.roms();
        std::thread::spawn(move || post(Msg::Library(specular_core::library::scan(&roms, &specular_core::library::builtin_systems()))));
        while let Ok(first) = rx.recv() {
            let latest: HashMap<PathBuf, _> = std::iter::once(first).chain(rx.try_iter()).map(|Save(p, v)| (p, v)).collect();
            for (path, values) in latest {
                if let Err(e) = store::store_layer(&path, &values) { eprintln!("settings: {}: {e}", path.display()); }
            }
        }
    });
    (tx, handle)
}

/// Ignore power-key presses this long after waking: the press that woke the device arrives once
/// it is running again and must not send it straight back to sleep.
const WAKE_GRACE: Duration = Duration::from_millis(1500);

struct Runner {
    app: RefCell<App>,
    window: slint::Weak<AppWindow>,
    rows: Rc<VecModel<RowData>>,
    hints: Rc<VecModel<Hint>>,
    profile: DeviceProfile,
    hardware: mpsc::Sender<HwCmd>,
    io: mpsc::Sender<Save>,
    configs: PathBuf,
    /// Fires when the next held-button repeat is due.
    repeat: slint::Timer,
    clock: slint::Timer,
    ignore_power_until: Cell<Duration>,
    /// The layout last drawn: a new one gets fresh row slots.
    shown_layout: Cell<Option<Layout>>,
    /// Pictures loaded so far, by ROM path: an image, or None if the game has none yet. A game
    /// not in it hasn't been loaded (or was dropped, see `MAX_PICTURES`).
    art: RefCell<HashMap<PathBuf, Option<slint::Image>>>,
    /// Whether the message being handled is a held button's repeat.
    repeating: Cell<bool>,
    /// The game whose picture is beside the list. It follows the focus, but while a direction is
    /// held only once the focus rests (`PICTURE_DELAY`), so held scrolling doesn't load each one.
    picture: RefCell<Option<PathBuf>>,
    picture_timer: slint::Timer,
    art_worker: mpsc::Sender<art::ArtCmd>,
    art_loader: mpsc::Sender<Vec<PathBuf>>,
    /// What the art worker and loader were last told (less the pictures loaded since), so
    /// unchanged lists aren't sent again.
    on_screen: RefCell<Vec<PathBuf>>,
    requested: RefCell<Vec<PathBuf>>,
}

/// While a direction is held, the picture beside the list changes only once the focus has
/// rested this long.
const PICTURE_DELAY: Duration = Duration::from_millis(120);
/// When more than this many pictures are loaded, all but those on screen and the one beside the
/// list are dropped.
const MAX_PICTURES: usize = 64;

impl Runner {
    fn handle(self: &Rc<Self>, msg: Msg) {
        match &msg {
            Msg::Button(e) if e.button == Button::Power && e.time < self.ignore_power_until.get() => return,
            Msg::Woke => {
                self.ignore_power_until.set(now() + WAKE_GRACE);
                self.tick_clock(); // timers stood still while asleep
            }
            Msg::Library(games) => { let _ = self.art_worker.send(art::ArtCmd::Library(games.clone())); }
            _ => {}
        }
        // The window can be resized (on a desktop), changing how many columns a grid has.
        if let Some(w) = self.window.upgrade() { self.app.borrow_mut().set_grid_columns(list::grid_columns(w.get_grid_width())); }
        // Ticks only drive held-button repeats, so a tick's presses are repeats.
        self.repeating.set(matches!(msg, Msg::Tick(_)));
        let effects = self.app.borrow_mut().handle(msg);
        for e in effects { self.apply(e); }
        self.schedule_repeat();
    }

    fn apply(self: &Rc<Self>, e: Effect) {
        match e {
            Effect::Redraw(t) => self.refresh(t),
            Effect::SaveSettings(layer) => {
                let values = self.app.borrow().ctx().settings.layer(&layer);
                let _ = self.io.send(Save(layer.path(&self.configs), values));
            }
            Effect::Apply { key, value } => self.apply_setting(key, &value),
            Effect::Launch(game) => eprintln!("launch {} (the runtime comes later)", game.path.display()),
            Effect::Sleep => { let _ = self.hardware.send(HwCmd::Sleep); }
        }
    }

    /// Carries out the settings the device acts on; ignores the rest.
    fn apply_setting(&self, key: &str, value: &serde_json::Value) {
        let level = value.as_i64().unwrap_or(0);
        let cmd = match key {
            builtin::BRIGHTNESS => HwCmd::Brightness(level as u8),
            builtin::LIGHT_BAR => {
                let scale = |c: u8| (c as i64 * level / builtin::LIGHT_BAR_MAX) as u8;
                let Rgb(r, g, b) = ACCENT;
                HwCmd::Lights(vec![Rgb(scale(r), scale(g), scale(b)); self.profile.light_bar_leds])
            }
            _ => return,
        };
        let _ = self.hardware.send(cmd);
    }

    fn schedule_repeat(self: &Rc<Self>) {
        let Some(due) = self.app.borrow().next_deadline() else { self.repeat.stop(); return };
        let me = Rc::downgrade(self);
        self.repeat.start(slint::TimerMode::SingleShot, due.saturating_sub(now()), move || {
            if let Some(r) = me.upgrade() { r.handle(Msg::Tick(now())); }
        });
    }

    /// Shows the app's current screen; `t` animates a change of screen.
    fn refresh(&self, t: Option<Transition>) {
        let Some(w) = self.window.upgrade() else { return };
        let app = self.app.borrow();
        let (view, focus) = (app.view(), app.focus());
        let grid = view.layout == Layout::Grid;
        w.set_layout(match view.layout { Layout::List => ScreenLayout::List, Layout::ListWithPicture => ScreenLayout::ListWithPicture, Layout::Grid => ScreenLayout::Grid });
        let (width, height) = (if grid { w.get_grid_width() } else { w.get_list_width() }, w.get_list_height());
        let layout = list::layout(&view.rows, focus, width, height, grid);
        // Rows are drawn from a fixed set of slots, enough for every row near the view; a row
        // always uses slot `index % slots`, so while the list scrolls a row keeps its slot and
        // only the slots of rows coming into view change. A new screen or layout gets fresh
        // slots, so the old one's rows don't animate into the new one.
        let slots = list::max_drawn(width, height, grid);
        debug_assert!(layout.drawn.len() <= slots, "two drawn rows would share a slot");
        let empty = RowData { index: -1, ..Default::default() };
        let mut wanted = vec![empty; slots];
        for i in layout.drawn.clone() {
            wanted[i % slots] = self.row_data(&view.rows[i], i, layout.rects[i]);
        }
        if t.is_none() && self.shown_layout.get() == Some(view.layout) && self.rows.row_count() == slots {
            for (slot, r) in wanted.into_iter().enumerate() { if self.rows.row_data(slot).as_ref() != Some(&r) { self.rows.set_row_data(slot, r); } }
        } else {
            self.rows.set_vec(wanted);
        }
        self.shown_layout.set(Some(view.layout));
        let hints = self.hints(&view);
        if self.hints.iter().ne(hints.iter().cloned()) { self.hints.set_vec(hints); }
        w.set_screen_title(view.title.as_str().into());
        w.set_crumb(app.parent_title().unwrap_or_default().into());
        w.set_focused_row(focus as i32);
        let plate = layout.rects.get(focus).copied().unwrap_or_default();
        w.set_plate_top(plate.y);
        w.set_plate_height(plate.height);
        // Scrolls of more than a view (paging, wrapping from the last row to the first) jump:
        // animating them would pass rows that aren't drawn.
        w.set_scroll_jumps((layout.scroll - w.get_scroll()).abs() > height);
        w.set_scroll(layout.scroll);
        if let Some(b) = app.ctx().battery {
            let pct = b.percent.map_or("–".to_string(), |p| format!("{p}%"));
            w.set_battery(if b.charging { format!("{pct} charging") } else { pct }.into());
        }
        if let Some(t) = t {
            w.set_shift(if t == Transition::Push { 64.0 } else { -64.0 });
            w.set_transitions(w.get_transitions() + 1);
        }
        let held = self.repeating.get();
        drop(app);
        self.update_art(&w, &view, focus, &layout.drawn, held);
    }

    /// Keeps the art service busy with what is on screen: the worker finds missing art for the
    /// rows drawn, and the loader loads the pictures shown (a grid's, nearest the focus first, or
    /// the one beside the list, which waits for the focus to rest while `held`; see `picture`).
    fn update_art(&self, w: &AppWindow, view: &ScreenView, focus: usize, drawn: &std::ops::Range<usize>, held: bool) {
        let rom = |i: usize| view.rows.get(i).and_then(art_of).map(|(_, rom)| rom.clone());
        let on_screen: Vec<PathBuf> = drawn.clone().filter_map(rom).collect();
        if *self.on_screen.borrow() != on_screen {
            let _ = self.art_worker.send(art::ArtCmd::OnScreen(on_screen.clone()));
            *self.on_screen.borrow_mut() = on_screen;
        }
        let wanted: Vec<PathBuf> = match view.layout {
            Layout::Grid => {
                let mut near: Vec<usize> = drawn.clone().collect();
                near.sort_by_key(|&i| i.abs_diff(focus));
                near.into_iter().filter_map(rom).collect()
            }
            Layout::ListWithPicture => {
                let focused = rom(focus);
                if focused != *self.picture.borrow() {
                    if held {
                        self.picture_timer.start(slint::TimerMode::SingleShot, PICTURE_DELAY, move || {
                            let focused = focused.clone();
                            on_ui(move |r| { *r.picture.borrow_mut() = focused; r.refresh(None); });
                        });
                    } else {
                        self.picture_timer.stop();
                        *self.picture.borrow_mut() = focused;
                    }
                }
                let picture = self.picture.borrow().clone();
                let row = picture.as_ref().and_then(|p| view.rows.iter().find(|r| art_of(r).is_some_and(|(_, rom)| rom == p)));
                // A picture still loading leaves the last one up, rather than blinking to nothing.
                if let Some(cover) = row.map(|r| self.cover(r)) && cover.known { w.set_picture(cover); }
                picture.into_iter().collect()
            }
            Layout::List => vec![],
        };
        let art = self.art.borrow();
        let wanted: Vec<PathBuf> = wanted.into_iter().filter(|p| !art.contains_key(p)).collect();
        if !wanted.is_empty() && *self.requested.borrow() != wanted {
            let _ = self.art_loader.send(wanted.clone());
            *self.requested.borrow_mut() = wanted;
        }
    }

    /// A picture arrived from the loader.
    fn art_loaded(&self, rom: PathBuf, picture: Option<slint::Image>) {
        self.requested.borrow_mut().retain(|p| p != &rom);
        let mut art = self.art.borrow_mut();
        art.insert(rom, picture);
        if art.len() > MAX_PICTURES {
            let (on_screen, picture) = (self.on_screen.borrow(), self.picture.borrow());
            art.retain(|p, _| on_screen.contains(p) || picture.as_ref() == Some(p));
        }
        drop(art);
        self.refresh(None);
    }

    /// The art worker stored a new picture: load it if it is shown.
    fn art_stored(&self, rom: PathBuf) {
        self.art.borrow_mut().remove(&rom);
        self.refresh(None);
    }

    /// The cover for a row with a picture: the picture if loaded, else a generated one.
    fn cover(&self, r: &Row) -> Cover {
        let Some((system, rom)) = art_of(r) else { return Cover::default() };
        let known = self.art.borrow().get(rom).cloned();
        let (top, bottom) = cover_colours(&r.label);
        Cover {
            known: known.is_some(), has_art: matches!(known, Some(Some(_))), art: known.flatten().unwrap_or_default(),
            top, bottom, title: r.label.as_str().into(), system: system.to_uppercase().into(),
        }
    }

    /// Row `index` of a screen, ready to draw at `rect` (u).
    fn row_data(&self, r: &Row, index: usize, rect: Rect) -> RowData {
        let (kind, value, on, fraction) = match &r.kind {
            RowKind::Header => (Kind::Header, String::new(), false, 0.0),
            RowKind::Text { detail } | RowKind::Image { detail, .. } => (Kind::Text, detail.clone().unwrap_or_default(), false, 0.0),
            RowKind::Toggle { on } => (Kind::Toggle, String::new(), *on, 0.0),
            RowKind::Choice { options, selected } => (Kind::Choice, options.get(*selected).cloned().unwrap_or_default(), false, 0.0),
            RowKind::Number { value, min, max, unit, .. } => (Kind::Number, format!("{value}{unit}"), false, (value - min) as f32 / (max - min).max(1) as f32),
            RowKind::Action { value } => (Kind::Action, value.clone().unwrap_or_default(), false, 0.0),
        };
        RowData {
            shown: true, index: index as i32, kind, label: r.label.as_str().into(), value: value.into(), on, fraction,
            enabled: r.enabled, overridden: r.overridden, x: rect.x, top: rect.y, width: rect.width, height: rect.height,
            cover: self.cover(r),
        }
    }

    /// The view's hints, for buttons the device has.
    fn hints(&self, view: &ScreenView) -> Vec<Hint> {
        view.hints.iter().filter(|(b, _)| self.profile.buttons.contains(b))
            .map(|(b, label)| Hint { button: button_label(*b).into(), label: label.as_str().into() }).collect()
    }

    /// Shows the time, and wakes again at the start of the next minute (not every second).
    fn tick_clock(self: &Rc<Self>) {
        let mut next = Duration::from_secs(60);
        if let (Some(w), Some((h, m, s))) = (self.window.upgrade(), local_time()) {
            w.set_clock(format!("{h:02}:{m:02}").into());
            next = Duration::from_secs(60 - s as u64);
        }
        let me = Rc::downgrade(self);
        self.clock.start(slint::TimerMode::SingleShot, next, move || { if let Some(r) = me.upgrade() { r.tick_clock(); } });
    }
}

/// The system and ROM of a row that shows a game's box art.
fn art_of(r: &Row) -> Option<(&'static str, &PathBuf)> {
    match &r.kind { RowKind::Image { image: ImageRef::Art { system, rom }, .. } => Some((system, rom)), _ => None }
}

/// A generated cover's gradient (top, bottom), from the title, so each game keeps its own.
fn cover_colours(title: &str) -> (slint::Color, slint::Color) {
    // FNV-1a: any stable hash will do.
    let hash = title.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
    let hue = (hash % 360) as f32;
    (slint::Color::from_hsva(hue, 0.62, 0.493, 1.0), slint::Color::from_hsva(hue + 40.0, 2.0 / 3.0, 0.21, 1.0))
}

/// The local time (hours, minutes, seconds), or None while the clock is clearly unset (before
/// 2025, as on a device that hasn't had network time yet).
fn local_time() -> Option<(u32, u32, u32)> {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    (tm.tm_year + 1900 >= 2025).then_some((tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32))
}

fn button_label(b: Button) -> &'static str {
    use Button::*;
    match b {
        A => "A", B => "B", X => "X", Y => "Y", L1 => "L1", R1 => "R1", L2 => "L2", R2 => "R2",
        Select => "SELECT", Start => "START", Menu => "MENU", F1 => "F1", F2 => "F2",
        Up => "↑", Down => "↓", Left => "←", Right => "→", VolUp => "+", VolDown => "−", Power => "⏻",
    }
}

/// Desktop keys: (the key's text as Slint reports it, the button, its name in SPECULAR_SCRIPT).
/// Arrows are the d-pad; X/Z/S/A are A/B/X/Y; Q/W are L1/R1; Enter, Tab and Esc are Start,
/// Select and Menu.
const KEYS: &[(&str, Button, &str)] = &[
    ("\u{f700}", Button::Up, "up"), ("\u{f701}", Button::Down, "down"), ("\u{f702}", Button::Left, "left"),
    ("\u{f703}", Button::Right, "right"), ("x", Button::A, "a"), ("z", Button::B, "b"), ("s", Button::X, "x"),
    ("a", Button::Y, "y"), ("q", Button::L1, "l1"), ("w", Button::R1, "r1"), ("1", Button::L2, "l2"),
    ("2", Button::R2, "r2"), ("\n", Button::Start, "start"), ("\t", Button::Select, "select"),
    ("\u{1b}", Button::Menu, "menu"), ("-", Button::VolDown, "voldown"), ("=", Button::VolUp, "volup"), ("p", Button::Power, "power"),
];

fn key_button(text: &str) -> Option<Button> {
    let t = text.to_lowercase();
    KEYS.iter().find(|(k, _, _)| *k == t).map(|&(_, b, _)| b)
}

/// For development and visual checks: `SPECULAR_SCRIPT="down,down,a"` presses those buttons (by
/// the names in `KEYS`) 400 ms apart (or `SPECULAR_SCRIPT_MS`), the first after two steps, then,
/// one step after the last, saves the window as raw RGBA to `SPECULAR_SNAPSHOT` (default
/// /tmp/specular.rgba) and quits.
fn script(window: &AppWindow, names: &str) -> slint::Timer {
    let mut steps: Vec<Button> = names.split(',')
        .filter_map(|n| KEYS.iter().find(|(_, _, name)| *name == n.trim()).map(|&(_, b, _)| b)).collect();
    steps.reverse();
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut started = false;
    let step = std::env::var("SPECULAR_SCRIPT_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(400);
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(step), move || {
        if !started { started = true; return; } // let the scan and the first frames finish
        if let Some(b) = steps.pop() {
            for pressed in [true, false] { post(Msg::Button(ButtonEvent { button: b, pressed, time: now() })); }
            return;
        }
        let Some(w) = weak.upgrade() else { return };
        if let Ok(shot) = w.window().take_snapshot() {
            let path = std::env::var("SPECULAR_SNAPSHOT").unwrap_or_else(|_| "/tmp/specular.rgba".into());
            let _ = std::fs::write(&path, shot.as_bytes());
            eprintln!("snapshot: {}×{} RGBA in {path}", shot.width(), shot.height());
        }
        let _ = slint::quit_event_loop();
    });
    timer
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_keys_match_slints_key_codes() {
        use slint::platform::Key;
        for (key, code) in [(Key::UpArrow, "\u{f700}"), (Key::DownArrow, "\u{f701}"), (Key::LeftArrow, "\u{f702}"),
                            (Key::RightArrow, "\u{f703}"), (Key::Return, "\n"), (Key::Tab, "\t"), (Key::Escape, "\u{1b}")] {
            assert_eq!(slint::SharedString::from(key).as_str(), code);
        }
    }
}
