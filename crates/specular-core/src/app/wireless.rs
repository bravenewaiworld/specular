//! WiFi and Bluetooth in the app: what the radios report (`WifiState`, `BtState`), their screens,
//! and `App::reconcile_radios`: the radios as the settings and screens want them, and the sound
//! output as the radios and the headphone jack allow (also on a device without radios).
//!
//! WiFi on Auto is on while something needs it (`WifiNeed`, or the Wi-Fi screen being open),
//! then for `WIFI_GRACE` more, so needs that come and go don't keep restarting the radio. A need
//! made while WiFi isn't connected lapses after `NEED_TIMEOUT` unless it connects by then, so a
//! place with no known network doesn't keep the radio on; once connected, a need holds until it
//! is dropped.

use std::collections::HashMap;
use std::time::Duration;

use super::{App, Ctx, Effect, Outcome, Screen, action, screens, setting_press};
use crate::device::Output;
use crate::input::{Button, Press};
use crate::model::{Icon, Layout, Row, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;
use crate::settings::{Layer, Visibility, builtin};
use crate::wireless::Secret;
use crate::wireless::bluetooth::{BtCmd, BtDevice, BtEvent, BtKind};
use crate::wireless::wifi::{self, Interface, JoinStep, Network, WifiCmd, WifiEvent, WifiLink};

/// How long WiFi on Auto stays on after the last need is gone.
pub const WIFI_GRACE: Duration = Duration::from_secs(60);
/// How long a need keeps WiFi on Auto on while it doesn't connect.
pub const NEED_TIMEOUT: Duration = Duration::from_secs(30);

/// Why WiFi is needed, while it is on Auto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WifiNeed {
    /// Box art to download.
    BoxArt,
}

/// What the WiFi service last reported.
#[derive(Clone, Debug, PartialEq)]
pub struct WifiState {
    pub link: WifiLink,
    pub networks: Vec<Network>,
    /// A saved network that last refused its password: joining it asks for a new one.
    pub wrong_password: Option<String>,
    /// The device's other interfaces (not WiFi's).
    pub interfaces: Vec<Interface>,
}

impl Default for WifiState {
    fn default() -> Self { Self { link: WifiLink::Off, networks: Vec::new(), wrong_password: None, interfaces: Vec::new() } }
}

/// What the Bluetooth service last reported, and the command the user is waiting on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtState {
    pub powered: bool,
    pub searching: bool,
    pub devices: Vec<BtDevice>,
    /// A pair, connect or disconnect still under way: shown on its device's row until the
    /// device's state shows it done, or it fails.
    pub busy: Option<BtCmd>,
}

impl BtState {
    fn device(&self, address: &str) -> Option<&BtDevice> { self.devices.iter().find(|d| d.address.0 == address) }

    /// Whether the busy command is done, as the devices now show.
    fn busy_done(&self) -> bool {
        let Some(cmd) = &self.busy else { return true };
        let state = |a: &crate::wireless::bluetooth::BtAddress| self.device(&a.0).map(|d| (d.paired, d.connected));
        match cmd {
            BtCmd::Pair(a) => state(a).is_none_or(|(paired, _)| paired),
            BtCmd::Connect(a) => state(a).is_none_or(|(_, connected)| connected),
            BtCmd::Disconnect(a) => state(a).is_none_or(|(_, connected)| !connected),
            _ => true,
        }
    }
}

/// What the radios and sound were last told, so only changes are sent.
#[derive(Debug)]
pub(super) struct Radios {
    /// Nothing is sent before the settings are loaded: they say what the radios should do.
    pub settings_loaded: bool,
    wifi_power: Option<bool>,
    /// A scan was asked for since the Wi-Fi screen opened (or WiFi came on with it open).
    scanned: bool,
    bt_power: Option<bool>,
    bt_search: bool,
    pub output: Output,
    /// A Bluetooth output that failed to open: not used again until its device stops taking
    /// sound (disconnects), so a reconnection tries it again.
    failed: Option<Output>,
    /// Needs, and when each was made; None once WiFi connected (it then holds until dropped).
    needs: HashMap<WifiNeed, Option<Duration>>,
    /// Whether WiFi was needed at the last reconcile, and until when it stays on after that.
    needed: bool,
    grace_until: Duration,
}

