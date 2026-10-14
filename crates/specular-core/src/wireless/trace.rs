//! The wireless trace: one line on stderr for every exchange with a radio's service, so what a
//! device did can be read in its log and replayed (see `scenario`). Lines are
//!
//! ```text
//! wireless <service> <seconds> <kind> <payload>
//! ```
//!
//! `seconds` counts from the first line, `kind` is `call` (a command sent) or `event` (an event
//! reported), each with the value as JSON; backends add `send` and `recv` for their own protocol
//! exchanges, as text. Secrets are never written (see `Secret`). The trace is always on: a few
//! lines per user action, so it costs the same everywhere.

use std::sync::OnceLock;
use std::time::Instant;

use serde::Serialize;

use super::{Events, Wireless};

/// Writes one trace line. `text` must hold no line breaks (they are replaced) and no secrets.
pub fn line(service: &str, kind: &str, text: &str) {
    static START: OnceLock<Instant> = OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    eprintln!("wireless {service} {t:.3} {kind} {}", text.replace('\n', "\\n"));
}

fn json(v: &impl Serialize) -> String { serde_json::to_string(v).unwrap_or_else(|e| format!("\"unserialisable: {e}\"")) }

/// A service whose commands and events are traced, as `service`.
pub struct Traced<S> { service: &'static str, inner: S }

impl<S> Traced<S> {
    pub fn new(service: &'static str, inner: S) -> Self { Self { service, inner } }
}

impl<C: Serialize, E: Serialize + 'static, S: Wireless<C, E>> Wireless<C, E> for Traced<S> {
    fn start(&mut self, events: Events<E>) {
        let service = self.service;
        line(service, "start", "");
        self.inner.start(Box::new(move |e| { line(service, "event", &json(&e)); events(e) }));
    }

    fn send(&mut self, cmd: C) {
        line(self.service, "call", &json(&cmd));
        self.inner.send(cmd);
    }
}

impl<C, E, S: Wireless<C, E> + ?Sized> Wireless<C, E> for Box<S> {
    fn start(&mut self, events: Events<E>) { (**self).start(events) }
    fn send(&mut self, cmd: C) { (**self).send(cmd) }
}
