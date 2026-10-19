//! The time zone in the app: the Time zone screen, the zone picker (areas, then the zones in
//! one), and finding the zone automatically (`App::reconcile_time_zone`).
//!
//! Automatic (the default) asks a lookup service for the network's zone each time WiFi connects,
//! which tells it the network's public address; so it needs the user's permission
//! (`builtin::TIME_ZONE_LOOKUP`). The question is put unprompted the first time a lookup would
//! happen (WiFi connected) while the user is browsing (home, a game list, Settings, the radios'
//! screens), never during a game; and when the user opens the Time zone screen, or chooses
//! Automatic, without having allowed it. It stays until answered (Menu doesn't leave it). A
//! device without WiFi counts as connected from the start, so it is asked (or looks up) once,
//! at start. Declining makes the zone manual: the zone in effect stays (UTC on a new device),
//! and the question is never put again unprompted. Picking a zone by hand makes it manual too.
//!
//! "Automatic" and "manual" are `time.zone`'s values ("auto", "manual"); the screen says
//! "Automatically" and "By hand".
//!
//! The device keeps the zone; the app only shows it and asks for changes. Failures (no network,
//! an unknown zone) are logged by the UI's time zone thread and otherwise only show as the zone
//! staying as it was.

use std::collections::HashMap;

use super::{App, Ctx, Effect, Outcome, Screen, action, screens, setting_press};
use crate::input::{Button, Press};
use crate::model::{Layout, Row, RowId, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;
use crate::settings::{Layer, builtin};
use crate::timezone::{ZoneCmd, ZoneEvent};
use crate::wireless::wifi::WifiLink;

/// What the app knows of the device's time zone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeZoneState {
    /// The zone in effect; None until the device says, or if it couldn't.
    pub zone: Option<String>,
    /// The zones to choose from, in the device's order.
    pub zones: Vec<String>,
}

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::TIME_ZONE, Box::new(TimeZoneScreen));
    screens.insert(screens::ZONE_AREAS, Box::new(AreasScreen));
    screens.insert(screens::ZONE_CITIES, Box::new(CitiesScreen));
    screens.insert(screens::ZONE_CONSENT, Box::new(ConsentScreen));
}

fn automatic(ctx: &Ctx) -> bool { ctx.settings.get(builtin::TIME_ZONE, &Layer::Global).is_some_and(|(v, _)| v == "auto") }
fn lookup_allowed(ctx: &Ctx) -> bool { ctx.settings.on(builtin::TIME_ZONE_LOOKUP) }

/// Sets a global setting, ignoring the error: the app passes only values it knows are valid.
fn set(ctx: &mut Ctx, key: &str, value: serde_json::Value) { let _ = ctx.settings.set(key, &Layer::Global, value); }

/// The zone in effect, if the device has said.
fn current(ctx: &Ctx) -> Option<&str> { ctx.time_zone.as_ref()?.zone.as_deref() }
/// The zones to choose from.
fn zones(ctx: &Ctx) -> &[String] { ctx.time_zone.as_ref().map_or(&[], |t| &t.zones) }

/// A zone's area: "Europe" for "Europe/Paris"; None for a zone without one ("UTC").
fn area(zone: &str) -> Option<&str> { zone.split_once('/').map(|(a, _)| a) }

/// A zone (or area) as the user sees it: "America/New York" for "America/New_York".
fn label(zone: &str) -> String { zone.replace('_', " ") }

/// The picker's first screen, with the focus on the zone in effect's area, or on the zone itself
/// if it has none (UTC).
fn areas(ctx: &Ctx) -> ScreenState {
    let mut s = ScreenState::new(screens::ZONE_AREAS, None);
    s.focus = current(ctx).map(|z| RowId(area(z).map_or(format!("zone:{z}"), |a| format!("area:{a}"))));
    s
}

impl App {
    /// Tells the app whether the device's time zone can be set (until told, it can't). Call before
    /// the first message.
    pub fn has_time_zone(&mut self, yes: bool) { self.ctx.time_zone = yes.then(TimeZoneState::default); }

    /// The connection the network is reached through: the WiFi network's name while connected,
    /// "" on a device without WiFi (assumed connected all along: see the module's documentation),
    /// else None.
    fn connection(&self) -> Option<String> {
        match self.ctx.wifi.as_ref().map(|w| &w.link) {
            None => Some(String::new()),
            Some(WifiLink::Connected { ssid, .. }) => Some(ssid.clone()),
            Some(_) => None,
        }
    }