impl Default for Radios {
    fn default() -> Self {
        Self { settings_loaded: false, wifi_power: None, scanned: false, bt_power: None, bt_search: false, output: Output::Speaker, failed: None,
               needs: HashMap::new(), needed: false, grace_until: Duration::ZERO }
    }
}

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::WIFI, Box::new(WifiScreen));
    screens.insert(screens::BLUETOOTH, Box::new(BluetoothScreen));
    screens.insert(screens::FORGET, Box::new(ForgetScreen));
}

fn setting<'a>(ctx: &'a Ctx, key: &str) -> Option<&'a serde_json::Value> { ctx.settings.get(key, &Layer::Global).map(|(v, _)| v) }
fn wifi_mode(ctx: &Ctx) -> &str { setting(ctx, builtin::WIFI).and_then(|v| v.as_str()).unwrap_or("off") }
fn bluetooth_on(ctx: &Ctx) -> bool { setting(ctx, builtin::BLUETOOTH).and_then(|v| v.as_bool()).unwrap_or(false) }

impl App {
    /// Says which radios the device has (none until told). Call before the first message.
    pub fn has_radios(&mut self, wifi: bool, bluetooth: bool) {
        self.ctx.wifi = wifi.then(WifiState::default);
        self.ctx.bluetooth = bluetooth.then(BtState::default);
        self.changed();
    }

    /// When WiFi on Auto may next need turning off (a need lapsing, or the grace ending), so the
    /// caller can send a `Tick` then.
    pub fn wifi_deadline(&self) -> Option<Duration> {
        let lapses = self.radios.needs.values().flatten().map(|since| *since + NEED_TIMEOUT);
        lapses.chain([self.radios.grace_until]).filter(|t| *t > self.now).min()
    }

    pub(super) fn wifi_need(&mut self, need: WifiNeed, wanted: bool) {
        if !wanted { self.radios.needs.remove(&need); return; }
        let connected = matches!(self.ctx.wifi.as_ref().map(|w| &w.link), Some(WifiLink::Connected { .. }));
        // Asking again restarts a lapsing need's time; a connected one holds already.
        if self.radios.needs.get(&need) != Some(&None) { self.radios.needs.insert(need, (!connected).then_some(self.now)); }
    }

    fn wifi_needed(&self) -> bool {
        self.nav.contains(&screens::WIFI) || self.radios.needs.values().any(|since| since.is_none_or(|t| self.now < t + NEED_TIMEOUT))
    }

    /// Whether failures should be shown: only on the radio's own screen, where the user asked
    /// for something (they are in the trace either way).
    fn on_screen(&self, id: &ScreenId) -> bool { self.nav.contains(id) }

    pub(super) fn wifi_event(&mut self, e: WifiEvent) -> Vec<Effect> {
        let notify = self.on_screen(&screens::WIFI);
        let Some(w) = &mut self.ctx.wifi else { return vec![] };
        let mut out = vec![];
        match e {
            WifiEvent::Link(link) => {
                if let WifiLink::Connected { ssid, .. } = &link {
                    self.radios.needs.values_mut().for_each(|since| *since = None);
                    if w.wrong_password.as_ref() == Some(ssid) { w.wrong_password = None; }
                }
                w.link = link;
            }
            WifiEvent::Networks(n) => w.networks = n,
            WifiEvent::Interfaces(i) => w.interfaces = i,
            WifiEvent::Failed(err) if notify => {
                if let wifi::WifiError::WrongPassword { ssid } = &err { w.wrong_password = Some(ssid.clone()); }
                out.push(Effect::Notice(err.message()));
            }
            WifiEvent::Failed(wifi::WifiError::WrongPassword { ssid }) => w.wrong_password = Some(ssid),
            WifiEvent::Failed(_) => {}
        }
        self.changed();
        out.push(Effect::Redraw(None));
        out
    }

    pub(super) fn bluetooth_event(&mut self, e: BtEvent) -> Vec<Effect> {
        let notify = self.on_screen(&screens::BLUETOOTH);
        let Some(b) = &mut self.ctx.bluetooth else { return vec![] };
        let mut out = vec![];
        match e {
            BtEvent::Powered(on) => { b.powered = on; b.searching &= on; }
            BtEvent::Searching(on) => b.searching = on,
            BtEvent::Devices(d) => {
                b.devices = d;
                if b.busy_done() { b.busy = None; }
            }
            BtEvent::Failed(err) => {
                b.busy = None;
                if notify { out.push(Effect::Notice(err.message())); }
            }
        }
        self.changed();
        out.push(Effect::Redraw(None));
        out
    }

