//! A developer screen listing every icon in each of its states, to check them on a device
//! without the radios or the battery being in those states.

use std::collections::HashMap;

use super::{Ctx, Outcome, Screen, screens};
use crate::device::Battery;
use crate::input::{Button, Press};
use crate::model::{Icon, Row, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;
use crate::wireless::bluetooth::BtKind;

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) { screens.insert(screens::ICONS, Box::new(IconsScreen)); }

struct IconsScreen;

fn header(text: &str) -> Row { Row::new(format!("section:{text}"), text, RowKind::Header) }

fn sample(label: &str, detail: &str, icon: Icon) -> Row {
    Row::new(format!("{label} {detail}"), label, RowKind::Text { detail: Some(detail.into()) }).icon(icon)
}

fn rows() -> Vec<Row> {
    let wifi = |label, bars, locked| sample(label, if locked { "Secured" } else { "Open" }, Icon::Wifi { bars, locked });
    let mut rows = vec![
        header("Wi-Fi"),
        wifi("No signal", Some(0), false), wifi("1 bar", Some(1), false), wifi("2 bars", Some(2), true),
        wifi("3 bars", Some(3), false), wifi("4 bars", Some(4), true), wifi("Unknown", None, false),
        header("Bluetooth"),
    ];
    let kinds = [("Headphones", BtKind::Headphones), ("Speaker", BtKind::Speaker), ("Controller", BtKind::Controller), ("Keyboard", BtKind::Keyboard),
                 ("Phone", BtKind::Phone), ("Computer", BtKind::Computer), ("Other", BtKind::Other)];
    for lit in [true, false] {
        rows.extend(kinds.iter().map(|&(label, kind)| sample(label, if lit { "Connected" } else { "Not connected" }, Icon::Bluetooth { kind, lit })));
    }
    rows.push(header("Battery"));
    let battery = |label: &str, percent, charging, plugged| sample(label, "", Icon::Battery(Battery { percent, charging, plugged }));
    rows.extend([
        battery("Full, on power", Some(100), false, true), battery("Full", Some(100), false, false), battery("78 %", Some(78), false, false),
        battery("50 %", Some(50), false, false), battery("30 %", Some(30), false, false), battery("16 %", Some(16), false, false),
        // 16 and 15 straddle `Battery::low`'s threshold.
        battery("15 %, low", Some(15), false, false), battery("5 %, low", Some(5), false, false), battery("0 %", Some(0), false, false),
        battery("8 % charging", Some(8), true, true), battery("60 % charging", Some(60), true, true), battery("Unknown", None, false, false),
    ]);
    rows
}

impl Screen for IconsScreen {
    fn view(&self, _: &Ctx, _: &ScreenState) -> ScreenView {
        ScreenView::list("Icons", rows())
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { Outcome::Back } else { Outcome::None }
    }
}
