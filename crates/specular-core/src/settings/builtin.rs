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
/// The LEDs' brightness (its key predates there being more LEDs than the light bar).
pub const LED_BRIGHTNESS: &str = "lights.brightness";
/// The LEDs' brightness steps (0 = off).
pub const LED_BRIGHTNESS_MAX: i64 = 5;
/// The accent (a palette id): the on-screen light bar, toggles and unfocused gauges, and the LEDs
/// the device profile marks `Accent::First`.
pub const ACCENT: &str = "appearance.accent";
/// The second accent (a palette id, or `SAME`): the focus marks, and the LEDs marked
/// `Accent::Second`.
pub const SECOND_ACCENT: &str = "appearance.second_accent";
/// Sleep after this many seconds without a button press, outside a running game ("never" for
/// never).
pub const SLEEP_AFTER: &str = "power.sleep_after_seconds";
/// How a system's games are shown: "list" (beside the focused game's box art) or "grid".
pub const LIBRARY_VIEW: &str = "library.view";
/// Shows the game's frame rate and the core's time per frame over the game (a developer setting).
pub const FRAME_STATS: &str = "dev.frame_stats";
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
        SettingDecl { key: LED_BRIGHTNESS, section: "Lights", label: "Brightness",
            kind: SettingKind::Int { min: 0, max: LED_BRIGHTNESS_MAX, step: 1, unit: "" }, default: json!(3),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: ACCENT, section: "Appearance", label: "Accent",
            kind: SettingKind::Colour { same_as: None }, default: json!("teal"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: SECOND_ACCENT, section: "Appearance", label: "Second accent",
            kind: SettingKind::Colour { same_as: Some(ACCENT) }, default: json!("coral"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: LIBRARY_VIEW, section: "Library", label: "Games shown as",
            kind: SettingKind::Choice(&[("list", "List"), ("grid", "Grid")]), default: json!("list"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: SLEEP_AFTER, section: "Power", label: "Sleep after",
            kind: SettingKind::Choice(&[("never", "Never"), ("30", "30 sec"), ("60", "1 min"), ("180", "3 min"), ("300", "5 min"), ("600", "10 min")]), default: json!("30"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: DEVELOPER_DETAILS, section: "System", label: "Developer details",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: FRAME_STATS, section: "Developer", label: "Frame statistics",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global },
    ]
}