    pub(super) fn sound_failed(&mut self, output: Output) -> Vec<Effect> {
        let Output::Bluetooth(address) = &output else { return vec![] };
        let name = self.ctx.bluetooth.as_ref().and_then(|b| b.device(&address.0)).map_or(address.0.clone(), |d| d.name.clone());
        self.radios.failed = Some(output);
        vec![Effect::Notice(format!("Couldn't play sound on {name}"))]
    }

    /// Commands that bring the radios and the sound output to what the settings and screens
    /// want now; sent after every message.
    pub(super) fn reconcile_radios(&mut self) -> Vec<Effect> {
        if !self.radios.settings_loaded { return vec![]; }
        let mut e = vec![];
        if self.ctx.wifi.is_some() {
            let needed = self.wifi_needed();
            if self.radios.needed && !needed { self.radios.grace_until = self.now + WIFI_GRACE; }
            self.radios.needed = needed;
            let keep_on = setting(&self.ctx, builtin::WIFI_KEEP_ON) == Some(&true.into());
            let want = match wifi_mode(&self.ctx) {
                "on" => true,
                "off" => false,
                _ => keep_on || needed || self.now < self.radios.grace_until,
            };
            if self.radios.wifi_power != Some(want) {
                self.radios.wifi_power = Some(want);
                self.radios.scanned = false;
                e.push(Effect::Wifi(WifiCmd::Power(want)));
            }
            let screen = self.nav.contains(&screens::WIFI);
            if want && screen && !self.radios.scanned { e.push(Effect::Wifi(WifiCmd::Scan)); }
            self.radios.scanned = want && screen;
        }
        let mut headsets = vec![];
        if let Some(b) = &self.ctx.bluetooth {
            let power = bluetooth_on(&self.ctx);
            let search = power && self.nav.contains(&screens::BLUETOOTH);
            // Searching starts after powering on, and stops before powering off.
            let mut cmds = vec![];
            if self.radios.bt_power != Some(power) { cmds.push(BtCmd::Power(power)); }
            if self.radios.bt_search != search { cmds.insert(if search { cmds.len() } else { 0 }, BtCmd::Search(search)); }
            (self.radios.bt_power, self.radios.bt_search) = (Some(power), search);
            e.extend(cmds.into_iter().map(Effect::Bluetooth));
            headsets = b.devices.iter().filter(|d| d.audio && power).map(|d| Output::Bluetooth(d.address.clone())).collect();
        }
        // A Bluetooth device taking sound comes first, then headphones in the jack.
        if self.radios.failed.as_ref().is_some_and(|f| !headsets.contains(f)) { self.radios.failed = None; }
        let wired = if self.headphones { Output::Headphones } else { Output::Speaker };
        let output = headsets.into_iter().find(|o| Some(o) != self.radios.failed.as_ref()).unwrap_or(wired);
        if self.radios.output != output {
            self.radios.output = output.clone();
            e.push(Effect::SoundTo(output));
        }
        e
    }
}

/// The settings screen's "Wireless" section: a row per radio, opening its screen.
pub(super) fn settings_rows(ctx: &Ctx) -> Vec<Row> {
    let wifi = ctx.wifi.as_ref().map(|w| {
        let value = match (&w.link, wifi_mode(ctx)) {
            (_, "off") => "Off".to_string(),
            (WifiLink::Connected { ssid, .. }, _) => ssid.clone(),
            (WifiLink::Connecting { .. }, _) => "Connecting…".into(),
            (_, "auto") => "Auto".into(),
            _ => "Not connected".into(),
        };
        Row::new("open:wifi", "Wi-Fi", RowKind::Action { value: Some(value) })
    });
    let bluetooth = ctx.bluetooth.as_ref().map(|b| {
        let connected = b.devices.iter().find(|d| d.connected).filter(|_| bluetooth_on(ctx));
        let value = if !bluetooth_on(ctx) { "Off".to_string() } else { connected.map_or("On".into(), |d| d.name.clone()) };
        Row::new("open:bluetooth", "Bluetooth", RowKind::Action { value: Some(value) })
    });
    let rows: Vec<Row> = wifi.into_iter().chain(bluetooth).collect();
    if rows.is_empty() { return rows; }
    std::iter::once(Row::new("section:Wireless", "Wireless", RowKind::Header)).chain(rows).collect()
}

