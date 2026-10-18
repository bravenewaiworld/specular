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
/// The volume of Bluetooth headphones and speakers (all of them share it), 0 to `VOLUME_MAX`;
/// `VOLUME` is the device's own speaker and jack.
pub const BLUETOOTH_VOLUME: &str = "sound.bluetooth_volume";
/// WiFi: "on", "off", or "auto" (on only while something needs it: see `app::WifiNeed`). Shown
/// on the Wi-Fi screen.
pub const WIFI: &str = "wireless.wifi";
/// Bluetooth on or off. Shown on the Bluetooth screen.
pub const BLUETOOTH: &str = "wireless.bluetooth";
/// WiFi set to Auto stays on (a developer setting, for SSH and file transfer over WiFi, which
/// don't ask for it yet).
pub const WIFI_KEEP_ON: &str = "dev.wifi_keep_on";
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
            help: "How bright the screen is, from 1 (dimmest) to 10 (brightest).",
            kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" }, default: json!(6),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: SCALING, section: "Display", label: "Scaling",
            help: "How a game's picture fills the screen: \"integer\" makes it the largest whole multiple of its size that fits, for the sharpest pixels; \"fit\" makes it as large as fits, keeping its shape; \"stretch\" fills the whole screen.",
            kind: SettingKind::Choice(&[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")]), default: json!("integer"),
            visibility: Visibility::User, deepest: Scope::Game },
        SettingDecl { key: VOLUME, section: "Sound", label: "Volume",
            help: "The volume of the device's own speaker and headphone jack, from 0 (silent) to 20. The volume keys change it.",
            kind: SettingKind::Int { min: 0, max: VOLUME_MAX, step: 1, unit: "" }, default: json!(12),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: BLUETOOTH_VOLUME, section: "Sound", label: "Bluetooth volume",
            help: "The volume of Bluetooth headphones and speakers (all of them share it), from 0 (silent) to 20. The volume keys change it while sound goes to one.",
            kind: SettingKind::Int { min: 0, max: VOLUME_MAX, step: 1, unit: "" }, default: json!(10),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: LED_BRIGHTNESS, section: "Lights", label: "Brightness",
            help: "How bright the device's lights are, from 0 (off) to 5 (brightest).",
            kind: SettingKind::Int { min: 0, max: LED_BRIGHTNESS_MAX, step: 1, unit: "" }, default: json!(3),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: ACCENT, section: "Appearance", label: "Accent",
            help: "The main accent colour, by name. It colours the glowing bar along the top of the screen, switches, gauges that aren't selected, and some of the device's lights.",
            kind: SettingKind::Colour { same_as: None }, default: json!("teal"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: SECOND_ACCENT, section: "Appearance", label: "Second accent",
            help: "The second accent colour, by name, or \"same\" for the main accent colour. It marks what is selected, and colours the rest of the device's lights.",
            kind: SettingKind::Colour { same_as: Some(ACCENT) }, default: json!("coral"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: LIBRARY_VIEW, section: "Library", label: "Games shown as",
            help: "How a system's games are shown: \"list\" lists them beside the selected game's box art; \"grid\" shows their box art side by side.",
            kind: SettingKind::Choice(&[("list", "List"), ("grid", "Grid")]), default: json!("list"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: SLEEP_AFTER, section: "Power", label: "Sleep after",
            help: "Sleep after this many seconds without a button press: \"30\", \"60\", \"180\", \"300\" or \"600\" (in quotes), or \"never\". The device never goes to sleep by itself while a game is being played.",
            kind: SettingKind::Choice(&[("never", "Never"), ("30", "30 sec"), ("60", "1 min"), ("180", "3 min"), ("300", "5 min"), ("600", "10 min")]), default: json!("30"),
            visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: DEVELOPER_DETAILS, section: "System", label: "Developer details",
            help: "Also shows, in Specular's menus, the settings and technical details meant for developers.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: WIFI, section: "Wireless", label: "Wi-Fi",
            help: "\"on\", \"off\", or \"auto\", which turns Wi-Fi on only while something needs it (such as downloading box art).",
            kind: SettingKind::Choice(&[("auto", "Auto"), ("on", "On"), ("off", "Off")]), default: json!("auto"),
            visibility: Visibility::OwnScreen, deepest: Scope::Global },
        SettingDecl { key: BLUETOOTH, section: "Wireless", label: "Bluetooth",
            help: "Whether Bluetooth is on, for headphones and speakers.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::OwnScreen, deepest: Scope::Global },
        SettingDecl { key: WIFI_KEEP_ON, section: "Developer", label: "Keep Wi-Fi on",
            help: "Keeps Wi-Fi on while it is set to \"auto\", so the device can still be reached over Wi-Fi (for SSH, or copying files).",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global },
        SettingDecl { key: FRAME_STATS, section: "Developer", label: "Frame statistics",
            help: "Shows, over the game, its frame rate, how long the emulator spends on each frame, and how much sound is waiting to be played: for finding what causes slowdowns or crackling sound.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global },
    ]
}
