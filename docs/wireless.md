# Wireless

Status: Implemented · Updated: 2026-10-18

WiFi and Bluetooth: the services that drive the radios, how the app keeps them as the settings and screens want, Bluetooth audio, and the trace and scenarios that let a radio problem seen on a device be replayed on a PC. Radios are slow, flaky and stateful, so their behaviour has to be observable and reproducible.

## Decisions

- **Radios go through the standard Linux services: wpa_supplicant and BlueZ** (2026-10-14), in `specular-linux`, shared by any Linux device. A port supplies only what isn't theirs: powering the WiFi hardware and asking for an address (DHCP).
- **WiFi is On, Off or Auto, and Auto is the default** (2026-10-14). Auto turns WiFi on only while something needs it (box art downloads, or the Wi-Fi screen being open), which saves battery. A developer setting, "Keep Wi-Fi on", stops Auto from turning it off, so SSH over WiFi stays up.
- **A network is saved only once it has connected** (2026-10-14), so a wrong password is never kept.
- **Failures are shown only on the radio's own screen** (2026-10-14), where the user asked for something. Elsewhere they are only in the trace.
- **Every exchange with a radio is traced, always** (2026-10-14). It is a few lines per user action, so it costs the same everywhere and there is no separate mode in which behaviour could differ. Passwords are never written.
- **Bluetooth headphones and speakers are a sound output** (2026-10-14), through bluez-alsa, with one volume they all share. A game's sound can switch to them mid-game, and falls back to the device's own output (headphones in the jack, or the speaker) if they fail.
- **The Wi-Fi screen shows the device's address while connected; with Developer details, also its other interfaces' addresses** (2026-10-18), so files can be sent to it, and a developer can find it over the USB link even with WiFi off. The other interfaces come from the WiFi service, as an event, so they are traced and can be scripted like the rest. Only IPv4 addresses are shown. Rejected: a network service of its own (a third beside WiFi and Bluetooth, with its own trace and scenario, for one list); IPv6 addresses (long, and a link-local one can't be typed without its interface).
- **Addresses are kept current by reading the interfaces every 5 s** (2026-10-18), with getifaddrs. The other interfaces are reported when they change; when WiFi's interface no longer has the link's address (a renewal gave a new one, or the lease ran out), wpa_supplicant's status is read again. Rejected: netlink address notifications (more code than a poll, for a screen that can wait 5 s); reading the status with every signal reading (twice the trace lines while connected, all the time, for a change that is rare).
- **On the keyboard, Select shows or hides masked text, and a hint says which** (2026-10-18). The text is hidden each time the keyboard opens. Rejected: a key among the keys (one more to walk past; Y and X are already Shift and Symbols); remembering it shown (a password on screen by surprise).

## Design

### Services

Each radio is a service: commands in, events out. Commands may block (loading a driver takes seconds), so each radio has a thread of its own in the UI crate. Failures arrive as events rather than return values, so the app sees one stream of what happened.

```rust
pub trait Wireless<C, E>: Send {
    /// Once, before any command. The service then reports its whole state unasked.
    fn start(&mut self, events: Events<E>);
    fn send(&mut self, cmd: C);
}
pub type Wifi = dyn Wireless<WifiCmd, WifiEvent>;
pub type Bluetooth = dyn Wireless<BtCmd, BtEvent>;

pub enum WifiCmd { Power(bool), Scan, Join { ssid: String, password: Option<Secret> }, Forget { ssid: String } }
pub enum WifiEvent { Link(WifiLink), Networks(Vec<Network>), Interfaces(Vec<Interface>), Failed(WifiError) }
/// One IPv4 address of a network interface.
pub struct Interface { pub name: String, pub address: String }
pub enum WifiLink { Off, Idle, Connecting { ssid: String, step: JoinStep }, Connected { ssid: String, address: String, bars: Option<u8> } }

pub enum BtCmd { Power(bool), Search(bool), Pair(BtAddress), Connect(BtAddress), Disconnect(BtAddress), Forget(BtAddress) }
pub enum BtEvent { Powered(bool), Searching(bool), Devices(Vec<BtDevice>), Failed(BtError) }

/// Text that must never be logged. Debug and serialising both give "<secret>".
pub struct Secret(String);
```

Each error variant keeps the service's own words in a `detail` for the trace, and has a sentence for the user (`WifiError::message`, `BtError::message`).

**wpa_supplicant.** Text commands over its control socket, plus a second, attached socket for its unsolicited `CTRL-EVENT-…` messages. One worker thread owns the connection; commands and messages reach it through one channel, so they are handled in the order they happened. Joining selects the network (which disables the others while it tries) and saves it only once connected; the others are enabled again either way. The address is read back from wpa_supplicant's status. The worker also reads the interfaces' addresses, on or off: the others' are reported when they change, and when WiFi's own interface no longer has the connected link's address, the status is read again.

```rust
/// What a device does around wpa_supplicant.
pub trait WifiDevice: Send {
    /// The hardware and its services (wpa_supplicant, DHCP) on or off.
    fn set_power(&mut self, on: bool) -> io::Result<()>;
    /// Just joined a network: drop the old address and ask for a new one now.
    fn joined(&mut self) {}
    /// The addresses of the interfaces that are up, WiFi's included. getifaddrs by default.
    fn addresses(&self) -> io::Result<Vec<Interface>>;
}
```

| Wait | |
|---|---|
| Control socket to appear after power on | 10 s |
| A reply | 5 s |
| Joining, before giving up | 30 s |
| An address, before reporting `NoAddress` (it keeps waiting) | 20 s |
| Reconnecting after wpa_supplicant went away (e.g. around sleep) | every 1 s |
| Signal strength, while connected | every 10 s |
| The interfaces' addresses, on or off | every 5 s |

A WiFi password must be 8–63 printable ASCII characters, or 64 hex digits; others are refused before they reach the service.

**BlueZ.** Over the system D-Bus. One thread follows BlueZ's objects (the adapter, devices and audio connections), keeping only the properties Specular reads, and reports what changed. Slow calls (pairing, connecting: tens of seconds) each get a thread of their own, so later commands aren't held up. If the bus or bluetoothd isn't there yet, the service reports `NotAvailable` once and tries again every 5 s. Specular registers a "NoInputNoOutput" pairing agent that agrees only to pairings Specular started. Pairing also trusts the device, so it may reconnect by itself.

### Keeping the radios as wanted

After every message, the app works out what the radios and sound output should be from the settings and the screens open, and sends only the changes. Nothing is sent until the settings are loaded.

- **WiFi on Auto** is on while something needs it, then for 60 s more, so needs that come and go don't keep restarting the radio. A need made while WiFi isn't connected lapses after 30 s unless it connects by then, so a place with no known network doesn't keep the radio on; once connected, a need holds until dropped.
- **Bluetooth search** runs only while the Bluetooth screen is open: it costs power, and slows WiFi, which may share the antenna. It starts after powering on and stops before powering off.
- **Sound output** is the first connected Bluetooth device taking sound, else headphones in the jack, else the speaker (see [architecture.md](architecture.md)). An output that failed to open isn't tried again until its device disconnects, so a reconnection tries it afresh.

### The trace

One line on stderr per exchange:

```text
wireless <service> <seconds> <kind> <payload>
```

`seconds` counts from the first line. `kind` is `start`, `call` (a command sent) or `event` (an event reported), with the value as JSON; the services add `send` and `recv` for their own protocol exchanges, as text. Line breaks in a payload are escaped. A command a scenario has no answer for is traced as `scenario unanswered`.

### Scenarios

A scenario is a scripted radio, for the desktop build and tests. The desktop build reads `$SPECULAR_SCENARIO` (`{"wifi": <script>, "bluetooth": <script>}`), or a built-in one with a few networks and devices.

```json
{ "start":   [ { "after_ms": 0, "event": {"Link": "Idle"} } ],
  "answers": [ { "to": {"Join": {"ssid": "Home"}},
                 "then": [ { "after_ms": 800, "event": {"Link": {"Connected": {"ssid": "Home", "address": "10.0.0.2"}}} } ],
                 "repeat": true } ] }
```

Commands and events are written as they appear in the trace. A command takes the first unused answer it matches (a `repeat` answer is never used up); it matches if every field the answer's `to` gives is equal, so `to` can leave fields out. `after_ms` counts from the command, or from the start.

`tools/trace-to-scenario.py` turns a device's log into a scenario: events before a service's first command become its `start`; each command becomes a one-use answer made of the events that followed it until the next command, at the same delays. Repeating the same steps on the desktop then replays the same events. Passwords aren't in the trace, so an answer to a join matches any password. Events the service sends by itself, on a timer (signal strength, other interfaces' addresses), land in the answer to the command before them, so they replay only after that command.