/// The status bar's icons for the radios: WiFi when it is on (no bars until connected, then its
/// live strength), Bluetooth when it is powered (lit while a device is connected). Neither
/// shows a padlock or says what the connected device is.
pub(super) fn status_icons(ctx: &Ctx) -> Vec<Icon> {
    let wifi = ctx.wifi.as_ref().and_then(|w| match w.link {
        WifiLink::Off => None,
        WifiLink::Idle | WifiLink::Connecting { .. } => Some(Icon::Wifi { bars: Some(0), locked: false }),
        WifiLink::Connected { bars, .. } => Some(Icon::Wifi { bars, locked: false }),
    });
    let bluetooth = ctx.bluetooth.as_ref().filter(|b| b.powered).map(|b| Icon::Bluetooth { kind: BtKind::Other, lit: b.devices.iter().any(|d| d.connected) });
    wifi.into_iter().chain(bluetooth).collect()
}

/// A press on a row of `settings_rows`: A opens its screen.
pub(super) fn settings_press(row: &Row, p: Press) -> Option<Outcome> {
    if p.button != Button::A { return None; }
    let id = match row.id.0.as_str() { "open:wifi" => screens::WIFI, "open:bluetooth" => screens::BLUETOOTH, _ => return None };
    Some(Outcome::Push(ScreenState::new(id, None)))
}


/// The Wi-Fi screen: the setting, the device's address while connected, and with Developer
/// details its other interfaces' addresses even with WiFi off (to find it over a USB link), then
/// the networks (the current one first, then by signal, then saved ones out of range). A
/// network's row id is "net:" and its name.
struct WifiScreen;

impl WifiScreen {
    fn network<'a>(ctx: &'a Ctx, row: &Row) -> Option<&'a Network> {
        let ssid = row.id.0.strip_prefix("net:")?;
        ctx.wifi.as_ref()?.networks.iter().find(|n| n.ssid == ssid)
    }
}

/// What a network's row says about it (its signal and security are in its icon).
fn network_detail(n: &Network, link: &WifiLink) -> String {
    match link {
        WifiLink::Connected { ssid, .. } if *ssid == n.ssid => return "Connected".into(),
        WifiLink::Connecting { ssid, step } if *ssid == n.ssid => return match step {
            JoinStep::Associating => "Connecting…",
            JoinStep::Authenticating => "Checking password…",
            JoinStep::GettingAddress => "Getting an address…",
        }.into(),
        _ => {}
    }
    if !n.supported { return "Not supported".into(); }
    match (n.saved, n.in_range) {
        (true, false) => "Saved · out of range",
        (true, true) => "Saved",
        (false, _) => "",
    }.into()
}

/// A network's icon: its signal and whether it has a password. The signal is the live reading
/// while connected to it (if there is one yet), else the last scan's.
fn network_icon(n: &Network, link: &WifiLink) -> Icon {
    let live = match link { WifiLink::Connected { ssid, bars, .. } if *ssid == n.ssid => *bars, _ => None };
    Icon::Wifi { bars: Some(live.unwrap_or(n.bars)), locked: n.secured }
}

