//! Bluetooth through BlueZ (bluetoothd), over the system D-Bus. One thread follows BlueZ's
//! objects (the adapter, devices, and audio connections) and reports what changed; commands are
//! method calls, the slow ones (pairing, connecting: tens of seconds) each on a thread of their
//! own so later commands aren't held up. Every exchange with BlueZ is traced as `send` / `recv`.
//!
//! When the bus or bluetoothd isn't there (yet: it may start after Specular), the service reports
//! `NotAvailable` once and keeps trying every few seconds. BlueZ without an adapter is Bluetooth
//! off, not a failure: with a `BtController`, the adapter is gone while the controller is off and
//! appears a few seconds after it is powered on. So an adapter that never appears (a controller
//! that fails to start, or no Bluetooth hardware) shows only as Bluetooth staying off.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use specular_core::wireless::bluetooth::{BtAddress, BtCmd, BtDevice, BtError, BtEvent, BtKind};
use specular_core::wireless::{Events, Wireless, trace};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Value};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const TRANSPORT: &str = "org.bluez.MediaTransport1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const AGENT_PATH: &str = "/org/specular/agent";
/// A2DP Source: an audio connection on which we are the source, so the device is the sink.
const A2DP_SOURCE: &str = "0000110a-0000-1000-8000-00805f9b34fb";
/// How long to wait before trying to reach BlueZ again.
const RETRY: Duration = Duration::from_secs(5);
/// The longest a call may take (BlueZ answers a stalled pairing itself, well before this).
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// The interfaces Specular follows, and the properties it reads from each; the rest are dropped.
const KEPT: &[(&str, &[&str])] = &[
    (ADAPTER, &["Powered", "Discovering"]),
    (DEVICE, &["Address", "Name", "Icon", "Paired", "Connected", "Adapter"]),
    (TRANSPORT, &["Device", "UUID"]),
];

/// A property value Specular reads (object paths are kept as text).
#[derive(Clone, Debug, PartialEq)]
enum Prop { Bool(bool), Text(String) }

type Props = BTreeMap<String, Prop>;

/// The properties of `iface` in `raw` that Specular reads, or None for an interface it doesn't
/// follow.
fn kept_props(iface: &str, raw: &HashMap<String, OwnedValue>) -> Option<Props> {
    let (_, names) = KEPT.iter().find(|(i, _)| *i == iface)?;
    Some(raw.iter().filter(|(k, _)| names.contains(&k.as_str())).filter_map(|(k, v)| {
        let p = match &**v {
            Value::Bool(b) => Prop::Bool(*b),
            Value::Str(s) => Prop::Text(s.to_string()),
            Value::ObjectPath(p) => Prop::Text(p.to_string()),
            _ => return None,
        };
        Some((k.clone(), p))
    }).collect())
}

/// What BlueZ has, as far as Specular cares: object path → interface → properties.
#[derive(Default, Debug)]
struct Objects(BTreeMap<String, BTreeMap<String, Props>>);

impl Objects {
    fn added(&mut self, path: &str, ifaces: &HashMap<String, HashMap<String, OwnedValue>>) {
        for (iface, raw) in ifaces {
            if let Some(props) = kept_props(iface, raw) { self.0.entry(path.into()).or_default().insert(iface.clone(), props); }
        }
    }

    fn removed(&mut self, path: &str, ifaces: &[String]) {
        let Some(o) = self.0.get_mut(path) else { return };
        ifaces.iter().for_each(|i| { o.remove(i); });
        if o.is_empty() { self.0.remove(path); }
    }

    /// Applies a PropertiesChanged signal (only to interfaces already known).
    fn changed(&mut self, path: &str, iface: &str, props: Props, invalidated: &[String]) {
        let Some(p) = self.0.get_mut(path).and_then(|o| o.get_mut(iface)) else { return };
        invalidated.iter().for_each(|k| { p.remove(k); });
        p.extend(props);
    }

    fn get(&self, path: &str, iface: &str) -> Option<&Props> { self.0.get(path)?.get(iface) }

