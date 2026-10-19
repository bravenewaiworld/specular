//! WiFi through wpa_supplicant's control interface: text commands and replies over a Unix
//! datagram socket in its control directory (its `ctrl_interface`, e.g. /run/wpa_supplicant/wlan0),
//! and a second socket attached for its unsolicited `<N>CTRL-EVENT-…` messages.
//!
//! One worker thread owns the connection and the state. Commands and wpa_supplicant's messages
//! reach it through one channel, so they are handled in the order they happened. The device
//! supplies what isn't wpa_supplicant's: power, and DHCP (`WifiDevice`); the address is read back
//! from wpa_supplicant's STATUS. The interfaces' addresses are also read every few seconds: the
//! other interfaces' are reported (`WifiEvent::Interfaces`), and when WiFi's own interface no
//! longer has the connected link's address (a renewal gave a new one, or the lease ran out),
//! STATUS is read again.
//!
//! Joining a network selects it (SELECT_NETWORK, which disables the others while it tries) and
//! saves it (SAVE_CONFIG) only once it has connected, so a wrong password is never saved; the
//! others are enabled again either way. Every exchange is traced (`trace::line`, service "wifi",
//! kinds `send` and `recv`) with passwords replaced by `<secret>`.

use std::borrow::Cow;
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use specular_core::wireless::wifi::{Interface, JoinStep, Network, WifiCmd, WifiError, WifiEvent, WifiLink, valid_password};
use specular_core::wireless::{Events, Secret, Wireless, trace};

/// How long wpa_supplicant's control socket may take to appear after powering on.
const SOCKET_WAIT: Duration = Duration::from_secs(10);
/// How long a reply may take.
const REPLY_WAIT: Duration = Duration::from_secs(5);
/// How long joining may take before it is given up.
const JOIN_WAIT: Duration = Duration::from_secs(30);
/// How long DHCP may take before `NoAddress` is reported (the wait for an address goes on).
const ADDRESS_WAIT: Duration = Duration::from_secs(20);
/// How often the address is looked for, while waiting for one.
const ADDRESS_POLL: Duration = Duration::from_secs(1);
/// How often to try reconnecting after wpa_supplicant went away while WiFi is on (it is
/// restarted around sleep, for one).
const RECONNECT_POLL: Duration = Duration::from_secs(1);
/// How often the signal strength is read, while connected.
const SIGNAL_POLL: Duration = Duration::from_secs(10);
/// How often the interfaces' addresses are read, whether WiFi is on or off (short in tests, so
/// they needn't wait).
const INTERFACES_POLL: Duration = Duration::from_millis(if cfg!(test) { 50 } else { 5000 });

/// What a device does around wpa_supplicant.
pub trait WifiDevice: Send {
    /// Turns the WiFi hardware and its services (wpa_supplicant, DHCP) on or off. Turning off
    /// what is already off succeeds. When on, wpa_supplicant's control socket may appear a little
    /// later.
    fn set_power(&mut self, on: bool) -> io::Result<()>;
    /// The device has just joined a network: the time to drop any address from a previous
    /// network and ask for a new one at once.
    fn joined(&mut self) {}
    /// The addresses of the interfaces that are up, WiFi's included: WiFi's is the one named
    /// like the control socket's file (e.g. wlan0).
    fn addresses(&self) -> io::Result<Vec<Interface>> { crate::interfaces::addresses() }
}

/// WiFi through wpa_supplicant. Commands return at once; their work is done on a thread.
pub struct WpaSupplicant { setup: Option<(PathBuf, Box<dyn WifiDevice>)>, tx: Option<Sender<Input>> }

impl WpaSupplicant {
    /// wpa_supplicant for the interface whose control socket is `socket` (its control directory
    /// and the interface's name, e.g. /run/wpa_supplicant/wlan0).
    pub fn new(socket: impl Into<PathBuf>, device: Box<dyn WifiDevice>) -> Self {
        Self { setup: Some((socket.into(), device)), tx: None }
    }
}

impl Wireless<WifiCmd, WifiEvent> for WpaSupplicant {
    fn start(&mut self, events: Events<WifiEvent>) {
        let Some((socket, device)) = self.setup.take() else { return };
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            socket, device, events, tx: tx.clone(), conn: None, powered: false, link: WifiLink::Off,
            saved: vec![], seen: vec![], join: None, address: None, lost_at: None, signal_due: None,
            interfaces: vec![], interfaces_due: Instant::now(),
        };
        self.tx = Some(tx);
        std::thread::Builder::new().name("wifi".into()).spawn(move || worker.run(rx)).expect("spawning the WiFi thread");
    }

    fn send(&mut self, cmd: WifiCmd) {
        if let Some(tx) = &self.tx { let _ = tx.send(Input::Cmd(cmd)); }
    }
}