impl Screen for WifiScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let mut rows: Vec<Row> = ctx.settings.row(builtin::WIFI, &Layer::Global).into_iter().collect();
        let mode = wifi_mode(ctx);
        let address = |id: String, label: &str, address: &str| Row::new(id, label, RowKind::Note { detail: Some(address.into()) });
        let wifi = ctx.wifi.as_ref().filter(|_| mode != "off");
        if mode == "auto" && wifi.is_some() { rows.push(Row::note("note:auto", "On only while needed")); }
        if let Some(WifiLink::Connected { address: a, .. }) = wifi.map(|w| &w.link) { rows.push(address("address".into(), "Address", a)); }
        if ctx.visibility() == Visibility::Developer && let Some(w) = &ctx.wifi {
            rows.extend(w.interfaces.iter().map(|i| address(format!("address:{}:{}", i.name, i.address), &i.name, &i.address)));
        }
        if let Some(w) = wifi {
            rows.push(Row::new("section:networks", "Networks", RowKind::Header));
            if w.link == WifiLink::Off {
                rows.push(Row::note("note:starting", "Starting…"));
            } else if w.networks.is_empty() {
                rows.push(Row::note("note:looking", "Looking for networks…"));
            }
            let current = match &w.link { WifiLink::Connected { ssid, .. } | WifiLink::Connecting { ssid, .. } => Some(ssid), _ => None };
            let mut networks: Vec<&Network> = w.networks.iter().collect();
            networks.sort_by_key(|n| (Some(&n.ssid) != current, !n.in_range, std::cmp::Reverse(n.bars), n.ssid.to_lowercase()));
            rows.extend(networks.iter().map(|n| {
                Row::new(format!("net:{}", n.ssid), &n.ssid, RowKind::Text { detail: Some(network_detail(n, &w.link)) }).icon(network_icon(n, &w.link))
            }));
        }
        ScreenView {
            title: "Wi-Fi".into(), layout: Layout::List, rows,
            hints: vec![(Button::Y, "Search".into()), (Button::X, "Forget".into()), (Button::B, "Back".into()), (Button::A, "Join".into())],
            entry: None,
        }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        let powered = ctx.wifi.as_ref().is_some_and(|w| w.link != WifiLink::Off);
        match (p.button, focused) {
            (Button::B, _) => Outcome::Back,
            (Button::Y, _) if !p.repeat && powered => Outcome::Effects(vec![Effect::Wifi(WifiCmd::Scan)]),
            (_, Some(row)) if row.id.0 == builtin::WIFI => setting_press(ctx, row, p),
            (Button::A, Some(row)) => {
                let Some(n) = Self::network(ctx, row) else { return Outcome::None };
                let connected = matches!(&ctx.wifi.as_ref().unwrap().link, WifiLink::Connected { ssid, .. } if *ssid == n.ssid);
                if connected { return Outcome::None; }
                if !n.supported { return Outcome::Effects(vec![Effect::Notice(wifi::WifiError::Unsupported { ssid: n.ssid.clone(), detail: String::new() }.message())]); }
                let refused = ctx.wifi.as_ref().unwrap().wrong_password.as_ref() == Some(&n.ssid);
                if (n.saved && !refused) || !n.secured { return Outcome::Effects(vec![Effect::Wifi(WifiCmd::Join { ssid: n.ssid.clone(), password: None })]); }
                Outcome::Type { prompt: format!("Password for {}", n.ssid), masked: true, purpose: n.ssid.clone() }
            }
            (Button::X, Some(row)) if !p.repeat => match Self::network(ctx, row) {
                Some(n) if n.saved => Outcome::Push(ScreenState::new(screens::FORGET, Some(Forget::Wifi(n.ssid.clone()).param()))),
                _ => Outcome::None,
            },
            _ => Outcome::None,
        }
    }

    /// A password typed for the network `ssid`.
    fn entered(&self, _: &mut Ctx, _: &mut ScreenState, ssid: &str, text: String) -> Result<Outcome, String> {
        if !wifi::valid_password(&text) {
            return Err(wifi::WifiError::BadPassword { ssid: ssid.into() }.message());
        }
        Ok(Outcome::Effects(vec![Effect::Wifi(WifiCmd::Join { ssid: ssid.into(), password: Some(Secret::new(text)) })]))
    }
}

/// The Bluetooth screen: the setting, then paired devices and the devices found by searching
/// (which runs while this screen is open). A device's row id is "dev:" and its address.
struct BluetoothScreen;

impl BluetoothScreen {
    fn device<'a>(ctx: &'a Ctx, row: &Row) -> Option<&'a BtDevice> {
        ctx.bluetooth.as_ref()?.device(row.id.0.strip_prefix("dev:")?)
    }
}

/// What a device's row says about it (what it is is in its icon).
fn device_detail(d: &BtDevice, busy: Option<&BtCmd>) -> String {
    match busy {
        Some(BtCmd::Pair(a)) if *a == d.address => return "Pairing…".into(),
        Some(BtCmd::Connect(a)) if *a == d.address => return "Connecting…".into(),
        Some(BtCmd::Disconnect(a)) if *a == d.address => return "Disconnecting…".into(),
        _ => {}
    }
    if d.paired {
        return match (d.connected, d.audio) { (true, true) => "Connected · sound", (true, false) => "Connected", _ => "Not connected" }.into();
    }
    String::new()
}

