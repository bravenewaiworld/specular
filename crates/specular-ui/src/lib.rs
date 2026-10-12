//! Specular's UI: draws the core's screens with Slint and carries out the app's effects. All
//! slow work happens off the UI thread: file access on an I/O thread, hardware on a hardware
//! thread; their results come back as messages to the app.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ComponentHandle, Model, SharedString, VecModel};
use specular_core::app::{App, Effect, Msg};
use specular_core::device::{Device, DeviceProfile, Hardware, Rgb};
use specular_core::input::{Button, ButtonEvent};
use specular_core::list;
use specular_core::model::{RowKind, ScreenView};
use specular_core::nav::Transition;
use specular_core::settings::{Layer, store};

slint::include_modules!();

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Installs a device's Slint platform (how it draws and how its buttons arrive), given the
/// function that receives the buttons, called on the UI thread.
pub type InstallPlatform = Box<dyn FnOnce(Box<dyn FnMut(ButtonEvent)>) -> Result<(), Error>>;

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
}

/// Runs Specular on `device` until the event loop ends. `install` sets up the device's platform;
/// without one, Slint's default window is used and keyboard keys act as buttons (the desktop).
pub fn run(device: &dyn Device, data: DataRoot, install: Option<InstallPlatform>) -> Result<(), Error> {
    let profile = device.profile();
    let keyboard = install.is_none();
    if let Some(install) = install {
        install(Box::new(|e| post(Msg::Button(e))))?;
    }
    let window = AppWindow::new()?;
    window.global::<Theme>().set_u(profile.ppi as f32 / 400.0);
    window.window().set_size(slint::PhysicalSize::new(profile.width, profile.height));

    let runner = Rc::new(Runner {
        app: RefCell::new(App::new(specular_core::library::builtin_systems(), specular_core::settings::builtin::settings())),
        window: window.as_weak(),
        rows: Rc::new(VecModel::default()),
        profile: profile.clone(),
        hardware: hardware_thread(device.hardware()),
        io: io_thread(data.clone()),
        configs: data.configs(),
        repeat: slint::Timer::default(),
        clock: slint::Timer::default(),
        ignore_power_until: Cell::new(Duration::ZERO),
    });
    window.set_rows(runner.rows.clone().into());
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
    RUNNER.with(|r| r.borrow_mut().take());
    Ok(())
}

thread_local! {
    /// The running UI, reachable from messages posted by other threads.
    static RUNNER: RefCell<Option<Rc<Runner>>> = RefCell::default();
}

/// Delivers a message to the app on the UI thread, from any thread.
fn post(msg: Msg) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(r) = RUNNER.with(|r| r.borrow().clone()) { r.handle(msg); }
    });
}

enum HwCmd { Brightness(u8), Lights(Vec<Rgb>), Sleep }

/// The hardware thread: carries out commands and reads the battery every 10 s (on some devices
/// each read is a slow bus transfer).
fn hardware_thread(hw: Hardware) -> mpsc::Sender<HwCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        if let Ok(b) = hw.power.battery() { post(Msg::Battery(b)); }
        let result = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(HwCmd::Brightness(l)) => hw.display.set_brightness(l),
            Ok(HwCmd::Lights(c)) => hw.lights.show(&c),
            Ok(HwCmd::Sleep) => { let r = hw.power.sleep(); post(Msg::Woke); r }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        if let Err(e) = result { eprintln!("hardware: {e}"); }
    });
    tx
}

enum IoCmd { Save(PathBuf, serde_json::Map<String, serde_json::Value>) }

/// The I/O thread: loads the settings and scans the library at start, then saves settings.
fn io_thread(data: DataRoot) -> mpsc::Sender<IoCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        match store::load_layer(&Layer::Global.path(&data.configs())) {
            Ok(values) => post(Msg::SettingsLoaded(Layer::Global, values)),
            Err(e) => eprintln!("settings: {e:?}"),
        }
        post(Msg::Library(specular_core::library::scan(&data.roms(), &specular_core::library::builtin_systems())));
        for cmd in rx {
            match cmd {
                IoCmd::Save(path, values) => if let Err(e) = store::store_layer(&path, &values) { eprintln!("settings: {}: {e}", path.display()); },
            }
        }
    });
    tx
}

/// Ignore power-key presses this long after waking: the press that woke the device arrives once
/// it is running again and must not send it straight back to sleep.
const WAKE_GRACE: Duration = Duration::from_millis(1500);

struct Runner {
    app: RefCell<App>,
    window: slint::Weak<AppWindow>,
    rows: Rc<VecModel<RowData>>,
    profile: DeviceProfile,
    hardware: mpsc::Sender<HwCmd>,
    io: mpsc::Sender<IoCmd>,
    configs: PathBuf,
    /// Fires when the next held-button repeat is due.
    repeat: slint::Timer,
    clock: slint::Timer,
    ignore_power_until: Cell<Duration>,
}

impl Runner {
    fn handle(self: &Rc<Self>, msg: Msg) {
        if let Msg::Button(e) = &msg {
            if e.button == Button::Power && e.time < self.ignore_power_until.get() { return; }
        }
        if let Msg::Woke = msg { self.ignore_power_until.set(now() + WAKE_GRACE); }
        let effects = self.app.borrow_mut().handle(msg);
        for e in effects { self.apply(e); }
        self.schedule_repeat();
    }