    fn with<'a>(&'a self, iface: &'a str) -> impl Iterator<Item = (&'a str, &'a Props)> {
        self.0.iter().filter_map(move |(path, o)| Some((path.as_str(), o.get(iface)?)))
    }

    /// The first adapter (the Brick has one).
    fn adapter(&self) -> Option<&str> { self.with(ADAPTER).next().map(|(p, _)| p) }

    fn adapter_state(&self) -> AdapterState {
        let a = self.with(ADAPTER).next();
        AdapterState { present: a.is_some(), powered: a.is_some_and(|(_, a)| flag(a, "Powered")) }
    }

    fn devices(&self, adapter: &str) -> Vec<BtDevice> {
        self.with(DEVICE).filter(|(_, p)| text(p, "Adapter") == Some(adapter)).filter_map(|(path, p)| {
            let connected = flag(p, "Connected");
            Some(BtDevice {
                address: BtAddress(text(p, "Address")?.into()),
                name: text(p, "Name")?.into(),
                kind: kind(text(p, "Icon").unwrap_or("")),
                paired: flag(p, "Paired"),
                connected,
                audio: connected && self.with(TRANSPORT).any(|(_, t)| text(t, "Device") == Some(path) && text(t, "UUID") == Some(A2DP_SOURCE)),
            })
        }).collect()
    }

    /// The device with `address`: its object path and the name to show in errors.
    fn device(&self, address: &BtAddress) -> Option<(String, String)> {
        let (path, p) = self.with(DEVICE).find(|(_, p)| text(p, "Address") == Some(&address.0))?;
        Some((path.into(), text(p, "Name").unwrap_or(&address.0).into()))
    }

    /// The adapter's state; with no adapter, off with no devices.
    fn state(&self) -> State {
        let Some((adapter, a)) = self.with(ADAPTER).next() else { return State::default() };
        State { powered: flag(a, "Powered"), searching: flag(a, "Discovering"), devices: self.devices(adapter) }
    }
}

fn text<'a>(p: &'a Props, name: &str) -> Option<&'a str> { match p.get(name)? { Prop::Text(t) => Some(t), _ => None } }
fn flag(p: &Props, name: &str) -> bool { p.get(name) == Some(&Prop::Bool(true)) }

/// A device's kind from its BlueZ icon name (which BlueZ derives from its class).
fn kind(icon: &str) -> BtKind {
    match icon {
        "audio-headphones" | "audio-headset" => BtKind::Headphones,
        "audio-card" | "audio-speakers" => BtKind::Speaker,
        "input-gaming" => BtKind::Controller,
        "input-keyboard" => BtKind::Keyboard,
        "phone" => BtKind::Phone,
        "computer" => BtKind::Computer,
        _ => BtKind::Other,
    }
}

/// Whether BlueZ has an adapter, and whether it is powered.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AdapterState { present: bool, powered: bool }

/// The wanted commands due as the adapter went from `before` to `now`. The power when an adapter
/// appears, or when its power changes to other than wanted (BlueZ powers a new adapter itself, as
/// configured, racing our own request). The search once it is powered: BlueZ refuses to search
/// with an unpowered adapter.
fn due(before: AdapterState, now: AdapterState, wanted: Wanted) -> Vec<BtCmd> {
    let mut cmds = vec![];
    if !now.present { return cmds; }
    let changed = !before.present || before.powered != now.powered;
    if let Some(on) = wanted.power && changed && on != now.powered { cmds.push(BtCmd::Power(on)); }
    if wanted.search && now.powered && !before.powered { cmds.push(BtCmd::Search(true)); }
    cmds
}

/// BlueZ's answer to powering an adapter whose power is already changing (as BlueZ powers a new
/// adapter itself): not a failure, as the change arrives as a property change.
const POWER_CHANGING: &[&str] = &["org.bluez.Error.Busy"];

/// What the app is told: the adapter's state and the devices.
#[derive(Clone, Debug, Default, PartialEq)]
struct State { powered: bool, searching: bool, devices: Vec<BtDevice> }

/// The events that bring an app that last heard `last` (None: nothing yet) up to `now`.
fn changes(last: Option<&State>, now: &State) -> Vec<BtEvent> {
    let mut e = vec![];
    if last.is_none_or(|l| l.powered != now.powered) { e.push(BtEvent::Powered(now.powered)); }
    if last.is_none_or(|l| l.searching != now.searching) { e.push(BtEvent::Searching(now.searching)); }
    if last.is_none_or(|l| l.devices != now.devices) { e.push(BtEvent::Devices(now.devices.clone())); }
    e
}

/// A failed call, as "<D-Bus error name>: <message>".
type Failure = String;

