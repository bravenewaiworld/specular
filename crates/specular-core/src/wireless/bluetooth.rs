//! What Bluetooth services take and report.

use serde::{Deserialize, Serialize};

/// A device's address, as BlueZ writes it ("AA:BB:CC:DD:EE:FF").
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BtAddress(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BtCmd {
    Power(bool),
    /// Looking for new devices to pair (costs power, and slows WiFi, which may share the antenna).
    Search(bool),
    /// Pairs, trusts (so it may reconnect by itself) and connects.
    Pair(BtAddress),
    Connect(BtAddress),
    Disconnect(BtAddress),
    /// Removes the pairing.
    Forget(BtAddress),
}

/// What a device is, from its class or icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BtKind { Headphones, Speaker, Controller, Keyboard, Phone, Computer, Other }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BtDevice {
    pub address: BtAddress,
    /// Its name; devices without one aren't reported.
    pub name: String,
    pub kind: BtKind,
    pub paired: bool,
    pub connected: bool,
    /// Connected and taking sound (an audio sink, ready for playback).
    pub audio: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BtEvent {
    Powered(bool),
    Searching(bool),
    /// Every device known: paired ones, and those found by searching. Sent whenever one changes.
    Devices(Vec<BtDevice>),
    Failed(BtError),
}

/// Why Bluetooth couldn't do what was asked. `detail`s hold the service's own words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BtError {
    /// The hardware or its service isn't there.
    NotAvailable { detail: String },
    PairingFailed { name: String, detail: String },
    ConnectFailed { name: String, detail: String },
    /// Anything else, at `step` ("power", "search", "forget", ...).
    Service { step: String, detail: String },
}

impl BtError {
    /// A sentence for the user.
    pub fn message(&self) -> String {
        match self {
            BtError::NotAvailable { .. } => "Bluetooth isn't available".into(),
            BtError::PairingFailed { name, .. } => format!("Couldn't pair with {name}. Put it in pairing mode, then try again."),
            BtError::ConnectFailed { name, .. } => format!("Couldn't connect to {name}. Check it is on and nearby, then try again."),
            // (The step and detail are in the trace.)
            BtError::Service { .. } => "Bluetooth had a problem. Try again, or turn Bluetooth off and on.".into(),
        }
    }
}
