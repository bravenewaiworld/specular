//! Specular's own settings. Only what people genuinely disagree on is a user setting; the rest
//! have fixed defaults, or are developer settings hidden behind "Developer details".

use serde_json::json;

use super::{Scope, SettingDecl, SettingKind, Visibility};

pub const BRIGHTNESS: &str = "display.brightness";
/// How a game's picture fills the screen: "integer" (the largest whole multiple of its size),
/// "fit" (as large as fits, keeping its shape) or "stretch" (the whole screen).
pub const SCALING: &str = "video.scaling";
/// Sound volume, 0 (silent) to `VOLUME_MAX`; the volume keys step it.
pub const VOLUME: &str = "sound.volume";
pub const VOLUME_MAX: i64 = 20;
pub const LIGHT_BAR: &str = "lights.brightness";
/// The light bar's brightness steps (0 = off).
pub const LIGHT_BAR_MAX: i64 = 5;
/// How a system's games are shown: "list" (beside the focused game's box art) or "grid".
pub const LIBRARY_VIEW: &str = "library.view";
/// Shows the settings and details meant for developers.
pub const DEVELOPER_DETAILS: &str = "ui.developer_details";

pub fn settings() -> Vec<SettingDecl> {
    vec![
        SettingDecl { key: BRIGHTNESS, section: "Display", label: "Brightness",
            kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" }, default: json!(6),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: SCALING, section: "Display", label: "Scaling",
            kind: SettingKind::Choice(&[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")]), default: json!("integer"),
            visibility: Visibility::User, deepest: Scope::Game },
        SettingDecl { key: VOLUME, section: "Sound", label: "Volume",
            kind: SettingKind::Int { min: 0, max: VOLUME_MAX, step: 1, unit: "" }, default: json!(12),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: LIGHT_BAR, section: "Lights", label: "Light bar",
            kind: SettingKind::Int { min: 0, max: LIGHT_BAR_MAX, step: 1, unit: "" }, default: json!(3),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: LIBRARY_VIEW, section: "Library", label: "Games shown as",
            kind: SettingKind::Choice(&[("list", "List"), ("grid", "Grid")]), default: json!("list"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: "power.sleep_after", section: "Power", label: "Sleep after",
            kind: SettingKind::Choice(&[("never", "Never"), ("1", "1 min"), ("3", "3 min"), ("5", "5 min"), ("10", "10 min")]), default: json!("5"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: DEVELOPER_DETAILS, section: "System", label: "Developer details",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: "dev.frame_stats", section: "Developer", label: "Frame statistics",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global },
    ]
}