/// The error to report for a failed `step` on the device called `name`.
fn bt_error(step: &str, name: &str, detail: Failure) -> BtError {
    const GONE: &[&str] = &["org.freedesktop.DBus.Error.ServiceUnknown", "org.freedesktop.DBus.Error.NameHasNoOwner", "org.freedesktop.DBus.Error.UnknownObject"];
    if GONE.iter().any(|g| detail.starts_with(g)) { return BtError::NotAvailable { detail }; }
    match step {
        "pair" => BtError::PairingFailed { name: name.into(), detail },
        "connect" => BtError::ConnectFailed { name: name.into(), detail },
        _ => BtError::Service { step: step.into(), detail },
    }
}

/// `result`, with the errors that mean "already done" (D-Bus error names in `done`) as success.
fn tolerate<T>(result: Result<T, Failure>, done: &[&str]) -> Result<(), Failure> {
    match result {
        Err(e) if !done.iter().any(|d| e.starts_with(&format!("{d}:"))) => Err(e),
        _ => Ok(()),
    }
}

/// The connection to BlueZ, while there is one.
struct Link { conn: Connection, objects: Objects }

struct Shared {
    events: Events<BtEvent>,
    link: Mutex<Option<Link>>,
    /// Devices (object paths) Specular is pairing with: the agent agrees only to these.
    pairing: Arc<Mutex<HashSet<String>>>,
    /// The power and search last asked for, applied again as an adapter appears and is powered
    /// (see `due`): asked for before BlueZ or its adapter was there, or lost when bluetoothd
    /// restarted or the controller was powered off (an adapter starts powered as BlueZ is
    /// configured, and searches end with the connection or adapter that started them).
    wanted: Mutex<Wanted>,
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Wanted { power: Option<bool>, search: bool }

/// Switches a device's Bluetooth controller on and off: its own power, which BlueZ's adapter power
/// doesn't cut.
pub trait BtController: Send {
    /// Powers the controller on or off. Returns once asked, not once done: when powered on, the
    /// controller appears to BlueZ as an adapter some seconds later; when powered off, its adapter
    /// goes away.
    fn set_power(&mut self, on: bool) -> io::Result<()>;
}

/// Bluetooth through BlueZ; see the module's description.
#[derive(Default)]
pub struct Bluez { shared: Option<Arc<Shared>>, controller: Option<Box<dyn BtController>> }

impl Bluez {
    /// For a controller that is always powered: `BtCmd::Power` sets only the adapter's power.
    pub fn new() -> Self { Self::default() }

    /// For a controller that `controller` powers: `BtCmd::Power(true)` powers it before the
    /// adapter, and `BtCmd::Power(false)` after the adapter, so devices are disconnected first.
    pub fn with_controller(controller: Box<dyn BtController>) -> Self { Self { shared: None, controller: Some(controller) } }

    /// Powers the controller, if there is one to power, reporting a failure as an event.
    fn power_controller(&mut self, on: bool) {
        let (Some(c), Some(s)) = (&mut self.controller, &self.shared) else { return };
        trace::line("bluetooth", "send", &format!("controller power {on}"));
        let result = c.set_power(on);
        trace::line("bluetooth", "recv", &format!("controller power {on}: {}", result.as_ref().map_or_else(|e| e.to_string(), |()| "ok".into())));
        if let Err(e) = result { (s.events)(BtEvent::Failed(BtError::Service { step: "controller power".into(), detail: e.to_string() })); }
    }
}

impl Wireless<BtCmd, BtEvent> for Bluez {
    fn start(&mut self, events: Events<BtEvent>) {
        let shared = Arc::new(Shared { events, link: Mutex::new(None), pairing: Arc::default(), wanted: Mutex::default() });
        self.shared = Some(shared.clone());
        std::thread::Builder::new().name("bluez".into()).spawn(move || follow(&shared)).expect("spawning the BlueZ thread");
    }