/// What the worker handles.
enum Input {
    Cmd(WifiCmd),
    /// An unsolicited message from wpa_supplicant, as it came.
    Message(String),
    /// The attached socket failed: wpa_supplicant went away.
    Lost,
}

// --- The connection --------------------------------------------------------------------------

/// The two sockets to wpa_supplicant: requests, and its messages (read on a thread of its own,
/// which ends when this is dropped).
struct Conn { ctrl: UnixDatagram, monitor: UnixDatagram, closed: Arc<AtomicBool> }

/// A socket of ours, connected to wpa_supplicant's at `server`. Ours needs an address for the
/// replies: an abstract one (no file, so nothing is left behind).
fn connect(server: &Path) -> io::Result<UnixDatagram> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let name = format!("specular-wpa-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
    let s = UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(name)?)?;
    s.connect(server)?;
    s.set_read_timeout(Some(REPLY_WAIT))?;
    Ok(s)
}

impl Conn {
    /// Connects and attaches, forwarding wpa_supplicant's messages to `tx`.
    fn open(server: &Path, tx: Sender<Input>) -> io::Result<Conn> {
        let conn = Conn { ctrl: connect(server)?, monitor: connect(server)?, closed: Arc::new(AtomicBool::new(false)) };
        let reply = exchange(&conn.monitor, "ATTACH")?;
        if reply.trim() != "OK" { return Err(io::Error::other(format!("ATTACH: {}", reply.trim()))); }
        let (monitor, closed) = (conn.monitor.try_clone()?, conn.closed.clone());
        monitor.set_read_timeout(Some(Duration::from_secs(1)))?;
        std::thread::Builder::new().name("wifi-events".into()).spawn(move || {
            let mut buf = vec![0; 4096];
            while !closed.load(Ordering::Relaxed) {
                match monitor.recv(&mut buf) {
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).trim_end().to_string();
                        if closed.load(Ordering::Relaxed) { break; }
                        trace::line("wifi", "recv", &text);
                        if tx.send(Input::Message(text)).is_err() { break; }
                    }
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                    Err(e) => {
                        if !closed.load(Ordering::Relaxed) { trace::line("wifi", "lost", &e.to_string()); let _ = tx.send(Input::Lost); }
                        break;
                    }
                }
            }
        })?;
        Ok(conn)
    }

    /// Sends a command and returns the reply, skipping any late reply to an earlier one.
    fn request(&self, cmd: &str) -> io::Result<String> {
        self.ctrl.set_nonblocking(true)?;
        let mut buf = [0; 1];
        while self.ctrl.recv(&mut buf).is_ok() {}
        self.ctrl.set_nonblocking(false)?;
        exchange(&self.ctrl, cmd)
    }
}

/// One command and its reply on `s`.
fn exchange(s: &UnixDatagram, cmd: &str) -> io::Result<String> {
    trace::line("wifi", "send", &redact(cmd));
    s.send(cmd.as_bytes())?;
    // SCAN_RESULTS is the longest reply; wpa_supplicant cuts every reply at 4 KiB.
    let mut buf = vec![0; 16 * 1024];
    let n = s.recv(&mut buf)?;
    let reply = String::from_utf8_lossy(&buf[..n]).into_owned();
    trace::line("wifi", "recv", reply.trim_end());
    Ok(reply)
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self.monitor.send(b"DETACH");
    }
}

// --- The worker ------------------------------------------------------------------------------

/// A join under way.
struct Join {
    ssid: String,
    /// The network's id in wpa_supplicant.
    id: u32,
    /// Added for this join (removed again if it fails), rather than saved before.
    added: bool,
    /// A scan has started since, so "not found" means out of range.
    scanned: bool,
    since: Instant,
}

/// Waiting for an address (DHCP), after joining `ssid`.
struct AddressWait { ssid: String, since: Instant, next: Instant, reported: bool }

