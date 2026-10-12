//! Specular's own settings. Only what people genuinely disagree on is a user setting; the rest
//! have good defaults and stay in the files for those who look.

use serde_json::json;

use super::{Layer, SettingDecl, SettingKind, Visibility};

/// Shows the settings and details meant for developers.
pub const DEVELOPER_DETAILS: &str = "ui.developer_details";

/// Settings the device carries out (the rest only change what Specular does), applied as soon as
/// the settings are loaded and whenever they change.
pub const APPLIED: [&str; 2] = ["display.brightness", "lights.brightness"];

/// Any game may override these (its deepest layer).
fn game() -> Layer { Layer::Game { system: String::new(), game: String::new() } }

pub fn settings() -> Vec<SettingDecl> {
    vec![
        SettingDecl { key: "display.brightness", section: "Display", label: "Brightness",
            kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" }, default: json!(6),
            visibility: Visibility::User, deepest: Layer::Global },
        SettingDecl { key: "video.scaling", section: "Display", label: "Scaling",
            kind: SettingKind::Choice(&[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")]), default: json!("integer"),
            visibility: Visibility::User, deepest: game() },
        SettingDecl { key: "lights.brightness", section: "Lights", label: "Light bar",
            kind: SettingKind::Int { min: 0, max: 5, step: 1, unit: "" }, default: json!(3),
            visibility: Visibility::User, deepest: Layer::Global },
        SettingDecl { key: "power.sleep_after", section: "Power", label: "Sleep after",
            kind: SettingKind::Choice(&[("never", "Never"), ("1", "1 min"), ("3", "3 min"), ("5", "5 min"), ("10", "10 min")]), default: json!("5"),
            visibility: Visibility::User, deepest: Layer::Global },
        SettingDecl { key: DEVELOPER_DETAILS, section: "System", label: "Developer details",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::User, deepest: Layer::Global },
        SettingDecl { key: "dev.frame_stats", section: "Developer", label: "Frame statistics",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Layer::Global },
    ]
}