    pub(super) fn time_zone_event(&mut self, e: ZoneEvent) -> Vec<Effect> {
        let auto = automatic(&self.ctx) && lookup_allowed(&self.ctx);
        let Some(tz) = &mut self.ctx.time_zone else { return vec![] };
        match e {
            ZoneEvent::Started { zone, zones } => *tz = TimeZoneState { zone, zones },
            ZoneEvent::Set { zone, result: Ok(()) } => tz.zone = Some(zone),
            // Only if still automatic (the user may have picked a zone by hand since the lookup
            // was sent), and only if it differs from the zone in effect.
            ZoneEvent::LookedUp(Ok(zone)) if auto && tz.zone.as_ref() != Some(&zone) => return vec![Effect::TimeZone(ZoneCmd::Set(zone))],
            ZoneEvent::Set { result: Err(_), .. } | ZoneEvent::LookedUp(_) => return vec![],
        }
        self.changed();
        vec![Effect::Redraw(None)]
    }

    /// Called after every message. Sends a lookup when one is due: Automatic, allowed, and
    /// connected through a connection not yet looked up for (`zone_looked_up_for`, cleared when
    /// the connection ends or the zone is made manual, so choosing Automatic again looks it up
    /// again). Opens the question when Automatic isn't allowed yet and the user is browsing while
    /// connected, or is on the Time zone screen.
    pub(super) fn reconcile_time_zone(&mut self) -> Vec<Effect> {
        if !self.settings_loaded || self.ctx.time_zone.is_none() { return vec![]; }
        let auto = automatic(&self.ctx);
        let wanted = self.connection().filter(|_| auto);
        if wanted.is_none() { self.zone_looked_up_for = None; }
        if lookup_allowed(&self.ctx) {
            if self.zone_looked_up_for == wanted { return vec![]; }
            self.zone_looked_up_for = wanted;
            return vec![Effect::TimeZone(ZoneCmd::LookUp)];
        }
        // (While the question is open it is the top screen, so it isn't put twice.)
        let top = &self.nav.top().id;
        let browsing = [screens::HOME, screens::GAMES, screens::SETTINGS, screens::WIFI, screens::BLUETOOTH].contains(top);
        let due = auto && ((wanted.is_some() && browsing) || *top == screens::TIME_ZONE);
        if !due || self.playing.is_some() { return vec![]; }
        self.go(|nav| Some(nav.push(ScreenState::new(screens::ZONE_CONSENT, None))))
    }

    /// A press on the question: Allow makes the zone automatic and allows lookups; "Choose it
    /// myself" makes it manual and opens the picker; B makes it manual. Declining also sets
    /// `time.zone_lookup_allowed` to false. Each answer is saved to the global settings.
    pub(super) fn zone_consent(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        let (allow, pick) = match (button, row) {
            (Button::A, Some("allow")) => (true, false),
            (Button::A, Some("manual")) => (false, true),
            (Button::B, _) => (false, false),
            _ => return vec![],
        };
        set(&mut self.ctx, builtin::TIME_ZONE_LOOKUP, allow.into());
        set(&mut self.ctx, builtin::TIME_ZONE, if allow { "auto" } else { "manual" }.into());
        let mut e = vec![Effect::SaveSettings(Layer::Global)];
        let picker = pick.then(|| areas(&self.ctx));
        e.extend(self.go(|nav| { let t = nav.pop(); picker.map(|s| nav.push(s)).or(t) }));
        e
    }

    /// A zone picked (from the areas, for a zone without one such as UTC, or from an area's
    /// zones): it becomes the zone, by hand, and the picker closes.
    pub(super) fn pick_zone(&mut self, zone: &str) -> Vec<Effect> {
        set(&mut self.ctx, builtin::TIME_ZONE, "manual".into());
        let mut e = vec![Effect::SaveSettings(Layer::Global), Effect::TimeZone(ZoneCmd::Set(zone.into()))];
        e.extend(self.go(|nav| nav.close(&screens::ZONE_AREAS)));
        e
    }
}

/// Settings' Time section: the zone in effect, opening the Time zone screen ("open:time zone").
pub(super) fn settings_rows(ctx: &Ctx) -> Vec<Row> {
    if ctx.time_zone.is_none() { return vec![]; }
    vec![Row::new("section:Time", "Time", RowKind::Header),
         Row::new("open:time zone", "Time zone", RowKind::Action { value: current(ctx).map(label) })]
}

/// The Time zone screen: how the zone is set (`time.zone`: "Automatically" or "By hand"), and
/// the zone in effect, which opens the picker.
struct TimeZoneScreen;