struct Worker {
    socket: PathBuf,
    device: Box<dyn WifiDevice>,
    events: Events<WifiEvent>,
    tx: Sender<Input>,
    conn: Option<Conn>,
    /// Asked to be on (the connection may be missing for a while, as around sleep).
    powered: bool,
    /// As last reported.
    link: WifiLink,
    saved: Vec<Saved>,
    /// The last scan's networks.
    seen: Vec<Seen>,
    join: Option<Join>,
    address: Option<AddressWait>,
    /// When the connection was lost (or last tried again), while on.
    lost_at: Option<Instant>,
    /// When the signal strength is next read; set only while connected.
    signal_due: Option<Instant>,
    /// The other interfaces' addresses, as last reported.
    interfaces: Vec<Interface>,
    /// When the interfaces' addresses are next read; always set.
    interfaces_due: Instant,
}

impl Worker {
    fn run(mut self, rx: Receiver<Input>) {
        if self.socket.exists() && self.reconnect() {
            self.powered = true;
            self.report_all();
        } else {
            self.report(WifiEvent::Link(WifiLink::Off));
        }
        loop {
            let input = match self.deadline() {
                Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match input {
                Ok(Input::Cmd(cmd)) => if let Err(e) = self.command(cmd) { self.report(WifiEvent::Failed(e)) },
                Ok(Input::Message(m)) => self.message(&m),
                Ok(Input::Lost) => self.lost(),
                Err(RecvTimeoutError::Timeout) => self.tick(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn report(&self, e: WifiEvent) { (self.events)(e) }

    fn set_link(&mut self, link: WifiLink) {
        if link != self.link { self.link = link.clone(); self.report(WifiEvent::Link(link)); }
        self.schedule_signal();
    }

    /// Starts reading the signal as soon as the link is connected (an existing deadline is kept);
/// stops when it isn't.
    fn schedule_signal(&mut self) {
        let connected = matches!(self.link, WifiLink::Connected { .. });
        self.signal_due = if connected { self.signal_due.or(Some(Instant::now())) } else { None };
    }

    /// Reads the signal strength of the connected link and reports it if it changed. A failed or
    /// unreadable reading (a failed request is traced) keeps the last one.
    fn poll_signal(&mut self) {
        let WifiLink::Connected { ssid, address, bars } = self.link.clone() else { return };
        self.signal_due = Some(Instant::now() + SIGNAL_POLL);
        let Ok(reply) = self.request("signal", "SIGNAL_POLL") else { return };
        let now = field(&reply, "RSSI").and_then(|r| r.parse().ok()).map(bars_for);
        if now.is_some() && now != bars { self.set_link(WifiLink::Connected { ssid, address, bars: now }); }
    }

    fn report_networks(&self) { self.report(WifiEvent::Networks(networks(&self.seen, &self.saved))) }

    /// The next time `tick` has something to do.
    fn deadline(&self) -> Option<Instant> {
        let join = self.join.as_ref().map(|j| j.since + JOIN_WAIT);
        let address = self.address.as_ref().map(|a| a.next);
        let reconnect = self.lost_at.map(|t| t + RECONNECT_POLL);
        [join, address, reconnect, self.signal_due, Some(self.interfaces_due)].into_iter().flatten().min()
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.lost_at.is_some_and(|t| now >= t + RECONNECT_POLL) {
            self.lost_at = Some(now);
            if self.socket.exists() && self.reconnect() { self.report_all(); }
        }
        if self.join.as_ref().is_some_and(|j| now >= j.since + JOIN_WAIT) {
            self.fail_join(WifiError::Service { step: "join".into(), detail: format!("not connected after {} s", JOIN_WAIT.as_secs()) });
        }
        if self.address.as_ref().is_some_and(|a| now >= a.next) { self.check_address(); }
        if self.signal_due.is_some_and(|t| now >= t) { self.poll_signal(); }
        if now >= self.interfaces_due { self.poll_interfaces(); }
    }

    /// Reads the interfaces' addresses: reports the others' if they changed, and reads STATUS
    /// again if WiFi's own no longer has the connected link's address. A failed reading is traced
    /// and changes nothing.
    fn poll_interfaces(&mut self) {
        self.interfaces_due = Instant::now() + INTERFACES_POLL;
        let all = match self.device.addresses() {
            Ok(all) => all,
            Err(e) => return trace::line("wifi", "interfaces", &format!("unreadable: {e}")),
        };
        let own = self.socket.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (wifi, others): (Vec<Interface>, Vec<Interface>) = all.into_iter().partition(|i| i.name == own);
        if others != self.interfaces { self.interfaces = others.clone(); self.report(WifiEvent::Interfaces(others)); }
        let lost = matches!(&self.link, WifiLink::Connected { address, .. } if !wifi.iter().any(|i| i.address == *address));
        if lost { self.read_status(); }
    }

    /// Sets the link from a fresh STATUS, keeping the last signal reading (STATUS has none);
    /// connected without an address, waits for one.
    fn read_status(&mut self) {
        let Ok(status) = self.request("status", "STATUS") else { return };
        let mut link = status_link(&status);
        if let WifiLink::Connecting { ssid, step: JoinStep::GettingAddress } = &link { self.wait_for_address(ssid.clone()); }
        if let (WifiLink::Connected { bars, .. }, WifiLink::Connected { bars: was, .. }) = (&mut link, &self.link) { *bars = *was; }
        self.set_link(link);
    }

    /// Opens the connection; false (and traced) if it can't.
    fn reconnect(&mut self) -> bool {
        self.conn = None;
        match Conn::open(&self.socket, self.tx.clone()) {
            Ok(c) => { self.conn = Some(c); self.lost_at = None; true }
            Err(e) => { trace::line("wifi", "unreachable", &format!("{}: {e}", self.socket.display())); false }
        }
    }

    /// wpa_supplicant went away while on (restarted around sleep, or crashed): reports Idle and
    /// keeps trying to reconnect.
    fn lost(&mut self) {
        if !self.powered { return; }
        self.conn = None;
        self.join = None;
        self.address = None;
        self.lost_at = Some(Instant::now());
        self.set_link(WifiLink::Idle);
    }

    /// Sends `cmd`; an error if the connection fails (then lost) or wpa_supplicant says FAIL.
    fn request(&mut self, step: &str, cmd: &str) -> Result<String, WifiError> {
        let Some(conn) = &self.conn else { return Err(WifiError::NotAvailable { detail: "Wi-Fi is off".into() }) };
        match conn.request(cmd) {
            Ok(r) if r.starts_with("FAIL") || r.starts_with("UNKNOWN COMMAND") => {
                Err(WifiError::Service { step: step.into(), detail: format!("{}: {}", redact(cmd), r.trim()) })
            }
            Ok(r) => Ok(r),
            Err(e) => {
                self.lost();
                Err(WifiError::Service { step: step.into(), detail: format!("{}: {e}", redact(cmd)) })
            }
        }
    }

    /// Reads the saved networks.
    fn read_saved(&mut self) -> Result<(), WifiError> {
        let list = self.request("list", "LIST_NETWORKS")?;
        let mut saved = vec![];
        for (id, ssid) in parse_saved(&list) {
            let open = self.request("list", &format!("GET_NETWORK {id} key_mgmt"))?.trim() == "NONE";
            saved.push(Saved { id, ssid, open });
        }
        self.saved = saved;
        Ok(())
    }

    /// Reports everything: the link, and the networks known (after power on or reconnecting);
    /// then scans, for the networks in range.
    fn report_all(&mut self) {
        let result = (|| {
            self.read_saved()?;
            self.seen = parse_scan_results(&self.request("scan", "SCAN_RESULTS")?);
            let status = self.request("status", "STATUS")?;
            Ok::<_, WifiError>(status_link(&status))
        })();
        match result {
            Ok(link) => {
                if let WifiLink::Connecting { ssid, step: JoinStep::GettingAddress } = &link { self.wait_for_address(ssid.clone()); }
                self.link = link.clone();
                // The link has no signal reading yet: read it now rather than at the next poll.
                self.signal_due = None;
                self.schedule_signal();
                self.report(WifiEvent::Link(link));
                self.report_networks();
                // wpa_supplicant scans by itself only while it has a saved network to look for.
                let _ = self.command(WifiCmd::Scan);
            }
            Err(e) => self.report(WifiEvent::Failed(e)),
        }
    }

    fn command(&mut self, cmd: WifiCmd) -> Result<(), WifiError> {
        match cmd {
            WifiCmd::Power(on) => self.power(on),
            WifiCmd::Scan => match self.request("scan", "SCAN") {
                // Already scanning: its results will do.
                Err(WifiError::Service { detail, .. }) if detail.contains("FAIL-BUSY") => Ok(()),
                r => r.map(drop),
            },
            WifiCmd::Join { ssid, password } => self.start_join(ssid, password),
            WifiCmd::Forget { ssid } => self.forget(&ssid),
        }
    }

    fn power(&mut self, on: bool) -> Result<(), WifiError> {
        if on && self.conn.is_some() { self.report_all(); return Ok(()); }
        if on {
            self.powered = true;
            let started = self.device.set_power(true).map_err(|e| format!("power on: {e}")).and_then(|()| self.wait_for_socket());
            if let Err(detail) = started {
                self.powered = false;
                let _ = self.device.set_power(false);
                self.set_link(WifiLink::Off);
                return Err(WifiError::NotAvailable { detail });
            }
            self.report_all();
            return Ok(());
        }
        (self.powered, self.conn, self.join, self.address, self.lost_at) = (false, None, None, None, None);
        (self.seen, self.saved) = (vec![], vec![]);
        let off = self.device.set_power(false);
        self.set_link(WifiLink::Off);
        self.report_networks();
        off.map_err(|e| WifiError::Service { step: "power off".into(), detail: e.to_string() })
    }

    /// Waits for wpa_supplicant's socket and connects to it.
    fn wait_for_socket(&mut self) -> Result<(), String> {
        let until = Instant::now() + SOCKET_WAIT;
        while Instant::now() < until {
            if self.socket.exists() && self.reconnect() { return Ok(()); }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(format!("no wpa_supplicant at {} after {} s", self.socket.display(), SOCKET_WAIT.as_secs()))
    }

    fn start_join(&mut self, ssid: String, password: Option<Secret>) -> Result<(), WifiError> {
        if self.conn.is_none() { return Err(WifiError::NotAvailable { detail: "Wi-Fi is off".into() }); }
        if let Some(s) = self.seen.iter().find(|s| s.ssid == ssid && !s.joinable) {
            let detail = format!("{}: only open and WPA/WPA2 password networks can be joined", s.flags);
            return Err(WifiError::Unsupported { ssid, detail });
        }
        if password.as_ref().is_some_and(|p| !valid_password(p.expose())) { return Err(WifiError::BadPassword { ssid }); }
        if self.join.is_some() { self.fail_join(WifiError::Service { step: "join".into(), detail: format!("replaced by joining {ssid}") }); }
        self.read_saved()?;
        let saved = self.saved.iter().find(|s| s.ssid == ssid).map(|s| s.id);
        let (id, added) = match (&password, saved) {
            (None, Some(id)) => (id, false),
            _ => (self.add_network(&ssid, password.as_ref())?, true),
        };
        self.join = Some(Join { ssid: ssid.clone(), id, added, scanned: false, since: Instant::now() });
        self.address = None;
        self.set_link(WifiLink::Connecting { ssid, step: JoinStep::Associating });
        if let Err(e) = self.request("join", &format!("SELECT_NETWORK {id}")) { self.fail_join(e); }
        Ok(())
    }

    /// Adds a network (not saved yet), returning its id.
    fn add_network(&mut self, ssid: &str, password: Option<&Secret>) -> Result<u32, WifiError> {
        let reply = self.request("join", "ADD_NETWORK")?;
        let id: u32 = reply.trim().parse().map_err(|_| WifiError::Service { step: "join".into(), detail: format!("ADD_NETWORK: {}", reply.trim()) })?;
        let set = network_settings(ssid, password).into_iter().try_for_each(|(name, value)| {
            self.request("join", &format!("SET_NETWORK {id} {name} {value}")).map(drop)
        });
        if let Err(e) = set { let _ = self.request("join", &format!("REMOVE_NETWORK {id}")); return Err(e); }
        Ok(id)
    }

    /// The join under way connected: saves it (replacing older entries for the same name).
    fn save_join(&mut self) -> Result<(), WifiError> {
        let Some(join) = self.join.take() else { return Ok(()) };
        self.request("join", "ENABLE_NETWORK all")?;
        self.read_saved()?;
        let older: Vec<u32> = self.saved.iter().filter(|s| s.ssid == join.ssid && s.id != join.id).map(|s| s.id).collect();
        for id in older { self.request("save", &format!("REMOVE_NETWORK {id}"))?; }
        self.request("save", "SAVE_CONFIG")?;
        self.read_saved()?;
        self.report_networks();
        Ok(())
    }

    /// Gives up the join under way, removing the network if it was added for it.
    fn fail_join(&mut self, e: WifiError) {
        let Some(join) = self.join.take() else { return };
        if join.added { let _ = self.request("join", &format!("REMOVE_NETWORK {}", join.id)); }
        let _ = self.request("join", "ENABLE_NETWORK all");
        if !matches!(self.link, WifiLink::Connected { .. }) { self.set_link(WifiLink::Idle); }
        self.report(WifiEvent::Failed(e));
    }

    fn forget(&mut self, ssid: &str) -> Result<(), WifiError> {
        if self.join.as_ref().is_some_and(|j| j.ssid == ssid) { self.join = None; }
        self.read_saved()?;
        let ids: Vec<u32> = self.saved.iter().filter(|s| s.ssid == ssid).map(|s| s.id).collect();
        for id in &ids { self.request("forget", &format!("REMOVE_NETWORK {id}"))?; }
        if !ids.is_empty() { self.request("save", "SAVE_CONFIG")?; }
        self.read_saved()?;
        self.report_networks();
        Ok(())
    }

    fn wait_for_address(&mut self, ssid: String) {
        let now = Instant::now();
        self.address = Some(AddressWait { ssid, since: now, next: now + ADDRESS_POLL, reported: false });
    }

    /// Looks for the address while waiting for one: Connected once there is one, NoAddress
    /// (once) if it takes too long.
    fn check_address(&mut self) {
        let Ok(status) = self.request("status", "STATUS") else { return };
        let Some(wait) = &mut self.address else { return };
        // Leaving the network ends the wait too, through its message.
        let link = status_link(&status);
        if matches!(link, WifiLink::Connected { .. }) {
            self.address = None;
            self.set_link(link);
            return;
        }
        wait.next = Instant::now() + ADDRESS_POLL;
        let late = !wait.reported && wait.since.elapsed() >= ADDRESS_WAIT;
        wait.reported |= late;
        let ssid = wait.ssid.clone();
        if late { self.report(WifiEvent::Failed(WifiError::NoAddress { ssid })); }
    }

    /// The ssid the link is about: the join's, else the link's, else (rarely) STATUS's.
    fn current_ssid(&mut self, said: Option<String>) -> Option<String> {
        let link = match &self.link { WifiLink::Connecting { ssid, .. } | WifiLink::Connected { ssid, .. } => Some(ssid.clone()), _ => None };
        let known = self.join.as_ref().map(|j| j.ssid.clone()).or(said).or(link);
        known.or_else(|| self.request("status", "STATUS").ok().and_then(|s| field(&s, "ssid").map(decode_ssid)))
    }

    fn message(&mut self, text: &str) {
        // Left over from a connection since closed.
        if self.conn.is_none() { return; }
        match parse_message(text) {
            Message::Trying { ssid } => {
                if let Some(ssid) = self.current_ssid(ssid) { self.set_link(WifiLink::Connecting { ssid, step: JoinStep::Associating }); }
            }
            Message::Associated => {
                if let Some(ssid) = self.current_ssid(None) { self.set_link(WifiLink::Connecting { ssid, step: JoinStep::Authenticating }); }
            }
            Message::Connected => {
                let ssid = self.current_ssid(None);
                self.device.joined();
                if let Err(e) = self.save_join() { self.report(WifiEvent::Failed(e)); }
                if let Some(ssid) = ssid {
                    self.set_link(WifiLink::Connecting { ssid: ssid.clone(), step: JoinStep::GettingAddress });
                    self.wait_for_address(ssid);
                    self.check_address();
                }
            }
            Message::Disconnected => {
                self.address = None;
                // A join may first leave the current network.
                if self.join.is_none() { self.set_link(WifiLink::Idle); }
            }
            Message::TempDisabled { id, reason } => {
                let Some(join) = self.join.as_ref().filter(|j| j.id == id) else { return };
                let ssid = join.ssid.clone();
                self.fail_join(match reason.as_str() {
                    "WRONG_KEY" => WifiError::WrongPassword { ssid },
                    _ => WifiError::Service { step: "join".into(), detail: format!("{ssid}: {reason}") },
                });
            }
            Message::NotFound => {
                if let Some(join) = self.join.as_ref().filter(|j| j.scanned) {
                    let ssid = join.ssid.clone();
                    self.fail_join(WifiError::OutOfRange { ssid });
                }
            }
            Message::ScanStarted => if let Some(j) = &mut self.join { j.scanned = true },
            Message::ScanResults => match self.request("scan", "SCAN_RESULTS") {
                Ok(r) => { self.seen = parse_scan_results(&r); self.report_networks(); }
                Err(e) => self.report(WifiEvent::Failed(e)),
            },
            Message::Terminating => self.lost(),
            Message::Other => {}
        }
    }
}

// --- The protocol's text ---------------------------------------------------------------------

/// A saved network.
#[derive(Clone, Debug, PartialEq)]
struct Saved { id: u32, ssid: String, open: bool }

/// A network seen by a scan: the strongest of its access points.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    ssid: String,
    dbm: i32,
    secured: bool,
    /// Open, or secured by a WPA/WPA2 password (not WPA3-only, enterprise or WEP).
    joinable: bool,
    /// As wpa_supplicant gave them, e.g. "[WPA2-PSK-CCMP][ESS]".
    flags: String,
}

/// The `name value` settings that make a network for `ssid`. The name is given in hex so any
/// bytes are safe; the password quoted, or bare when it is a 64-digit raw key.
fn network_settings(ssid: &str, password: Option<&Secret>) -> Vec<(&'static str, String)> {
    let hex: String = ssid.bytes().map(|b| format!("{b:02x}")).collect();
    let key = match password.map(Secret::expose) {
        None => ("key_mgmt", "NONE".to_string()),
        Some(p) if p.len() == 64 && p.chars().all(|c| c.is_ascii_hexdigit()) => ("psk", p.to_string()),
        Some(p) => ("psk", format!("\"{p}\"")),
    };
    vec![("ssid", hex), key]
}

/// `cmd` as it may be traced: secrets replaced by `<secret>`.
fn redact(cmd: &str) -> Cow<'_, str> {
    let mut words = cmd.splitn(4, ' ');
    match (words.next(), words.next(), words.next()) {
        (Some("SET_NETWORK"), Some(id), Some(name)) if matches!(name, "psk" | "password" | "sae_password") || name.starts_with("wep_key") => {
            format!("SET_NETWORK {id} {name} <secret>").into()
        }
        _ => cmd.into(),
    }
}

/// An SSID as wpa_supplicant writes it (C-style escapes: `\xNN`, `\\`, `\"`, `\e`, `\n`, `\r`,
/// `\t`), as its bytes.
fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let (byte, len) = match (b[i], b.get(i + 1)) {
            (b'\\', Some(b'x')) => match s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => (v, 4),
                None => (b'\\', 1),
            },
            (b'\\', Some(b'e')) => (0x1b, 2),
            (b'\\', Some(b'n')) => (b'\n', 2),
            (b'\\', Some(b'r')) => (b'\r', 2),
            (b'\\', Some(b't')) => (b'\t', 2),
            (b'\\', Some(&c @ (b'\\' | b'"'))) => (c, 2),
            (c, _) => (c, 1),
        };
        out.push(byte);
        i += len;
    }
    out
}

/// An escaped SSID as text (bytes that aren't UTF-8 become U+FFFD).
fn decode_ssid(s: &str) -> String { String::from_utf8_lossy(&unescape(s)).into_owned() }

/// A line's value in a `key=value` reply (STATUS).
fn field<'a>(reply: &'a str, key: &str) -> Option<&'a str> {
    reply.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

