//! Specular's UI: draws the core's screens with Slint and carries out the app's effects. All
//! slow work happens off the UI thread: files on an I/O thread (and the library scan on its
//! own), hardware on a hardware thread; their results come back as messages to the app.

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
use specular_core::model::{RowKind, ScreenView};
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
    let runner = Rc::new(Runner {
        app: RefCell::new(App::new(specular_core::library::builtin_systems(), builtin::settings())),
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

/// Delivers a message to the app on the UI thread, from any thread.
fn post(msg: Msg) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(r) = RUNNER.with(|r| r.borrow().clone()) { r.handle(msg); }
    });
}

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
}

impl Runner {
    fn handle(self: &Rc<Self>, msg: Msg) {
        match &msg {
            Msg::Button(e) if e.button == Button::Power && e.time < self.ignore_power_until.get() => return,
            Msg::Woke => {
                self.ignore_power_until.set(now() + WAKE_GRACE);
                self.tick_clock(); // timers stood still while asleep
            }
            _ => {}
        }
        let effects = self.app.borrow_mut().handle(msg);
        for e in effects { self.apply(e); }
        self.schedule_repeat();
    }

    fn apply(self: &Rc<Self>, e: Effect) {
        match e {
            Effect::Redraw(t) => self.refresh(t),
            Effect::SaveSettings(layer) => {
                let values = self.app.borrow().ctx.settings.layer(&layer);
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
        let layout = list::layout(&view.rows, focus, w.get_list_height());
        // Rows are drawn from a fixed set of slots, enough for every row near the view; a row
        // always uses slot `index % slots`, so while the list scrolls a row keeps its slot and
        // only the slots of rows coming into view change. A new screen gets fresh slots, so the
        // old screen's rows don't animate into the new one.
        let slots = list::max_visible(w.get_list_height());
        let mut wanted = vec![RowData::default(); slots];
        for i in layout.visible.clone() {
            wanted[i % slots] = row_data(&view.rows[i], i, layout.tops[i], layout.heights[i]);
        }
        if t.is_none() && self.rows.row_count() == slots {
            for (slot, r) in wanted.into_iter().enumerate() { if self.rows.row_data(slot).as_ref() != Some(&r) { self.rows.set_row_data(slot, r); } }
        } else {
            self.rows.set_vec(wanted);
        }
        let hints = self.hints(&view);
        if self.hints.iter().ne(hints.iter().cloned()) { self.hints.set_vec(hints); }
        w.set_screen_title(view.title.as_str().into());
        w.set_crumb(app.parent_title().unwrap_or_default().into());
        w.set_focused_row(focus as i32);
        w.set_plate_top(layout.tops.get(focus).copied().unwrap_or(0.0));
        w.set_plate_height(layout.heights.get(focus).copied().unwrap_or(list::ROW_HEIGHT));
        w.set_scroll(layout.scroll);
        if let Some(b) = app.ctx.battery {
            let pct = b.percent.map_or("–".to_string(), |p| format!("{p}%"));
            w.set_battery(if b.charging { format!("{pct} charging") } else { pct }.into());
        }
        if let Some(t) = t {
            w.set_shift(if t == Transition::Push { 64.0 } else { -64.0 });
            w.set_transitions(w.get_transitions() + 1);
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

/// Row `index` of a screen, ready to draw at `top` (u).
fn row_data(r: &specular_core::model::Row, index: usize, top: f32, height: f32) -> RowData {
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
        enabled: r.enabled, overridden: r.overridden, top, height,
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