    fn send(&mut self, cmd: BtCmd) {
        let Some(shared) = self.shared.clone() else { return };
        match cmd {
            BtCmd::Power(true) => { self.power_controller(true); command(&shared, cmd); }
            BtCmd::Power(false) => { command(&shared, cmd); self.power_controller(false); }
            BtCmd::Pair(_) | BtCmd::Connect(_) => { std::thread::spawn(move || command(&shared, cmd)); }
            _ => command(&shared, cmd),
        }
    }
}

/// Follows BlueZ for good: a session while it's reachable, retrying when it isn't.
fn follow(s: &Shared) {
    let mut last: Option<State> = None;
    let mut available = true;
    loop {
        let err = session(s, &mut last, &mut available).unwrap_err();
        *s.link.lock().unwrap() = None;
        changes(last.as_ref(), &State::default()).into_iter().for_each(|e| (s.events)(e));
        last = Some(State::default());
        if available { (s.events)(BtEvent::Failed(BtError::NotAvailable { detail: err })); }
        available = false;
        std::thread::sleep(RETRY);
    }
}

/// Sets up with BlueZ (its objects, the pairing agent) and follows its signals, reporting changes
/// and applying `Shared::wanted` whenever an adapter appears, until BlueZ or the bus goes away
/// (the error says why).
fn session(s: &Shared, last: &mut Option<State>, available: &mut bool) -> Result<(), String> {
    let conn = zbus::blocking::connection::Builder::system().and_then(|b| b.method_timeout(CALL_TIMEOUT).build())
        .map_err(|e| format!("system bus: {e}"))?;
    // Every broadcast signal: a rule naming org.bluez as sender would compare it to the
    // signals' sender, BlueZ's unique name, and match nothing. Subscribed before reading the
    // objects, so no change falls in between.
    let signals = MessageIterator::for_match_rule("type='signal'", &conn, Some(256)).map_err(|e| e.to_string())?;
    let owner: String = conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "GetNameOwner", &BLUEZ)
        .and_then(|m| m.body().deserialize()).map_err(|_| "bluetoothd isn't running".to_string())?;
    let all: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        call(&conn, "/", OBJECT_MANAGER, "GetManagedObjects", &())?.body().deserialize().map_err(|e| e.to_string())?;
    let mut objects = Objects::default();
    all.iter().for_each(|(path, ifaces)| objects.added(path, ifaces));

    conn.object_server().at(AGENT_PATH, Agent { pairing: s.pairing.clone() }).map_err(|e| e.to_string())?;
    let agent = OwnedObjectPath::try_from(AGENT_PATH).expect("a valid path");
    call(&conn, "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent", &(&agent, "NoInputNoOutput"))?;
    call(&conn, "/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent", &(&agent,))?;

    let report = |objects: &Objects, last: &mut Option<State>| {
        let now = objects.state();
        changes(last.as_ref(), &now).into_iter().for_each(|e| (s.events)(e));
        *last = Some(now);
    };
    report(&objects, last);
    let adapter = objects.adapter_state();
    *s.link.lock().unwrap() = Some(Link { conn: conn.clone(), objects });
    *available = true;
    apply_due(s, AdapterState::default(), adapter);

    for msg in signals {
        let msg = msg.map_err(|e| e.to_string())?;
        let h = msg.header();
        let (Some(iface), Some(member), Some(path)) = (h.interface(), h.member(), h.path()) else { continue };
        let (iface, member, path) = (iface.as_str(), member.as_str(), path.as_str());
        if (iface, member) == ("org.freedesktop.DBus", "NameOwnerChanged") {
            let (name, _, new): (String, String, String) = msg.body().deserialize().map_err(|e| e.to_string())?;
            if name == BLUEZ && new != owner { return Err("bluetoothd stopped".into()); }
            continue;
        }
        if h.sender().map(|s| s.as_str()) != Some(owner.as_str()) { continue; }
        let (before, after) = {
            let mut guard = s.link.lock().unwrap();
            let Some(link) = guard.as_mut() else { return Err("disconnected".into()) };
            let objects = &mut link.objects;
            let before = objects.adapter_state();
            match (iface, member) {
                (OBJECT_MANAGER, "InterfacesAdded") => {
                    let (path, ifaces): (OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>) = msg.body().deserialize().map_err(|e| e.to_string())?;
                    let kept: Vec<_> = ifaces.iter().filter_map(|(i, raw)| Some((short(i), kept_props(i, raw)?))).collect();
                    if kept.is_empty() { continue; }
                    trace::line("bluetooth", "recv", &format!("InterfacesAdded {} {kept:?}", path.as_str()));
                    objects.added(&path, &ifaces);
                }
                (OBJECT_MANAGER, "InterfacesRemoved") => {
                    let (path, ifaces): (OwnedObjectPath, Vec<String>) = msg.body().deserialize().map_err(|e| e.to_string())?;
                    if !ifaces.iter().any(|i| KEPT.iter().any(|(k, _)| k == i)) { continue; }
                    trace::line("bluetooth", "recv", &format!("InterfacesRemoved {} {ifaces:?}", path.as_str()));
                    objects.removed(&path, &ifaces);
                }
                (PROPERTIES, "PropertiesChanged") => {
                    let (on, raw, invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) = msg.body().deserialize().map_err(|e| e.to_string())?;
                    let Some(props) = kept_props(&on, &raw) else { continue };
                    let invalidated: Vec<String> = invalidated.into_iter().filter(|k| KEPT.iter().any(|(i, n)| *i == on && n.contains(&k.as_str()))).collect();
                    if props.is_empty() && invalidated.is_empty() || objects.get(path, &on).is_none() { continue; }
                    trace::line("bluetooth", "recv", &format!("PropertiesChanged {path} {} {props:?} invalidated {invalidated:?}", short(&on)));
                    objects.changed(path, &on, props, &invalidated);
                }
                _ => continue,
            }
            report(objects, last);
            (before, objects.adapter_state())
        };
        // `apply_due` takes the link's lock, so it runs only once the block above released it.
        apply_due(s, before, after);
    }
    Err("the bus connection closed".into())
}