/// SCAN_RESULTS: the networks seen, one per name (the strongest), strongest first. Hidden
/// networks (no name, or one of zero bytes) are left out, as is a line cut short by
/// wpa_supplicant's 4 KiB limit on replies.
fn parse_scan_results(reply: &str) -> Vec<Seen> {
    let mut seen: Vec<Seen> = vec![];
    for line in reply.lines().skip(1) {
        let [_bssid, _freq, signal, flags, ssid] = line.splitn(5, '\t').collect::<Vec<_>>()[..] else { continue };
        let (Ok(dbm), bytes) = (signal.parse::<i32>(), unescape(ssid)) else { continue };
        if bytes.iter().all(|&b| b == 0) { continue; }
        let secured = ["WPA", "RSN", "WEP", "SAE", "EAP"].iter().any(|k| flags.contains(k));
        let s = Seen { ssid: String::from_utf8_lossy(&bytes).into_owned(), dbm, secured, joinable: !secured || flags.contains("PSK"), flags: flags.into() };
        match seen.iter_mut().find(|o| o.ssid == s.ssid) {
            Some(o) => { let joinable = o.joinable || s.joinable; if s.dbm > o.dbm { *o = s; } o.joinable = joinable; }
            None => seen.push(s),
        }
    }
    seen.sort_by_key(|s| std::cmp::Reverse(s.dbm));
    seen
}

