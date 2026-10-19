//! The time zone's thread: the device's zone (`TimeZone`), and the lookup that asks ipinfo.io for
//! the network's zone. Every exchange is logged (`timezone:`), with the lookup's answer as it came,
//! so a lookup seen on a device can be replayed with $SPECULAR_TIMEZONE_LOOKUP.

use std::sync::mpsc;
use std::time::Duration;

use specular_core::app::Msg;
use specular_core::timezone::{TimeZone, ZoneCmd, ZoneEvent, zone_name};

use crate::post;

/// The lookup service. Plain HTTP, since Specular's HTTP client has no TLS (and the system no
/// certificates). It answers a bare zone name, such as "America/Toronto".
const LOOKUP_URL: &str = "http://ipinfo.io/timezone";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Starts the thread: it reports the zone in effect and the zones to choose from, then carries
/// out the commands sent, in order.
pub fn thread(mut zone: Box<dyn TimeZone>) -> mpsc::Sender<ZoneCmd> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("timezone".into()).spawn(move || {
        let current = zone.get().map_err(|e| eprintln!("timezone: get: {e}")).ok();
        let zones = zone.list().map_err(|e| eprintln!("timezone: list: {e}")).unwrap_or_default();
        eprintln!("timezone: {} in effect, {} to choose from", current.as_deref().unwrap_or("unknown"), zones.len());
        post(Msg::TimeZone(ZoneEvent::Started { zone: current, zones }));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(LOOKUP_TIMEOUT))
            .user_agent(concat!("Specular/", env!("CARGO_PKG_VERSION"), " (retro game launcher)"))
            .build().into();
        for cmd in rx {
            let event = match cmd {
                ZoneCmd::Set(name) => {
                    let result = zone.set(&name);
                    match &result { Ok(()) => eprintln!("timezone: set {name}"), Err(e) => eprintln!("timezone: set {name}: refused: {e}") }
                    ZoneEvent::Set { zone: name, result }
                }
                ZoneCmd::LookUp => {
                    let result = look_up(&agent);
                    if let Err(e) = &result { eprintln!("timezone: lookup: {e}"); }
                    ZoneEvent::LookedUp(result)
                }
            };
            post(Msg::TimeZone(event));
        }
    });
    if let Err(e) = spawned { eprintln!("timezone: can't start its thread: {e}"); }
    tx
}

/// Asks the lookup service for the network's zone; with $SPECULAR_TIMEZONE_LOOKUP set, takes its
/// value as the answer instead, without the network (empty: no answer).
fn look_up(agent: &ureq::Agent) -> Result<String, String> {
    let answer = match std::env::var("SPECULAR_TIMEZONE_LOOKUP") {
        Ok(answer) => { eprintln!("timezone: lookup: $SPECULAR_TIMEZONE_LOOKUP answered {answer:?}"); answer }
        Err(_) => {
            let mut response = agent.get(LOOKUP_URL).call().map_err(|e| format!("{LOOKUP_URL}: {e}"))?;
            let body = response.body_mut().with_config().limit(1024).read_to_string().map_err(|e| format!("{LOOKUP_URL}: {e}"))?;
            eprintln!("timezone: lookup: {LOOKUP_URL} answered {body:?}");
            body
        }
    };
    zone_name(&answer)
}