/// Carries out the wanted commands that became due (see `due`).
fn apply_due(s: &Shared, before: AdapterState, now: AdapterState) {
    let wanted = *s.wanted.lock().unwrap();
    due(before, now, wanted).into_iter().for_each(|cmd| command(s, cmd));
}

/// "Device1" for "org.bluez.Device1", for shorter trace lines.
fn short(iface: &str) -> &str { iface.rsplit('.').next().unwrap_or(iface) }

/// Calls BlueZ's `method` on `path`, tracing the call and its answer.
fn call<B: serde::Serialize + DynamicType + std::fmt::Debug>(conn: &Connection, path: &str, iface: &str, method: &str, body: &B) -> Result<zbus::Message, Failure> {
    trace::line("bluetooth", "send", &format!("{path} {}.{method} {body:?}", short(iface)));
    let r = conn.call_method(Some(BLUEZ), path, Some(iface), method, body).map_err(|e| match e {
        zbus::Error::MethodError(name, text, _) => format!("{name}: {}", text.unwrap_or_default()),
        e => e.to_string(),
    });
    trace::line("bluetooth", "recv", &format!("{path} {method}: {}", r.as_ref().map_or_else(|e| e.as_str(), |_| "ok")));
    r
}

/// Sets property `name` of `iface` on `path`.
fn set(conn: &Connection, path: &str, iface: &str, name: &str, value: bool) -> Result<(), Failure> {
    call(conn, path, PROPERTIES, "Set", &(iface, name, Value::from(value))).map(drop)
}

/// Carries out `cmd`, reporting a failure as an event (the outcome itself arrives as BlueZ's
/// property changes).
fn command(s: &Shared, cmd: BtCmd) {
    match cmd {
        BtCmd::Power(on) => s.wanted.lock().unwrap().power = Some(on),
        BtCmd::Search(on) => s.wanted.lock().unwrap().search = on,
        _ => {}
    }
    let target = match &cmd { BtCmd::Pair(a) | BtCmd::Connect(a) | BtCmd::Disconnect(a) | BtCmd::Forget(a) => Some(a), _ => None };
    let found = s.link.lock().unwrap().as_ref()
        .and_then(|l| Some((l.conn.clone(), l.objects.adapter()?.to_string(), l.objects.adapter_state().powered, target.map(|a| l.objects.device(a)))));
    let Some((conn, adapter, powered, device)) = found else {
        // Applied once BlueZ and an adapter are there (see `due`).
        if matches!(cmd, BtCmd::Power(_) | BtCmd::Search(_)) { return; }
        return (s.events)(BtEvent::Failed(BtError::NotAvailable { detail: "BlueZ or its adapter isn't there".into() }));
    };
    // Applied once the adapter is powered (see `due`).
    if cmd == BtCmd::Search(true) && !powered { return; }
    let (path, name) = match device {
        Some(None) => {
            let a = &target.unwrap().0;
            return (s.events)(BtEvent::Failed(BtError::Service { step: "find device".into(), detail: format!("BlueZ doesn't know {a}") }));
        }
        Some(Some(d)) => d,
        None => Default::default(),
    };
    let result = match cmd {
        BtCmd::Power(on) => tolerate(set(&conn, &adapter, ADAPTER, "Powered", on), POWER_CHANGING).map_err(|e| ("power", e)),
        BtCmd::Search(on) => search(&conn, &adapter, on).map_err(|e| ("search", e)),
        BtCmd::Pair(_) => pair(s, &conn, &path),
        BtCmd::Connect(_) => connect(&conn, &path),
        BtCmd::Disconnect(_) => call(&conn, &path, DEVICE, "Disconnect", &()).map(drop).map_err(|e| ("disconnect", e)),
        BtCmd::Forget(_) => {
            let device = OwnedObjectPath::try_from(path.as_str()).expect("BlueZ's own path");
            call(&conn, &adapter, ADAPTER, "RemoveDevice", &(&device,)).map(drop).map_err(|e| ("forget", e))
        }
    };
    if let Err((step, detail)) = result { (s.events)(BtEvent::Failed(bt_error(step, &name, detail))); }
}