/// LIST_NETWORKS: each saved network's id and name.
fn parse_saved(reply: &str) -> Vec<(u32, String)> {
    reply.lines().skip(1).filter_map(|l| {
        let mut f = l.split('\t');
        Some((f.next()?.parse().ok()?, decode_ssid(f.next()?)))
    }).collect()
}

/// Signal strength in bars, 1–4, for a network in range, from its level in dBm.
fn bars_for(dbm: i32) -> u8 {
    match dbm { -55.. => 4, -65.. => 3, -75.. => 2, _ => 1 }
}

/// Everything known, as the app wants it: those in range (strongest first), then saved ones out
/// of range.
fn networks(seen: &[Seen], saved: &[Saved]) -> Vec<Network> {
    let in_range = seen.iter().map(|s| Network {
        ssid: s.ssid.clone(), bars: bars_for(s.dbm), secured: s.secured, saved: saved.iter().any(|v| v.ssid == s.ssid), in_range: true,
        supported: s.joinable,
    });
    let mut out: Vec<Network> = in_range.collect();
    for s in saved {
        if out.iter().any(|n| n.ssid == s.ssid) { continue; }
        out.push(Network { ssid: s.ssid.clone(), bars: 0, secured: !s.open, saved: true, in_range: false, supported: true });
    }
    out
}

