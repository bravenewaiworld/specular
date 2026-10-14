//! A simulated radio that follows a script, for the desktop build and tests. A script says what
//! happens when the service starts and how it answers commands:
//!
//! ```json
//! { "start":   [ { "after_ms": 0, "event": {"Link": "Idle"} } ],
//!   "answers": [ { "to": {"Join": {"ssid": "Home"}},
//!                  "then": [ { "after_ms": 800, "event": {"Link": {"Connected": {"ssid": "Home", "address": "10.0.0.2"}}} } ],
//!                  "repeat": true } ] }
//! ```
//!
//! Commands and events are written as they serialise (as in the trace). A command takes the
//! first answer it matches that hasn't been used (a `repeat` answer is never used up); it matches
//! if every field the answer's `to` gives is equal, so `to` can leave fields out. A command with
//! no answer does nothing. `after_ms` counts from the command (or the start).

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Events, Wireless, trace};

/// One event of a script, `after_ms` after what set it off.
#[derive(Clone, Debug, PartialEq)]
struct Timed { after: Duration, event: Value }

#[derive(Clone, Debug)]
struct Answer { to: Value, then: Vec<Timed>, repeat: bool, used: bool }

/// A scripted service; see the module's description.
pub struct Scenario<E> {
    start: Vec<Timed>,
    answers: Vec<Answer>,
    events: Option<Arc<Events<E>>>,
    _event: PhantomData<fn(E)>,
}

impl<E: DeserializeOwned + 'static> Scenario<E> {
    /// Reads a script. Fails, naming the place, on a script that isn't one or an event that isn't
    /// an `E`.
    pub fn from_json(script: &Value) -> Result<Self, String> {
        const NONE: &Vec<Value> = &Vec::new();
        let timeline = |v: Option<&Value>, at: &str| -> Result<Vec<Timed>, String> {
            v.map_or(Ok(NONE), |v| v.as_array().ok_or(format!("{at}: not a list")))?.iter().enumerate().map(|(i, t)| {
                let event = t.get("event").cloned().ok_or(format!("{at}[{i}]: no event"))?;
                serde_json::from_value::<E>(event.clone()).map_err(|e| format!("{at}[{i}]: {e}"))?;
                Ok(Timed { after: Duration::from_millis(t.get("after_ms").and_then(Value::as_u64).unwrap_or(0)), event })
            }).collect()
        };
        let answers = script.get("answers").map_or(Ok(NONE), |v| v.as_array().ok_or("answers: not a list".to_string()))?;
        Ok(Self {
            start: timeline(script.get("start"), "start")?,
            answers: answers.iter().enumerate().map(|(i, a)| Ok(Answer {
                to: a.get("to").cloned().ok_or(format!("answers[{i}]: no \"to\""))?,
                then: timeline(a.get("then"), &format!("answers[{i}].then"))?,
                repeat: a.get("repeat").and_then(Value::as_bool).unwrap_or(false),
                used: false,
            })).collect::<Result<_, String>>()?,
            events: None,
            _event: PhantomData,
        })
    }

    /// Plays `timeline` on a thread of its own.
    fn play(&self, timeline: Vec<Timed>) {
        let Some(events) = self.events.clone() else { return };
        if timeline.is_empty() { return; }
        std::thread::spawn(move || {
            let mut elapsed = Duration::ZERO;
            for t in timeline {
                std::thread::sleep(t.after.saturating_sub(elapsed));
                elapsed = elapsed.max(t.after);
                // Checked when the script was read.
                if let Ok(e) = serde_json::from_value(t.event) { events(e); }
            }
        });
    }
}

/// Whether `value` has everything `pattern` gives.
fn matches(pattern: &Value, value: &Value) -> bool {
    match (pattern, value) {
        (Value::Object(p), Value::Object(v)) => p.iter().all(|(k, pv)| v.get(k).is_some_and(|vv| matches(pv, vv))),
        _ => pattern == value,
    }
}

impl<C: Serialize, E: DeserializeOwned + 'static> Wireless<C, E> for Scenario<E> {
    fn start(&mut self, events: Events<E>) {
        self.events = Some(Arc::new(events));
        self.play(self.start.clone());
    }

    fn send(&mut self, cmd: C) {
        let cmd = serde_json::to_value(&cmd).unwrap_or(Value::Null);
        let Some(a) = self.answers.iter_mut().find(|a| !a.used && matches(&a.to, &cmd)) else {
            trace::line("scenario", "unanswered", &cmd.to_string());
            return;
        };
        a.used = !a.repeat;
        let then = a.then.clone();
        self.play(then);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use serde_json::json;

    use super::*;
    use crate::wireless::wifi::{WifiCmd, WifiEvent, WifiLink};

    #[test]
    fn answers_in_order_matching_parts_of_commands() {
        let script = json!({
            "start": [{"event": {"Link": "Idle"}}],
            "answers": [
                {"to": {"Join": {"ssid": "Home"}}, "then": [{"after_ms": 5, "event": {"Link": {"Connected": {"ssid": "Home", "address": "a"}}}}]},
                {"to": "Scan", "then": [{"event": {"Networks": []}}], "repeat": true},
            ],
        });
        let mut s = Scenario::<WifiEvent>::from_json(&script).unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        Wireless::<WifiCmd, _>::start(&mut s, Box::new(move |e| tx.lock().unwrap().send(e).unwrap()));
        let next = || rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(next(), WifiEvent::Link(WifiLink::Idle));
        s.send(WifiCmd::Join { ssid: "Home".into(), password: Some(crate::wireless::Secret::new("hunter22")) });
        assert_eq!(next(), WifiEvent::Link(WifiLink::Connected { ssid: "Home".into(), address: "a".into() }));
        s.send(WifiCmd::Join { ssid: "Home".into(), password: None }); // used up: nothing
        s.send(WifiCmd::Scan);
        s.send(WifiCmd::Scan);
        assert_eq!((next(), next()), (WifiEvent::Networks(vec![]), WifiEvent::Networks(vec![])));
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn bad_scripts_say_where() {
        let e = Scenario::<WifiEvent>::from_json(&json!({"answers": [{"to": "Scan", "then": [{"event": "Nonsense"}]}]})).err().unwrap();
        assert!(e.starts_with("answers[0].then[0]:"), "{e}");
    }
}