impl Screen for TimeZoneScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let mut rows: Vec<Row> = ctx.settings.row(builtin::TIME_ZONE, &Layer::Global).into_iter().collect();
        if automatic(ctx) {
            rows.push(Row::note("note:auto", if lookup_allowed(ctx) { "Found from your network's address (by ipinfo.io) each time Wi-Fi connects" } else { "Waiting for your permission to look it up" }));
        }
        rows.push(Row::new("zone", "Zone", RowKind::Action { value: Some(current(ctx).map_or("Unknown".into(), label)) }));
        ScreenView::list("Time zone", rows)
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused) {
            (Button::B, _) => Outcome::Back,
            (Button::A, Some(row)) if row.id.0 == "zone" => Outcome::Push(areas(ctx)),
            // While By hand and not allowed: any press that changes the choice would select
            // Automatically (there are only two), so ask for permission instead.
            (Button::A | Button::Left | Button::Right, Some(row)) if row.id.0 == builtin::TIME_ZONE && !automatic(ctx) && !lookup_allowed(ctx) => {
                if p.repeat { Outcome::None } else { Outcome::Push(ScreenState::new(screens::ZONE_CONSENT, None)) }
            }
            (_, Some(row)) if row.id.0 == builtin::TIME_ZONE => setting_press(ctx, row, p),
            _ => Outcome::None,
        }
    }
}

/// The picker's first screen: zones without an area (UTC), then the areas ("Europe"), each
/// opening its zones. Row ids are "zone:" and a zone, or "area:" and an area. A on a zone is
/// handled by the app (`App::pick_zone`) before this screen sees it.
struct AreasScreen;

impl Screen for AreasScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let zones = zones(ctx);
        let mut rows: Vec<Row> = zones.iter().filter(|z| area(z).is_none()).map(|z| action(&format!("zone:{z}"), &label(z))).collect();
        let mut seen = vec![];
        for a in zones.iter().filter_map(|z| area(z)) {
            if !seen.contains(&a) { seen.push(a); rows.push(action(&format!("area:{a}"), &label(a))); }
        }
        if rows.is_empty() { rows.push(Row::note("note:none", "The list of time zones couldn't be read")); }
        ScreenView { title: "Choose a time zone".into(), layout: Layout::List, rows, hints: vec![(Button::B, "Back".into()), (Button::A, "Open".into())], entry: None }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused.and_then(|r| r.id.0.strip_prefix("area:"))) {
            (Button::B, _) => Outcome::Back,
            (Button::A, Some(a)) => {
                let mut s = ScreenState::new(screens::ZONE_CITIES, Some(a.into()));
                s.focus = current(ctx).map(|z| RowId(format!("zone:{z}")));
                Outcome::Push(s)
            }
            _ => Outcome::None,
        }
    }
}

/// One area's zones (the area is the params), in the device's order (by name, which the letter
/// index relies on), the zone in effect marked. Row ids are "zone:" and the zone; A on one is
/// handled by the app (`App::pick_zone`) before this screen sees it.
struct CitiesScreen;

impl Screen for CitiesScreen {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let a = state.params.as_deref().unwrap_or_default();
        let rows = zones(ctx).iter().filter(|z| area(z) == Some(a)).map(|z| {
            // "Argentina / Buenos Aires" for America/Argentina/Buenos_Aires.
            let name = label(&z[a.len() + 1..]).replace('/', " / ");
            let detail = (current(ctx) == Some(z.as_str())).then(|| "Current".into());
            Row::new(format!("zone:{z}"), name, RowKind::Text { detail })
        }).collect();
        ScreenView {
            title: label(a), layout: Layout::List, rows,
            hints: vec![(Button::B, "Back".into()), (Button::A, "Choose".into())], entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { Outcome::Back } else { Outcome::None }
    }

    fn letter_index(&self) -> bool { true }
}

/// The question whether the zone may be looked up: it says that ipinfo.io learns the network's
/// public address, and offers Allow or Choose it myself (B declines). The app handles its
/// presses (`App::zone_consent`).
struct ConsentScreen;

impl Screen for ConsentScreen {
    fn view(&self, _: &Ctx, _: &ScreenState) -> ScreenView {
        ScreenView {
            title: "Look up the time zone?".into(), layout: Layout::List,
            rows: vec![
                Row::note("note:what", "Each time Wi-Fi connects, Specular can ask ipinfo.io which time zone your network is in. This tells ipinfo.io your network's public address."),
                action("allow", "Allow"), action("manual", "Choose it myself"),
            ],
            hints: vec![(Button::B, "Don't allow".into()), (Button::A, "Select".into())], entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}
