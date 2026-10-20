//! Specular's own settings. Only what people genuinely disagree on is a user setting; the rest
//! have fixed defaults, or are developer settings hidden behind "Developer details".

use serde_json::json;

use super::{leak, Scope, SettingDecl, SettingKind, Visibility};
use crate::cpu;
use crate::library::{Autosave, CoreDecl};

pub const BRIGHTNESS: &str = "display.brightness";
/// How a game's picture fills the screen: "integer" (the largest whole multiple of its size),
/// "fit" (as large as fits, keeping its shape) or "stretch" (the whole screen).
pub const SCALING: &str = "video.scaling";
/// The speaker's volume, 0 (silent) to `VOLUME_MAX`. The volume keys step the volume of whichever
/// output sound goes to (see `device::Output::volume_setting`).
pub const VOLUME: &str = "sound.volume";
pub const VOLUME_MAX: i64 = 20;
/// The volume of headphones in the device's jack, 0 to `VOLUME_MAX`. Unused on a device that
/// can't tell when headphones are in: they get `VOLUME`.
pub const HEADPHONE_VOLUME: &str = "sound.headphone_volume";
/// The volume of Bluetooth headphones and speakers (all of them share it), 0 to `VOLUME_MAX`.
pub const BLUETOOTH_VOLUME: &str = "sound.bluetooth_volume";
/// Whether the interface makes its sounds (`app::Sound`).
pub const INTERFACE_SOUNDS: &str = "sound.interface_sounds";
/// WiFi: "on", "off", or "auto" (on only while something needs it: see `app::WifiNeed`). Shown
/// on the Wi-Fi screen.
pub const WIFI: &str = "wireless.wifi";
/// Bluetooth on or off. Shown on the Bluetooth screen.
pub const BLUETOOTH: &str = "wireless.bluetooth";
/// WiFi set to Auto stays on (a developer setting: SSH registers no `app::WifiNeed`, so on Auto
/// WiFi would otherwise turn off while it is in use).
pub const WIFI_KEEP_ON: &str = "dev.wifi_keep_on";
/// The LEDs' brightness (its key predates there being more LEDs than the light bar).
pub const LED_BRIGHTNESS: &str = "lights.brightness";
/// The LEDs' brightness steps (0 = off).
pub const LED_BRIGHTNESS_MAX: i64 = 5;
/// The LEDs and the on-screen light bar pulse in the warning colour as the battery becomes low
/// (`effects::LowBattery`).
pub const LOW_BATTERY_LIGHTS: &str = "lights.low_battery";
/// The LEDs and the on-screen light bar flash as a game saves (`effects::SaveFlash`).
pub const SAVE_FLASH: &str = "lights.save_flash";
/// How strongly the device rumbles: in `RUMBLE_MAX`ths of what the game or an effect asks (0 =
/// never; see `rumble::RumbleLine`).
pub const RUMBLE: &str = "rumble.strength";
/// The rumble strength's top step: as strongly as asked.
pub const RUMBLE_MAX: i64 = 5;
/// The device rumbles with the game's loud sounds (`effects::FollowSound`).
pub const RUMBLE_FOLLOWS_SOUND: &str = "rumble.follow_sound";
/// The accent (a palette id): the on-screen light bar, toggles and unfocused gauges, and the LEDs
/// the device profile marks `Accent::First`.
pub const ACCENT: &str = "appearance.accent";
/// The second accent (a palette id, or `SAME`): the focus marks, and the LEDs marked
/// `Accent::Second`.
pub const SECOND_ACCENT: &str = "appearance.second_accent";
/// Whether the light bar and the LEDs showing the first accent take each system's colour on its
/// game list and while its games are played (see `App::tint`).
pub const TINT_BY_SYSTEM: &str = "appearance.tint_by_system";
/// Sleep after this many seconds without a button press, outside a running game ("never" for
/// never).
pub const SLEEP_AFTER: &str = "power.sleep_after_seconds";
/// How a system's games are shown: "list" (beside the focused game's picture) or "grid".
pub const LIBRARY_VIEW: &str = "library.view";
/// What a system's games are shown with: an `art::ArtKind`'s id.
pub const ART: &str = "library.art";
/// Shows the game's frame rate and the core's time per frame over the game (a developer setting).
pub const FRAME_STATS: &str = "dev.frame_stats";
/// How the time zone is chosen: "auto" (looked up from the network, once `TIME_ZONE_LOOKUP`
/// allows it) or "manual" (picked from a list). The zone itself is kept by the device. Shown on
/// the Time zone screen.
pub const TIME_ZONE: &str = "time.zone";
/// Whether the user allowed telling the lookup service (ipinfo.io) the network's public address,
/// to find the time zone (see `app::timezone`).
pub const TIME_ZONE_LOOKUP: &str = "time.zone_lookup_allowed";
/// The CPU's lowest speed while a game plays: `cpu::AUTO` (chosen from its frames, from its core's
/// start level: `cpu::CpuMin`), `cpu::OFF` (the system's choice), or a speed in MHz (one of the
/// Brick's steps). Only in the config files.
pub const CPU_MIN: &str = "power.cpu_min";
/// When a game is autosaved during play: `AUTOSAVE_CORE` (its core's `CoreDecl::autosave`),
/// `AUTOSAVE_EVERY_10S` or `AUTOSAVE_AT_PAUSES` (see `library::Autosave`). Only in the config
/// files.
pub const AUTOSAVE: &str = "game.autosave";
// `AUTOSAVE`'s values.
pub const AUTOSAVE_CORE: &str = "core default";
pub const AUTOSAVE_EVERY_10S: &str = "every 10 s";
pub const AUTOSAVE_AT_PAUSES: &str = "at pauses";
/// Shows the settings and details meant for developers.
pub const DEVELOPER_DETAILS: &str = "ui.developer_details";

