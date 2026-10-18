//! What WiFi services take and report.

use serde::{Deserialize, Serialize};

use super::Secret;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WifiCmd {
    /// Turns the radio on (driver, network service, address) or off.
    Power(bool),
    /// Looks for networks in range; the result arrives as `WifiEvent::Networks`.
    Scan,
    /// Joins a network and saves it. `password` is None for an open network, or to rejoin a
    /// saved one with its saved password.
    Join { ssid: String, password: Option<Secret> },
    /// Forgets a saved network (leaving it if connected).
    Forget { ssid: String },
}

/// A network in range, or a saved one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Network {
    pub ssid: String,
    /// Signal strength, 0–4 bars (0 when out of range).
    pub bars: u8,
    pub secured: bool,
    pub saved: bool,
    /// Seen by the last scan.
    pub in_range: bool,
    /// Its security is one the device can join (absent in scenarios: true).
    #[serde(default = "yes")]
    pub supported: bool,
}

fn yes() -> bool { true }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WifiLink {
    Off,
    /// On, not connected: no saved network is in range, or it's still looking.
    Idle,
    Connecting { ssid: String, step: JoinStep },
    /// `address` is the device's address on the network, as text. `bars` is the live signal
    /// strength, 0–4 (None until the first reading, or when the service can't give one).
    Connected { ssid: String, address: String, #[serde(default)] bars: Option<u8> },
}

/// One IPv4 address of a network interface (one with several appears once for each).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interface { pub name: String, pub address: String }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinStep { Associating, Authenticating, GettingAddress }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WifiEvent {
    Link(WifiLink),
    /// Every network known: those in range, then saved ones out of range. Sent after each scan
    /// and whenever the saved networks change.
    Networks(Vec<Network>),
    /// The addresses of the device's other interfaces that are up and have a link, such as the
    /// USB network link (not WiFi's, whose address is in `Connected`, nor loopback). Sent when
    /// they change, whether WiFi is on or off; they count as none until then, so a device with
    /// none never sends it.
    Interfaces(Vec<Interface>),
    Failed(WifiError),
}

/// Why WiFi couldn't do what was asked. `detail`s hold the service's own words, for the trace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WifiError {
    /// The hardware or its service didn't start.
    NotAvailable { detail: String },
    WrongPassword { ssid: String },
    /// Joined, but the network gave no address (DHCP).
    NoAddress { ssid: String },
    OutOfRange { ssid: String },
    /// A password that WiFi can't use (it must be 8–63 characters, or 64 hex digits).
    BadPassword { ssid: String },
    /// Its security can't be used here (say, WPA3-only or enterprise).
    Unsupported { ssid: String, detail: String },
    /// Anything else, at `step` ("scan", "save", ...).
    Service { step: String, detail: String },
}

impl WifiError {
    /// A sentence for the user.
    pub fn message(&self) -> String {
        match self {
            WifiError::NotAvailable { .. } => "Wi-Fi didn't start".into(),
            WifiError::WrongPassword { ssid } => format!("Wrong password for {ssid}"),
            WifiError::NoAddress { ssid } => format!("{ssid} didn't give an address"),
            WifiError::OutOfRange { ssid } => format!("{ssid} is out of range"),
            WifiError::BadPassword { .. } => "A Wi-Fi password has 8 to 63 characters".into(),
            WifiError::Unsupported { ssid, .. } => format!("{ssid} uses security this device doesn't support"),
            WifiError::Service { step, .. } => format!("Wi-Fi: {step} failed"),
        }
    }
}

/// Whether `password` is one WiFi (WPA) accepts: 8–63 printable ASCII characters, or 64 hex
/// digits (a raw key).
pub fn valid_password(password: &str) -> bool {
    let printable = password.chars().all(|c| (' '..='~').contains(&c));
    (printable && (8..=63).contains(&password.len())) || (password.len() == 64 && password.chars().all(|c| c.is_ascii_hexdigit()))
}