/// The link as STATUS shows it. Connecting without a name (STATUS gives it only once
/// associated) shows as Idle.
fn status_link(reply: &str) -> WifiLink {
    let ssid = field(reply, "ssid").map(decode_ssid);
    let step = match field(reply, "wpa_state") {
        Some("AUTHENTICATING" | "ASSOCIATING") => JoinStep::Associating,
        Some("ASSOCIATED" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE") => JoinStep::Authenticating,
        Some("COMPLETED") => match (field(reply, "ip_address").filter(|a| *a != "0.0.0.0"), &ssid) {
            (Some(address), Some(ssid)) => return WifiLink::Connected { ssid: ssid.clone(), address: address.into(), bars: None },
            _ => JoinStep::GettingAddress,
        },
        _ => return WifiLink::Idle,
    };
    ssid.map_or(WifiLink::Idle, |ssid| WifiLink::Connecting { ssid, step })
}

/// The messages that matter, from wpa_supplicant's unsolicited ones.
#[derive(Debug, PartialEq)]
enum Message {
    /// Trying a network (authenticating or associating), named if the message says.
    Trying { ssid: Option<String> },
    Associated,
    /// The network accepted the device (the password was right).
    Connected,
    Disconnected,
    /// The network with this id is held off for a while after failing, for `reason` (WRONG_KEY,
    /// AUTH_FAILED, CONN_FAILED...).
    TempDisabled { id: u32, reason: String },
    /// A scan found no enabled network.
    NotFound,
    ScanStarted,
    ScanResults,
    /// wpa_supplicant is exiting.
    Terminating,
    Other,
}

/// What an unsolicited message (`<level>text`) says.
fn parse_message(text: &str) -> Message {
    let text = text.strip_prefix('<').and_then(|t| t.split_once('>')).map_or(text, |(_, t)| t);
    let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
    let value = |key: &str| rest.split(' ').find_map(|w| w.strip_prefix(key)?.strip_prefix('='));
    match name {
        "CTRL-EVENT-CONNECTED" => Message::Connected,
        "CTRL-EVENT-DISCONNECTED" => Message::Disconnected,
        "CTRL-EVENT-SSID-TEMP-DISABLED" => match value("id").and_then(|i| i.parse().ok()) {
            Some(id) => Message::TempDisabled { id, reason: value("reason").unwrap_or("").into() },
            None => Message::Other,
        },
        "CTRL-EVENT-NETWORK-NOT-FOUND" => Message::NotFound,
        "CTRL-EVENT-SCAN-STARTED" => Message::ScanStarted,
        "CTRL-EVENT-SCAN-RESULTS" => Message::ScanResults,
        "CTRL-EVENT-TERMINATING" => Message::Terminating,
        _ if text.starts_with("Trying to associate with ") || text.starts_with("SME: Trying to authenticate with ") => {
            // "... (SSID='name' freq=2432 MHz)"; the name isn't escaped for quotes, so up to the
            // last "' freq=".
            let ssid = text.split_once("SSID='").and_then(|(_, s)| s.rsplit_once("' freq=")).map(|(s, _)| decode_ssid(s));
            Message::Trying { ssid }
        }
        _ if text.starts_with("Associated with ") => Message::Associated,
        _ => Message::Other,
    }
}

#[cfg(test)]
mod tests;