/// Starts or stops looking for devices. Only classic Bluetooth (BR/EDR): headphones, speakers
/// and most controllers and keyboards use it, and leaving out Bluetooth Low Energy keeps out
/// the many beacons and appliances nearby.
fn search(conn: &Connection, adapter: &str, on: bool) -> Result<(), Failure> {
    if on {
        let filter = HashMap::from([("Transport", Value::from("bredr"))]);
        call(conn, adapter, ADAPTER, "SetDiscoveryFilter", &(filter,))?;
        tolerate(call(conn, adapter, ADAPTER, "StartDiscovery", &()), &["org.bluez.Error.InProgress"])
    } else {
        // Failed: "No discovery started"; NotReady: powered off, so not searching either.
        tolerate(call(conn, adapter, ADAPTER, "StopDiscovery", &()), &["org.bluez.Error.Failed", "org.bluez.Error.NotReady"])
    }
}

/// Pairs, trusts (so the device may reconnect by itself) and connects.
fn pair(s: &Shared, conn: &Connection, path: &str) -> Result<(), (&'static str, Failure)> {
    s.pairing.lock().unwrap().insert(path.into());
    let paired = tolerate(call(conn, path, DEVICE, "Pair", &()), &["org.bluez.Error.AlreadyExists"]);
    s.pairing.lock().unwrap().remove(path);
    paired.map_err(|e| ("pair", e))?;
    set(conn, path, DEVICE, "Trusted", true).map_err(|e| ("trust", e))?;
    connect(conn, path)
}

fn connect(conn: &Connection, path: &str) -> Result<(), (&'static str, Failure)> {
    tolerate(call(conn, path, DEVICE, "Connect", &()), &["org.bluez.Error.AlreadyConnected"]).map_err(|e| ("connect", e))
}

/// The pairing agent BlueZ asks while pairing. With no screen or keys to compare codes on
/// ("NoInputNoOutput"), it agrees to pairings Specular started and refuses any other, and lets
/// paired devices use their services.
struct Agent { pairing: Arc<Mutex<HashSet<String>>> }

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
}

impl Agent {
    /// Agrees if Specular is pairing with `device`; traces the request either way.
    fn ours(&self, request: &str, device: &OwnedObjectPath) -> Result<(), AgentError> {
        let ours = self.pairing.lock().unwrap().contains(device.as_str());
        trace::line("bluetooth", "recv", &format!("agent {request} {} -> {}", device.as_str(), if ours { "agreed" } else { "refused" }));
        if ours { Ok(()) } else { Err(AgentError::Rejected("not a pairing Specular started".into())) }
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) { trace::line("bluetooth", "recv", "agent Release"); }

    /// For older devices that use a fixed code; most such headsets take "0000".
    fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, AgentError> { self.ours("RequestPinCode", &device).map(|_| "0000".into()) }

    fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, AgentError> { self.ours("RequestPasskey", &device).map(|_| 0) }

    fn display_pin_code(&self, device: OwnedObjectPath, _pincode: String) -> Result<(), AgentError> { self.ours("DisplayPinCode", &device) }

    fn display_passkey(&self, device: OwnedObjectPath, _passkey: u32, _entered: u16) { trace::line("bluetooth", "recv", &format!("agent DisplayPasskey {}", device.as_str())); }

    fn request_confirmation(&self, device: OwnedObjectPath, _passkey: u32) -> Result<(), AgentError> { self.ours("RequestConfirmation", &device) }

    fn request_authorization(&self, device: OwnedObjectPath) -> Result<(), AgentError> { self.ours("RequestAuthorization", &device) }

    /// A paired device connecting one of its services (BlueZ asks only for untrusted ones).
    fn authorize_service(&self, device: OwnedObjectPath, uuid: String) {
        trace::line("bluetooth", "recv", &format!("agent AuthorizeService {} {uuid} -> agreed", device.as_str()));
    }

    fn cancel(&self) { trace::line("bluetooth", "recv", "agent Cancel"); }
}

#[cfg(test)]
mod tests;
