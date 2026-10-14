//! WiFi and Bluetooth. Each radio is a `Wireless` service: commands in, events out. Devices
//! supply the services (specular-linux has the standard wpa_supplicant and BlueZ ones);
//! `trace::Traced` records every command and event, and `scenario::Scenario` simulates a radio
//! from a script, which `tools/trace-to-scenario.py` can make from such a record, to replay what
//! a device did on a desktop.

pub mod bluetooth;
pub mod scenario;
pub mod trace;
pub mod wifi;

use serde::{Deserialize, Serialize, Serializer};

/// Where a service's events go. Called on the service's own threads.
pub type Events<E> = Box<dyn Fn(E) + Send + Sync>;

/// A radio's service. Commands may block (loading a driver takes seconds), so the UI sends them
/// from a thread of its own. Failures are reported as events rather than returned, so the app
/// sees one stream of what happened.
pub trait Wireless<C, E>: Send {
    /// Starts reporting events, once, before any command. The service then reports its whole
    /// state (powered or not, the networks or devices it knows) without being asked.
    fn start(&mut self, events: Events<E>);
    fn send(&mut self, cmd: C);
}

/// The WiFi service.
pub type Wifi = dyn Wireless<wifi::WifiCmd, wifi::WifiEvent>;
/// The Bluetooth service.
pub type Bluetooth = dyn Wireless<bluetooth::BtCmd, bluetooth::BtEvent>;

/// Text that must never be logged, such as a password. `Debug` and serialising both give
/// `<secret>`, so it can't leak into a trace; deserialising (in scenarios) takes any string.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
    /// The text itself, for handing to the service that needs it.
    pub fn expose(&self) -> &str { &self.0 }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("<secret>") }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str("<secret>") }
}