impl Screen for BluetoothScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let mut rows: Vec<Row> = ctx.settings.row(builtin::BLUETOOTH, &Layer::Global).into_iter().collect();
        if let Some(b) = ctx.bluetooth.as_ref().filter(|_| bluetooth_on(ctx)) {
            let row = |d: &BtDevice| {
                Row::new(format!("dev:{}", d.address.0), &d.name, RowKind::Text { detail: Some(device_detail(d, b.busy.as_ref())) })
                    .icon(Icon::Bluetooth { kind: d.kind, lit: d.connected })
            };
            if !b.powered {
                rows.push(Row::note("note:starting", "Starting…"));
            } else {
                let (paired, others): (Vec<&BtDevice>, Vec<&BtDevice>) = b.devices.iter().partition(|d| d.paired);
                if !paired.is_empty() {
                    rows.push(Row::new("section:paired", "My devices", RowKind::Header));
                    rows.extend(paired.into_iter().map(row));
                }
                rows.push(Row::new("section:others", "Other devices", RowKind::Header));
                if others.is_empty() { rows.push(Row::note("note:looking", if b.searching { "Looking for devices…" } else { "None found" })); }
                rows.extend(others.into_iter().map(row));
            }
        }
        ScreenView {
            title: "Bluetooth".into(), layout: Layout::List, rows,
            hints: vec![(Button::X, "Forget".into()), (Button::B, "Back".into()), (Button::A, "Select".into())],
            entry: None,
        }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused) {
            (Button::B, _) => Outcome::Back,
            (_, Some(row)) if row.id.0 == builtin::BLUETOOTH => setting_press(ctx, row, p),
            (Button::A, Some(row)) if !p.repeat => {
                let Some(d) = Self::device(ctx, row) else { return Outcome::None };
                let a = d.address.clone();
                let cmd = if !d.paired { BtCmd::Pair(a) } else if d.connected { BtCmd::Disconnect(a) } else { BtCmd::Connect(a) };
                ctx.bluetooth.as_mut().unwrap().busy = Some(cmd.clone());
                Outcome::Effects(vec![Effect::Bluetooth(cmd), Effect::Redraw(None)])
            }
            (Button::X, Some(row)) if !p.repeat => match Self::device(ctx, row) {
                Some(d) if d.paired => Outcome::Push(ScreenState::new(screens::FORGET, Some(Forget::Bluetooth(d.address.0.clone()).param()))),
                _ => Outcome::None,
            },
            _ => Outcome::None,
        }
    }
}

/// What the forget screen forgets, as its params: "wifi:<ssid>" or "bluetooth:<address>".
enum Forget { Wifi(String), Bluetooth(String) }

impl Forget {
    fn param(&self) -> String {
        match self { Forget::Wifi(ssid) => format!("wifi:{ssid}"), Forget::Bluetooth(a) => format!("bluetooth:{a}") }
    }

    fn parse(param: &str) -> Option<Self> {
        let (kind, what) = param.split_once(':')?;
        match kind { "wifi" => Some(Forget::Wifi(what.into())), "bluetooth" => Some(Forget::Bluetooth(what.into())), _ => None }
    }

    /// Its name for the user: the network's, or the device's (its address if no longer known).
    fn name(&self, ctx: &Ctx) -> String {
        match self {
            Forget::Wifi(ssid) => ssid.clone(),
            Forget::Bluetooth(a) => ctx.bluetooth.as_ref().and_then(|b| b.device(a)).map_or(a.clone(), |d| d.name.clone()),
        }
    }
}

/// "Forget …?": confirms forgetting a saved network or a paired device.
struct ForgetScreen;

impl Screen for ForgetScreen {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let name = state.params.as_deref().and_then(Forget::parse).map_or_else(String::new, |f| f.name(ctx));
        ScreenView {
            title: format!("Forget {name}?"), layout: Layout::List,
            rows: vec![action("cancel", "Cancel"), action("forget", "Forget")],
            hints: vec![(Button::B, "Cancel".into()), (Button::A, "Select".into())],
            entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        let forget = focused.is_some_and(|r| r.id.0 == "forget");
        match (p.button, state.params.as_deref().and_then(Forget::parse)) {
            (Button::A, Some(Forget::Wifi(ssid))) if forget => Outcome::BackWith(vec![Effect::Wifi(WifiCmd::Forget { ssid })]),
            (Button::A, Some(Forget::Bluetooth(a))) if forget => Outcome::BackWith(vec![Effect::Bluetooth(BtCmd::Forget(crate::wireless::bluetooth::BtAddress(a)))]),
            (Button::A | Button::B, _) => Outcome::Back,
            _ => Outcome::None,
        }
    }
}