pub fn settings() -> Vec<SettingDecl> {
    vec![
        SettingDecl { key: BRIGHTNESS, section: "Display", label: "Brightness",
            help: "How bright the screen is, from 1 (dimmest) to 10 (brightest).",
            kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" }, default: json!(6),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: SCALING, section: "Display", label: "Scaling",
            help: "How a game's picture fills the screen. Integer: as large as fits at a whole multiple of the game's own size, for the sharpest pixels. Fit: as large as fits, keeping its shape. Stretch: the whole screen.",
            kind: SettingKind::Choice(&[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")]), default: json!("integer"),
            visibility: Visibility::User, deepest: Scope::Game, core: None, system: None },
        SettingDecl { key: VOLUME, section: "Sound", label: "Speaker volume",
            help: "The volume of the device's own speaker, from 0 (silent) to 20. The volume keys change it while the speaker plays.",
            kind: SettingKind::Int { min: 0, max: VOLUME_MAX, step: 1, unit: "" }, default: json!(12),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: HEADPHONE_VOLUME, section: "Sound", label: "Headphone volume",
            help: "The volume of headphones in the jack, from 0 (silent) to 20. The volume keys change it while headphones are in.",
            kind: SettingKind::Int { min: 0, max: VOLUME_MAX, step: 1, unit: "" }, default: json!(10),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: BLUETOOTH_VOLUME, section: "Sound", label: "Bluetooth volume",
            help: "The volume of Bluetooth headphones and speakers (all of them share it), from 0 (silent) to 20. The volume keys change it while sound goes to one.",
            kind: SettingKind::Int { min: 0, max: VOLUME_MAX, step: 1, unit: "" }, default: json!(10),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: INTERFACE_SOUNDS, section: "Sound", label: "Interface sounds",
            help: "Plays a quiet tick as the selection moves, and short tones as screens open and close and switches turn on and off. They are never played over a game, even while it is paused.",
            kind: SettingKind::Bool, default: json!(true), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: LED_BRIGHTNESS, section: "Lights", label: "Brightness",
            help: "How bright the device's lights are, from 0 (off) to 5 (brightest).",
            kind: SettingKind::Int { min: 0, max: LED_BRIGHTNESS_MAX, step: 1, unit: "" }, default: json!(3),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: LOW_BATTERY_LIGHTS, section: "Lights", label: "Low battery warning",
            help: "Whether the device's lights, and the bar along the top of the screen, pulse red for a moment as the battery becomes low.",
            kind: SettingKind::Bool, default: json!(true), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: SAVE_FLASH, section: "Lights", label: "Flash when the game saves",
            help: "Whether the device's lights, and the bar along the top of the screen, flash for a moment when the game saves, or a save state is made.",
            kind: SettingKind::Bool, default: json!(true), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: RUMBLE, section: "Rumble", label: "Strength",
            help: "How strongly the device rumbles when a game makes it, from 0 (never) to 5 (as strongly as the game asks).",
            kind: SettingKind::Int { min: 0, max: RUMBLE_MAX, step: 1, unit: "" }, default: json!(RUMBLE_MAX),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: RUMBLE_FOLLOWS_SOUND, section: "Rumble", label: "Follow the sound",
            help: "Whether the device also rumbles with the game's loud sounds, such as explosions and drums, even in games that don't rumble.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: ACCENT, section: "Appearance", label: "Accent",
            help: "The interface's main colour, used for the glowing bar along the top of the screen, switches, gauges that aren't selected, and some of the device's lights.",
            kind: SettingKind::Colour { same_as: None }, default: json!("teal"),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: SECOND_ACCENT, section: "Appearance", label: "Second accent",
            help: "The colour that marks what is selected, and of the rest of the device's lights. Choosing Same as accent uses the main accent colour for both.",
            kind: SettingKind::Colour { same_as: Some(ACCENT) }, default: json!("coral"),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: TINT_BY_SYSTEM, section: "Appearance", label: "Tint by system",
            help: "Whether the glowing bar along the top of the screen, and the lights that show the main accent colour, take each system's own colour while its games are listed or one of them is played.",
            kind: SettingKind::Bool, default: json!(true), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: LIBRARY_VIEW, section: "Library", label: "Games shown as",
            help: "How a system's games are shown. List: a list, with the selected game's picture beside it. Grid: their pictures side by side. The Art setting chooses which picture.",
            kind: SettingKind::Choice(&[("list", "List"), ("grid", "Grid")]), default: json!("list"),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: ART, section: "Library", label: "Art",
            help: "Which picture games are shown with. Box art: the game's box; to use a picture of your own, put a .png, .jpg or .jpeg with the game's file name next to the game. Title screen: the game's title screen. Screenshot: a moment of play. Where you left off: the screen as you last quit the game. A game without that picture shows its box art, else a cover made from its name. Box art, title screens and screenshots are downloaded while Wi-Fi is connected.",
            kind: SettingKind::Choice(&[("boxart", "Box art"), ("title", "Title screen"), ("screenshot", "Screenshot"), ("left-off", "Where you left off")]), default: json!("boxart"),
            visibility: Visibility::User, deepest: Scope::System, core: None, system: None },
        SettingDecl { key: SLEEP_AFTER, section: "Power", label: "Sleep after",
            help: "How long the device waits, with no button pressed, before it goes to sleep. It never goes to sleep by itself while a game is being played, while File transfer is open, or while a computer is connected by USB cable.",
            kind: SettingKind::Choice(&[("never", "Never"), ("30", "30 sec"), ("60", "1 min"), ("180", "3 min"), ("300", "5 min"), ("600", "10 min")]), default: json!("30"),
            visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: CPU_MIN, section: "Power", label: "Lowest CPU speed while playing",
            help: "The lowest speed the processor may run at while a game plays (in the menus, and while a game is paused, the system chooses). Automatic: Specular chooses, starting at a speed set for each emulator (given at the end), and going a step faster each time the game can't keep up; it never slows down again until the game is closed. Off: the system chooses. A speed: at least that, for the whole game; the speeds listed are the TrimUI Brick's, and other devices use the nearest faster speed. The processor can still slow down if it gets too hot.",
            kind: SettingKind::Choice(&[(cpu::AUTO, "Automatic"), (cpu::OFF, "Off"), ("408", "408 MHz"), ("600", "600 MHz"), ("816", "816 MHz"),
                ("1008", "1008 MHz"), ("1200", "1200 MHz"), ("1416", "1416 MHz"), ("1608", "1608 MHz"), ("1800", "1800 MHz"), ("2000", "2000 MHz")]),
            default: json!(cpu::AUTO), visibility: Visibility::ConfigFiles, deepest: Scope::Game, core: None, system: None },
        SettingDecl { key: AUTOSAVE, section: "Saves", label: "Autosave during play",
            help: "When a game is saved automatically, so that it resumes where it was left. A game is always saved as its menu opens, before the device sleeps and when it is quit. Every 10 s: also every 10 seconds of play. At pauses: only then, as saving can make a game stutter for a moment (PlayStation games do); a crash, a hang or a failing SD card then loses the play since the game was last paused. Core default: the emulator's own choice (given at the end).",
            kind: SettingKind::Choice(&[(AUTOSAVE_CORE, "Core default"), (AUTOSAVE_EVERY_10S, "Every 10 s"), (AUTOSAVE_AT_PAUSES, "At pauses")]),
            default: json!(AUTOSAVE_CORE), visibility: Visibility::ConfigFiles, deepest: Scope::Game, core: None, system: None },
        SettingDecl { key: DEVELOPER_DETAILS, section: "System", label: "Developer details",
            help: "Also shows the settings and technical details meant for developers.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: WIFI, section: "Wireless", label: "Wi-Fi",
            help: "Whether Wi-Fi is on. Auto turns it on only while something needs it, such as downloading box art.",
            kind: SettingKind::Choice(&[("auto", "Auto"), ("on", "On"), ("off", "Off")]), default: json!("auto"),
            visibility: Visibility::OwnScreen, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: BLUETOOTH, section: "Wireless", label: "Bluetooth",
            help: "Whether Bluetooth is on, for headphones and speakers.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::OwnScreen, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: TIME_ZONE, section: "Time", label: "Set time zone",
            help: "How the time zone is set. Automatically: it is looked up each time Wi-Fi connects, once Look up the time zone allows it. By hand: the zone chosen in Settings, under Time zone, stays.",
            kind: SettingKind::Choice(&[("auto", "Automatically"), ("manual", "By hand")]), default: json!("auto"),
            visibility: Visibility::OwnScreen, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: TIME_ZONE_LOOKUP, section: "Time", label: "Look up the time zone",
            help: "Whether the time zone may be looked up on the internet, from ipinfo.io, which then sees the network's public IP address. Specular asks before the first lookup.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::OwnScreen, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: WIFI_KEEP_ON, section: "Developer", label: "Keep Wi-Fi on",
            help: "Keeps Wi-Fi on while it is set to Auto, so the device can still be reached over Wi-Fi (for SSH, or copying files).",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: FRAME_STATS, section: "Developer", label: "Frame statistics",
            help: "Shows, over the game, its frame rate, how long the emulator spends on each frame, and how much sound is waiting to be played: for finding what causes slowdowns or crackling sound.",
            kind: SettingKind::Bool, default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global, core: None, system: None },
    ]
}

/// For the settings whose default depends on the core (`CPU_MIN`'s start speed, `AUTOSAVE`'s "core
/// default"), appends to their help what that value is for each of `cores`, naming values by their
/// labels.
pub fn describe_cores(decls: &mut [SettingDecl], cores: &[&CoreDecl]) {
    for d in decls {
        let SettingKind::Choice(options) = d.kind else { continue };
        let label = |value: &str| options.iter().find(|(v, _)| *v == value).map_or(value.to_string(), |(_, l)| l.to_string());
        let (value, verb, each) = match d.key {
            CPU_MIN => (cpu::AUTO, "starts at", by_core(cores, |c| c.cpu_start.map(|mhz| format!("{mhz} MHz")), "the speed the system chooses")),
            AUTOSAVE => (AUTOSAVE_CORE, "is", by_core(cores, |c| match c.autosave { Autosave::AtPauses => Some(label(AUTOSAVE_AT_PAUSES)), Autosave::Every10s => None },
                &label(AUTOSAVE_EVERY_10S))),
            _ => continue,
        };
        d.help = leak(format!("{} {} {verb} {each}.", d.help, label(value)));
    }
}

/// Text such as "A for SwanStation; B for the others": the value `special` returns for each core it
/// returns one for, then `usual` for the rest.
fn by_core(cores: &[&CoreDecl], special: impl Fn(&CoreDecl) -> Option<String>, usual: &str) -> String {
    let mut each: Vec<String> = cores.iter().filter_map(|c| special(c).map(|v| format!("{v} for {}", c.name))).collect();
    if each.len() < cores.len() { each.push(format!("{usual} for {}", if each.is_empty() { "all" } else { "the others" })); }
    each.join("; ")
}