    fn apply(self: &Rc<Self>, e: Effect) {
        match e {
            Effect::Redraw(t) => self.refresh(t),
            Effect::SaveSettings(layer) => {
                let values = self.app.borrow().ctx.settings.layer(&layer);
                let _ = self.io.send(IoCmd::Save(layer.path(&self.configs), values));
            }
            Effect::Apply { key, value } => self.apply_setting(key, &value),
            Effect::Launch(game) => eprintln!("launch {} (the runtime comes later)", game.path.display()),
            Effect::Sleep => { let _ = self.hardware.send(HwCmd::Sleep); }
        }
    }

    /// Settings the device acts on.
    fn apply_setting(&self, key: &str, value: &serde_json::Value) {
        let level = value.as_u64().unwrap_or(0) as u8;
        let cmd = match key {
            "display.brightness" => HwCmd::Brightness(level),
            "lights.brightness" => {
                let accent = |c: u8| (c as u16 * level as u16 / 5) as u8;
                HwCmd::Lights(vec![Rgb(accent(0x2e), accent(0xc4), accent(0xb6)); self.profile.light_bar_leds])
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
        let view = app.view();
        let list_height = self.profile.height as f32 / (self.profile.ppi as f32 / 400.0) - 72.0 - 76.0 - 108.0;
        let layout = list::layout(&view.rows, view.focus, list_height);
        let rows: Vec<RowData> = view.rows.iter().zip(layout.tops.iter().zip(&layout.heights)).map(|(r, (&top, &height))| {
            let (kind, value, on, fraction) = match &r.kind {
                RowKind::Header => (0, String::new(), false, 0.0),
                RowKind::Text { detail } => (1, detail.clone().unwrap_or_default(), false, 0.0),
                RowKind::Image { detail, .. } => (2, detail.clone().unwrap_or_default(), false, 0.0),
                RowKind::Toggle { on } => (3, String::new(), *on, 0.0),
                RowKind::Choice { options, selected } => (4, options.get(*selected).cloned().unwrap_or_default(), false, 0.0),
                RowKind::Number { value, min, max, unit, .. } => (5, format!("{value}{unit}"), false, (value - min) as f32 / (max - min).max(1) as f32),
                RowKind::Action { value } => (6, value.clone().unwrap_or_default(), false, 0.0),
            };
            RowData { kind, label: r.label.as_str().into(), value: value.into(), on, fraction, enabled: r.enabled, overridden: r.overridden, top, height }
        }).collect();
        // Replace only rows that changed, so Slint redraws (and animates) only those.
        if self.rows.row_count() == rows.len() {
            for (i, r) in rows.into_iter().enumerate() { if self.rows.row_data(i).as_ref() != Some(&r) { self.rows.set_row_data(i, r); } }
        } else {
            self.rows.set_vec(rows);
        }
        w.set_screen_title(view.title.as_str().into());
        w.set_crumb(if app.depth() > 1 { "Library".into() } else { SharedString::new() });
        w.set_focused_row(view.focus as i32);
        w.set_plate_top(layout.tops.get(view.focus).copied().unwrap_or(0.0));
        w.set_plate_height(layout.heights.get(view.focus).copied().unwrap_or(list::ROW_HEIGHT));
        w.set_scroll(layout.scroll);
        w.set_hints(Rc::new(VecModel::from(self.hints(&view))).into());
        if let Some(b) = app.ctx.battery {
            let pct = b.percent.map_or("–".to_string(), |p| format!("{p}%"));
            w.set_battery(if b.charging { format!("{pct} charging") } else { pct }.into());
        }
        if let Some(t) = t {
            let from = match t { Transition::Push => 64.0, Transition::Pop => -64.0, Transition::Replace => 0.0 };
            w.set_animating(false);
            w.set_shift(from);
            w.set_fade(0.0);
            w.set_animating(true);
            w.set_shift(0.0);
            w.set_fade(1.0);
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

/// Desktop keys: arrows = d-pad, X = A, Z = B, S = X, A = Y, Q/W = L1/R1, 1/2 = L2/R2,
/// Enter = Start, Tab = Select, Esc = Menu, -/= = volume, P = power.
fn key_button(text: &str) -> Option<Button> {
    use slint::platform::Key;
    let is = |k: Key| text == SharedString::from(k).as_str();
    use Button::*;
    Some(if is(Key::UpArrow) { Up } else if is(Key::DownArrow) { Down } else if is(Key::LeftArrow) { Left }
        else if is(Key::RightArrow) { Right } else if is(Key::Return) { Start } else if is(Key::Tab) { Select }
        else if is(Key::Escape) { Menu } else {
            match text.to_ascii_lowercase().as_str() {
                "x" => A, "z" => B, "s" => X, "a" => Y, "q" => L1, "w" => R1, "1" => L2, "2" => R2,
                "-" => VolDown, "=" => VolUp, "p" => Power, _ => return None,
            }
        })
}

/// For development and visual checks: `SPECULAR_SCRIPT="down,down,a"` presses those buttons
/// (named as in `key_button`'s keys: up, down, left, right, a, b, x, y, l1, r1, start, select,
/// menu) 400 ms apart, starting after 1.5 s, then saves the window as raw RGBA to
/// `SPECULAR_SNAPSHOT` (default /tmp/specular.rgba) and quits.
fn script(window: &AppWindow, keys: &str) -> slint::Timer {
    use Button::*;
    let mut steps: Vec<Button> = keys.split(',').filter_map(|k| Some(match k.trim() {
        "up" => Up, "down" => Down, "left" => Left, "right" => Right, "a" => A, "b" => B, "x" => X, "y" => Y,
        "l1" => L1, "r1" => R1, "start" => Start, "select" => Select, "menu" => Menu, _ => return None,
    })).collect();
    steps.reverse();
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut started = false;
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(400), move || {
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
