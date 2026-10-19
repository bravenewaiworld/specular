//! The app: everything the user can do, as a state machine. Devices and the UI feed it messages
//! (buttons, time passing, scan results) and carry out the effects it returns (save a file, set
//! the brightness, launch a game). It never touches files, hardware or the UI itself, so all of
//! its behaviour is testable.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use crate::art::{self, ArtKind};
use crate::card::{Card, CardCmd};
use crate::device::{Battery, Output, Rgb, SshCmd, SshStatus};
use crate::input::{Button, ButtonEvent, Press};
use crate::keyboard::{KeyOutcome, Keyboard};
use crate::library::{self, Game, SystemDecl};
use crate::list;
use crate::model::{Icon, ImageRef, Layout, Row, RowKind, ScreenId, ScreenView, TextEntry};
use crate::nav::{NavStack, ScreenState, Transition};
use crate::repeat::Repeater;
use crate::settings::{Layer, SettingDecl, Settings, Visibility, builtin};
use crate::timezone::{ZoneCmd, ZoneEvent};
use crate::wireless::bluetooth::{BtCmd, BtEvent};
use crate::wireless::wifi::{WifiCmd, WifiEvent};

mod card;
mod cores;
mod credits;
mod icons;
mod install;
mod layers;
mod ssh;
mod transfer;
mod timezone;
mod wireless;
pub use timezone::TimeZoneState;
pub use wireless::{BtState, WifiNeed, WifiState};
pub use install::{InstallCmd, InstallState, TaskView};
pub use ssh::SshState;
pub use transfer::{Server, TransferState};
pub use credits::{Credit, CreditKind, Credits, Splash};

/// Something that happened.
#[derive(Debug)]
pub enum Msg {
    Button(ButtonEvent),
    /// The current time, on the clock button times use: due button repeats are generated, and
    /// the device sleeps if it has been left alone for the "Sleep after" time. Sent when a repeat
    /// or the sleep is due (`App::next_deadline`, `App::sleep_deadline`), and after `Woke`,
    /// `GameEnded`, `GameStopped` and at startup: the idle count starts from the first tick after
    /// those.
    Tick(Duration),
    /// A library scan (`Effect::ScanLibrary`) finished, or rescanned after the games changed.
    /// Ignored while the card can't be read.
    Library(Vec<Game>),
    /// The cores changed (cores the user added were found or removed, or one's options learned):
    /// the systems Specular plays now, with their cores, every setting with those cores' options
    /// (`settings::declarations`), and the files in `cores/` turned away. Sent before the
    /// `Library` of the scan that used these systems.
    Cores { systems: Vec<SystemDecl>, settings: Vec<SettingDecl>, turned_away: Vec<library::user_cores::TurnedAway> },
    /// The SD card's state, as the device reports it: at startup, then on each change (see
    /// `app::card`).
    Card(Card),
    /// A write to the SD card failed because it is read-only (EROFS).
    CardReadOnly,
    Battery(Battery),
    /// The device woke from sleep.
    Woke,
    /// The game launched couldn't start, and why. (None, a game that ended by itself, isn't sent
    /// yet.)
    GameEnded(Option<String>),
    /// The running game can't go on (its core's built-in BIOS halted it), with the reason for the
    /// player. The app quits the game and shows the reason; with no game playing, it is ignored.
    GameStopped(String),
    /// At start-up: the game last played didn't end (its launch record was left behind).
    LastGameStopped(crate::launch::LaunchRecord),
    /// A settings layer was read from its file.
    SettingsLoaded(Layer, serde_json::Map<String, serde_json::Value>),
    Wifi(WifiEvent),
    Bluetooth(BtEvent),
    /// Something needs WiFi (`wanted`) or no longer does, at `time` (on the clock `Tick` uses).
    /// Only matters while WiFi is on Auto.
    WifiNeed { need: WifiNeed, wanted: bool, time: Duration },
    /// Sound couldn't be played on this output (it didn't open): the device's own plays it
    /// instead, until the Bluetooth device reconnects.
    SoundFailed(Output),
    /// Headphones are plugged into the device's jack (true) or not: sent by a device that can
    /// tell, with the state when it starts following the jack, then on each change.
    Headphones(bool),
    /// The installer's answer to `InstallCmd::Offer`.
    InstallOffer(Result<crate::install::Offer, String>),
    /// The progress of the task started with `InstallCmd::Start`.
    InstallProgress(crate::install::Progress),
    /// A start (`start` true) or stop of the file transfer server, asked for with
    /// `Effect::FileTransfer`, has finished; Err: why it failed.
    FileTransfer { start: bool, result: Result<(), String> },
    /// `Effect::Ssh`'s command finished: SSH's state now, or why the command failed.
    Ssh { cmd: SshCmd, result: Result<SshStatus, String> },
    /// The running game's discs, as its core says: sent once it has started, and after a state is
    /// loaded or a disc changed. Ignored while no game runs.
    Discs(Discs),
    /// What the UI's time zone thread reports: the zone and the list at start, then the answers
    /// to `Effect::TimeZone`.
    TimeZone(ZoneEvent),
}

/// A game's discs, for the game menu's Disc row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Discs {
    /// 0 until the core says, and for a core that can't change discs; the Disc row shows from 2.
    pub count: usize,
    /// The disc in the drive (from 0, below `count`); None when none is (its tray is open).
    pub current: Option<usize>,
}

/// Something the app wants done, in the order given.
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// The screen changed; draw it again (with this transition, if the screen itself changed).
    Redraw(Option<Transition>),
    /// Write this settings layer to its file.
    SaveSettings(Layer),
    /// Read the SD card afresh: its settings files (`Msg::SettingsLoaded`, the global one first),
    /// then its cores (`Msg::Cores`) and games (`Msg::Library`), watching its folders anew
    /// (watches on a card that was released never fire again) and reading them again whenever
    /// they change. Nothing from an earlier reading may arrive after this. Only a `writable`
    /// card has its config folder brought up to date and files Specular generates written.
    ReadCard { writable: bool },
    /// Stop reading and watching the card: it is gone. Nothing from its reading may arrive after
    /// this.
    ForgetCard,
    /// One of Specular's settings has this value where it applies now: at the running game's layer
    /// while one is played, else the global one. Sent for every setting once the global settings
    /// have loaded, for one changed, and for those that may differ per system or game when a game
    /// starts. The UI carries out the ones the device or the UI acts on, such as brightness or
    /// scaling, and ignores the rest.
    Apply { key: &'static str, value: serde_json::Value },
    /// Start playing a game, its core's options set as `App::core_options` says.
    Launch(Game),
    /// One of the running game's core options changed. This is the full set of values to give its
    /// core now (`App::core_options`); options left out go back to the core's defaults.
    CoreOptions(Vec<(String, String)>),
    /// Pause (true) or resume the running game (its in-game menu opened or closed).
    PauseGame(bool),
    /// Save the running game's state, or load the state saved last.
    SaveState,
    LoadState,
    /// Put this disc (from 0) in the running game's drive.
    ChangeDisc(usize),
    ResetGame,
    /// End the running game (its battery save is written).
    QuitGame,
    /// End the running game without writing its saves: the card they go to is gone or read-only.
    AbandonGame,
    /// Show this short message for a moment.
    Notice(String),
    /// Show the volume: its level (0 to `builtin::VOLUME_MAX`), whether the sound is muted, and
    /// the output it is the volume of. Sent on every volume key press (even at an end) and
    /// whenever the mute switch moves.
    ShowVolume { level: i64, muted: bool, output: Output },
    /// Show this letter of the letter index for a moment: L2 or R2 jumped to it, or stayed on it
    /// (held at the first or last letter, or a list of one letter).
    ShowLetter(char),
    /// Silence the sound (true) or restore it (false). The volume level is untouched; `Apply`
    /// still carries it.
    Mute(bool),
    /// Sleep. A running game is paused first, and its saves written if the card can be written
    /// (`Ctx::card`): power can fail while asleep.
    Sleep,
    /// Turn the device off. If a game is being played, the app sends `QuitGame` just before this
    /// (in the same list), so its saves are written first (`AbandonGame` if the card can't be
    /// written).
    PowerOff,
    /// Restart the device; as `PowerOff`, after ending a game being played.
    Restart,
    Wifi(WifiCmd),
    Bluetooth(BtCmd),
    /// Sound goes to this output from now on, including a running game's.
    SoundTo(Output),
    Installer(InstallCmd),
    /// Start (true) or stop the file transfer server; the outcome comes back as
    /// `Msg::FileTransfer`. When a stop and a `Sleep` are in one list, the stop comes first:
    /// carried out in order, the server is stopped before the device sleeps.
    FileTransfer(bool),
    /// Carry out this SSH command; the result comes back as `Msg::Ssh`.
    Ssh(SshCmd),
    /// Play this interface sound. Only sent while they can be heard: never while a game is being
    /// played (it has the sound output, also while paused), muted, at volume 0 or with "Interface
    /// sounds" off.
    Sound(Sound),
    /// A command for the UI's time zone thread. The UI redraws the clock once a zone is set.
    TimeZone(ZoneCmd),
    Card(CardCmd),
}

/// The interface's sounds (made in specular-ui; see docs/look-and-motion.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    /// The focus moved, or a value stepped. At most one per `TICK_GAP`.
    Move,
    /// A screen opened. Starting a game makes no sound: the game would have to wait for the
    /// interface sounds' output (maybe a slow Bluetooth one) to open and close before opening its
    /// own.
    Open,
    /// A screen closed.
    Back,
    /// A switch turned on (true) or off.
    Toggle(bool),
}

/// `Sound::Move`s closer together than this are dropped, so a held direction (up to about 30
/// moves a second) ticks about 15 times a second rather than buzzing.
pub const TICK_GAP: Duration = Duration::from_millis(60);

/// Holding the power button this long opens the power menu; a shorter press sleeps.
pub const POWER_HOLD: Duration = Duration::from_secs(1);

/// Who a button press is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputTarget {
    /// The app's screens.
    App,
    /// The running game: its joypad's buttons go to it; the app keeps the others (Menu, Power,
    /// the volume keys). Never while the game menu or the power menu is open over the game.
    Game,
}

/// What a screen needs to build its view and act on presses.
pub struct Ctx {
    pub systems: Vec<SystemDecl>,
    pub games: Vec<Game>,
    /// Whether this card's library has been scanned yet (until then `games` is empty).
    pub scanned: bool,
    /// The SD card, once the device has reported it.
    pub card: Option<Card>,
    /// Whether home still shows the card's one-time message (errors were repaired) until the
    /// user dismisses it.
    pub card_note: bool,
    pub settings: Settings,
    pub battery: Option<Battery>,
    /// None on a device without WiFi (or Bluetooth); see `App::has_radios`.
    pub wifi: Option<WifiState>,
    pub bluetooth: Option<BtState>,
    /// None on a device without an installer; see `App::has_installer`.
    pub install: Option<InstallState>,
    /// None on a device without file transfer; see `App::has_file_transfer`.
    pub transfer: Option<TransferState>,
    /// None on a device where the app doesn't manage SSH; see `App::has_ssh`.
    pub ssh: Option<SshState>,
    pub credits: Credits,
    /// The files in the card's `cores/` that aren't used, and why (see `Msg::Cores`).
    pub turned_away: Vec<library::user_cores::TurnedAway>,
    /// The running game's discs (`Msg::Discs`); none until its core says.
    pub discs: Discs,
    /// None on a device whose time zone can't be set; see `App::has_time_zone`.
    pub time_zone: Option<TimeZoneState>,
}

impl Ctx {
    /// Whether the SD card is in and its files can be read (it is mounted, maybe read-only).
    pub fn card_in(&self) -> bool { self.card.as_ref().is_some_and(Card::readable) }

    /// Whether the SD card can be written to (saves, settings).
    pub fn card_writable(&self) -> bool { self.card.as_ref().is_some_and(Card::writable) }

    /// Which settings are shown: developer ones too while "Developer details" is on.
    pub fn visibility(&self) -> Visibility {
        if self.settings.on(builtin::DEVELOPER_DETAILS) { Visibility::Developer } else { Visibility::User }
    }
}

/// What a press on a screen leads to.
pub enum Outcome {
    /// Nothing on the screen changed (the built screen is kept).
    None,
    /// The screen's content changed; draw it again.
    Changed,
    Push(ScreenState),
    Back,
    /// Go back, then carry these out.
    BackWith(Vec<Effect>),
    /// Open the keyboard to type text for this screen, which gets it in `Screen::entered`, with
    /// `purpose` (say, the network the password is for).
    Type { prompt: String, masked: bool, purpose: String },
    /// Start playing this game.
    Launch(Game),
    /// Carry these out (include `Effect::Redraw` if the screen changed too).
    Effects(Vec<Effect>),
    /// This setting was changed (or reset) at this layer: the app saves the layer, and applies
    /// the setting where it takes effect (`Effect::Apply`, `Effect::CoreOptions`).
    SettingChanged(Layer, &'static str),
}

/// One kind of screen. Built-in screens implement it; plugins will register their own.
pub trait Screen {
    /// The screen's rows. The app builds this once and keeps it until the library, the settings
    /// or the screen shown change, or a press returns `Changed` or `Effects`; so it needn't be
    /// cheap, but it may depend only on `ctx.systems`, `ctx.games`, `ctx.settings` and
    /// `state.params` (not on the battery or the focus).
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView;
    /// Text typed on the keyboard this screen opened (`Outcome::Type`) was confirmed. Ok closes
    /// the keyboard and carries out the outcome as if it came from a press, except that an
    /// `Outcome::Push` replaces the keyboard; Err keeps the keyboard open and shows the message.
    fn entered(&self, _ctx: &mut Ctx, _state: &mut ScreenState, _purpose: &str, _text: String) -> Result<Outcome, String> { Ok(Outcome::None) }
    /// Whether L2 and R2 jump between the first letters of the rows' labels (`list::jump`,
    /// `library::index_letter`), for a list sorted by title.
    fn letter_index(&self) -> bool { false }
    /// Handles a press the app didn't take itself (up, down, L1 and R1 move the focus; so do left
    /// and right in a grid, and L2 and R2 with a `letter_index`; Menu goes home). Screens never
    /// see the power button: the app handles it (a tap sleeps, holding it opens the power menu).
    /// `focused` is the focused row, if the screen has one that can take presses (never a header
    /// or a disabled row).
    fn press(&self, ctx: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome;
}

/// The built-in screens' ids.
pub mod screens {
    use super::ScreenId;
    pub const HOME: ScreenId = ScreenId(std::borrow::Cow::Borrowed("home"));
    pub const GAMES: ScreenId = ScreenId(std::borrow::Cow::Borrowed("games"));
    /// The games matching a search, with the query in its params.
    pub const SEARCH: ScreenId = ScreenId(std::borrow::Cow::Borrowed("search"));
    pub const SETTINGS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("settings"));
    /// Over a paused game. Its presses are handled by the app itself (see `App::game_menu`).
    pub const GAME_MENU: ScreenId = ScreenId(std::borrow::Cow::Borrowed("game menu"));
    /// Power off or restart, opened by holding the power button (over the game menu during a
    /// game). Its presses are handled by the app itself (see `App::power_menu`).
    pub const POWER_MENU: ScreenId = ScreenId(std::borrow::Cow::Borrowed("power menu"));
    pub const WIFI: ScreenId = ScreenId(std::borrow::Cow::Borrowed("wifi"));
    /// Every icon in each of its states; a developer screen.
    pub const ICONS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("icons"));
    /// Every core, Specular's and those on the SD card (and the files there turned away); a
    /// developer screen.
    pub const CORES: ScreenId = ScreenId(std::borrow::Cow::Borrowed("cores"));
    pub const BLUETOOTH: ScreenId = ScreenId(std::borrow::Cow::Borrowed("bluetooth"));
    /// "Forget …?", with what to forget in its params (see `app::wireless::Forget`).
    pub const FORGET: ScreenId = ScreenId(std::borrow::Cow::Borrowed("forget"));
    /// The on-screen keyboard, for the screen below it. Its presses are handled by the app itself
    /// (see `App::keyboard`).
    pub const KEYBOARD: ScreenId = ScreenId(std::borrow::Cow::Borrowed("keyboard"));
    // The installer's screens (see `app::install`). The app handles presses on those that start a
    // task, and on the progress screen (`App::install_press`).
    /// "Install to internal storage?".
    pub const INSTALL: ScreenId = ScreenId(std::borrow::Cow::Borrowed("install"));
    /// The backups on the SD card.
    pub const RESTORE: ScreenId = ScreenId(std::borrow::Cow::Borrowed("restore"));
    /// The update files on the SD card.
    pub const UPDATES: ScreenId = ScreenId(std::borrow::Cow::Borrowed("updates"));
    /// Asks before a restore, an update or starting the previous version; its params name the task
    /// (`restore:<dir>`, `update:<file>`, `activate:<version>`).
    pub const CONFIRM: ScreenId = ScreenId(std::borrow::Cow::Borrowed("install confirm"));
    /// The question about a restore prepared from the installed system.
    pub const RESTORE_PENDING: ScreenId = ScreenId(std::borrow::Cow::Borrowed("restore pending"));
    /// A task's progress.
    pub const TASK: ScreenId = ScreenId(std::borrow::Cow::Borrowed("task"));
    pub const CREDITS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("credits"));
    /// One game's settings, with its path under `roms/` in its params; also opened from the in-game
    /// menu, over the paused game.
    pub const GAME_SETTINGS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("game settings"));
    /// The settings of all of one system's games, with the system's id in its params.
    pub const SYSTEM_SETTINGS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("system settings"));
    /// While it is open, PCs can read and write the SD card (see `app::transfer`).
    pub const FILE_TRANSFER: ScreenId = ScreenId(std::borrow::Cow::Borrowed("file transfer"));
    /// Settings → Time zone (see `app::timezone`).
    pub const TIME_ZONE: ScreenId = ScreenId(std::borrow::Cow::Borrowed("time zone"));
    /// The zone picker's first screen: zones with no area, then the areas. A on a zone is
    /// handled by the app (`App::pick_zone`).
    pub const ZONE_AREAS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("time zone areas"));
    /// One area's zones, the area in its params. A on a zone is handled by the app.
    pub const ZONE_CITIES: ScreenId = ScreenId(std::borrow::Cow::Borrowed("time zone cities"));
    /// The question whether the zone may be looked up. Its presses are handled by the app
    /// (`App::zone_consent`).
    pub const ZONE_CONSENT: ScreenId = ScreenId(std::borrow::Cow::Borrowed("time zone consent"));
    /// Over a game whose SD card went, until the card is back or the game is quit; its params
    /// hold the id of the card the game runs from (see `app::card`). Its presses are handled by
    /// the app itself.
    pub const CARD_WAIT: ScreenId = ScreenId(std::borrow::Cow::Borrowed("card wait"));
}

/// Makes a `ScreenId` from a name (for screens registered later).
pub fn screen_id(name: impl Into<Cow<'static, str>>) -> ScreenId { ScreenId(name.into()) }

/// The app's state: the screens, where the user is, and what the screens show.
pub struct App {
    ctx: Ctx,
    screens: HashMap<ScreenId, Box<dyn Screen>>,
    nav: NavStack,
    repeater: Repeater,
    /// When the buttons were last used, for "Sleep after". None means the idle count hasn't
    /// started: the next `Tick` starts it. Not consulted while a game is running (its buttons
    /// never reach the app); the game menu counts as idle.
    idle_since: Option<Duration>,
    /// Whether the mute switch is on. Not saved: the device reports the switch when it opens.
    muted: bool,
    /// Whether headphones are in the jack, as the device last said (`Msg::Headphones`).
    headphones: bool,
    /// When the power button went down, while that press can still be a tap (a release before
    /// `POWER_HOLD` sleeps). None once it has been held long enough to open the power menu, so
    /// its release does nothing; also None for a press the app never saw (the one that woke the
    /// device is dropped by the UI).
    power_down: Option<Duration>,
    /// The game being played (whether or not its menu is open).
    playing: Option<Game>,
    /// How many columns a grid has on this display (from the UI; see `list::grid_columns`).
    grid_columns: usize,
    /// The current screen as last built, and its parent's title; None when they must be built
    /// again.
    cache: RefCell<Option<(Rc<ScreenView>, Option<String>)>>,
    /// The keyboard while `screens::KEYBOARD` is open, and the purpose it was opened with.
    keyboard: Option<(Keyboard, String)>,
    /// The latest time seen (`Tick`, button and `WifiNeed` times).
    now: Duration,
    /// Whether the global settings (the card's, once it is read) have been loaded. Until then nothing is sent to the radios and
    /// the time zone isn't looked up, since the settings decide both.
    settings_loaded: bool,
    /// What the radios and sound were last told (see `wireless::Radios`).
    radios: wireless::Radios,
    /// The connection the time zone was last looked up for (see `App::reconcile_time_zone`).
    zone_looked_up_for: Option<String>,
    /// When the last `Sound::Move` was sent (see `TICK_GAP`).
    last_tick: Option<Duration>,
    /// True from sending `Effect::Sleep` until `Msg::Woke`.
    asleep: bool,
    /// Whether a write that failed as read-only had the device look at the card again, and its
    /// answer (`Msg::Card`) hasn't come yet.
    rechecking_card: bool,
}

impl App {
    /// An app with the built-in screens, knowing these systems and settings. It starts at home
    /// with no games, and reads the library and settings once a `Msg::Card` says the card can be
    /// read.
    pub fn new(systems: Vec<SystemDecl>, settings: Vec<SettingDecl>) -> Self {
        let mut screens: HashMap<ScreenId, Box<dyn Screen>> = HashMap::new();
        screens.insert(screens::HOME, Box::new(Home));
        screens.insert(screens::GAMES, Box::new(Games));
        screens.insert(screens::SEARCH, Box::new(Search));
        screens.insert(screens::SETTINGS, Box::new(SettingsScreen));
        screens.insert(screens::GAME_MENU, Box::new(GameMenu));
        screens.insert(screens::POWER_MENU, Box::new(PowerMenu));
        wireless::register(&mut screens);
        icons::register(&mut screens);
        install::register(&mut screens);
        credits::register(&mut screens);
        cores::register(&mut screens);
        layers::register(&mut screens);
        transfer::register(&mut screens);
        timezone::register(&mut screens);
        card::register(&mut screens);
        Self {
            ctx: Ctx { systems, games: Vec::new(), scanned: false, card: None, card_note: false, settings: Settings::new(settings), battery: None,
                       wifi: None, bluetooth: None, install: None, transfer: None, ssh: None, credits: Credits::default(), turned_away: Vec::new(),
                       discs: Discs::default(), time_zone: None },
            screens,
            nav: NavStack::new(ScreenState::new(screens::HOME, None)),
            repeater: Repeater::default(),
            idle_since: None,
            muted: false,
            headphones: false,
            power_down: None,
            grid_columns: 1,
            playing: None,
            cache: RefCell::default(),
            keyboard: None,
            now: Duration::ZERO,
            settings_loaded: false,
            radios: wireless::Radios::default(),
            zone_looked_up_for: None,
            last_tick: None,
            asleep: false,
            rechecking_card: false,
        }
    }

    /// Adds a screen (or replaces a built-in one).
    pub fn register(&mut self, id: ScreenId, screen: Box<dyn Screen>) { self.screens.insert(id, screen); self.changed(); }

    /// Tells the app how many columns the UI's grids have, so up and down move by a whole row.
    pub fn set_grid_columns(&mut self, n: usize) { self.grid_columns = n.max(1); }

    /// The game being played, if any.
    pub fn playing(&self) -> Option<&Game> { self.playing.as_ref() }

    /// The colour the light bar and the LEDs showing the first accent take instead of the accent,
    /// while "Tint by system" is on: the system's, while one of its games is played (its menu
    /// open or not) or its game list is the current screen. None elsewhere, search results
    /// included (they span systems).
    pub fn tint(&self) -> Option<Rgb> {
        if !self.ctx.settings.on(builtin::TINT_BY_SYSTEM) { return None; }
        let top = self.nav.top();
        let system = match &self.playing {
            Some(game) => game.system,
            None if top.id == screens::GAMES => top.params.as_deref()?,
            None => return None,
        };
        crate::palette::find(self.ctx.systems.iter().find(|s| s.id == system)?.tint)
    }

    /// Who buttons are for now: the game while one is played and no menu is open over it (nor a
    /// screen opened from its menu).
    pub fn input_target(&self) -> InputTarget {
        let menu = self.nav.contains(&screens::GAME_MENU) || self.nav.top().id == screens::POWER_MENU || self.nav.contains(&screens::CARD_WAIT);
        if self.playing.is_some() && !menu { InputTarget::Game } else { InputTarget::App }
    }

    /// The running game's core options, as given to its core: those set in a settings layer, or
    /// given a default by Specular (`settings::core_options::values`). Empty with no game, or a
    /// system without a core.
    pub fn core_options(&self) -> Vec<(String, String)> {
        let Some((game, core)) = self.playing.as_ref().zip(self.playing_core()) else { return vec![] };
        crate::settings::core_options::values(&self.ctx.settings, core, &game.settings_layer())
    }

    /// The core that plays the running game: its system's, as its "Core" setting chooses
    /// (`settings::core_of`).
    pub fn playing_core(&self) -> Option<&'static library::CoreDecl> {
        let game = self.playing.as_ref()?;
        let system = self.ctx.systems.iter().find(|s| s.id == game.system)?;
        crate::settings::core_of(&self.ctx.settings, system, &game.settings_layer())
    }

    /// The layer settings are applied at now: the running game's, else the global one.
    fn applied_layer(&self) -> Layer { self.playing.as_ref().map_or(Layer::Global, Game::settings_layer) }

    /// The effect that applies `key`'s current value: `Apply` at `applied_layer` for one of
    /// Specular's settings, `CoreOptions` for an option of the running game's core; None for
    /// another core's option.
    fn apply(&self, key: &'static str) -> Option<Effect> {
        let s = &self.ctx.settings;
        match s.decl(key)?.core {
            None => s.get(key, &self.applied_layer()).map(|(v, _)| Effect::Apply { key, value: v.clone() }),
            Some(core) => (self.playing_core()?.id() == core).then(|| Effect::CoreOptions(self.core_options())),
        }
    }

    /// `apply` for each of Specular's own settings (not core options) for which `which` is true.
    fn apply_all(&self, which: impl Fn(&SettingDecl) -> bool) -> Vec<Effect> {
        self.ctx.settings.decls().iter().filter(|d| d.core.is_none() && which(d)).filter_map(|d| self.apply(d.key)).collect()
    }

    /// What the screens see (read-only: changes go through messages, so the cache stays right).
    pub fn ctx(&self) -> &Ctx { &self.ctx }

    /// The status bar's icons, left to right: WiFi and Bluetooth when on, then the battery (if
    /// the device reports one).
    pub fn status(&self) -> Vec<Icon> {
        let mut icons = wireless::status_icons(&self.ctx);
        icons.extend(self.ctx.battery.map(Icon::Battery));
        icons
    }

    /// The title of the screen below the current one (for a breadcrumb), if any.
    pub fn parent_title(&self) -> Option<String> { self.built().1 }

    /// When the next button repeat, or the power button's hold, is due, so the caller can wake up
    /// for it.
    pub fn next_deadline(&self) -> Option<Duration> {
        let hold = self.power_down.map(|t| t + POWER_HOLD);
        [self.repeater.next_deadline(), hold].into_iter().flatten().min()
    }

    /// When the device will sleep if left alone, so the caller can wake up for it. None if it
    /// won't (see `stays_awake`; or sleep is off, or the idle count hasn't started).
    pub fn sleep_deadline(&self) -> Option<Duration> {
        if self.stays_awake() { return None; }
        Some(self.idle_since? + self.sleep_after()?)
    }

    /// Whether the device mustn't sleep by itself now: a game is running unpaused, an installer
    /// task is running, or file transfer is open (a transfer may be under way).
    fn stays_awake(&self) -> bool { self.input_target() == InputTarget::Game || self.task_running() || self.transfer_open() }

    /// Sleeps, stopping file transfer first (it starts again on waking, if still open).
    fn sleep(&mut self) -> Vec<Effect> {
        self.asleep = true;
        let mut e = self.reconcile_transfer();
        e.push(Effect::Sleep);
        e
    }

    /// The "Sleep after" time; None for "never" (not a number).
    fn sleep_after(&self) -> Option<Duration> {
        let (v, _) = self.ctx.settings.get(builtin::SLEEP_AFTER, &Layer::Global)?;
        Some(Duration::from_secs(v.as_str()?.parse().ok()?))
    }

    /// Sleeps if nothing has been pressed for the "Sleep after" time. The idle count is cleared
    /// when sleeping, so `sleep_deadline` doesn't stay due while the device suspends.
    fn sleep_if_idle(&mut self, now: Duration) -> Vec<Effect> {
        if self.stays_awake() { return vec![]; }
        let since = *self.idle_since.get_or_insert(now);
        match self.sleep_after() {
            Some(after) if now.saturating_sub(since) >= after => {
                self.idle_since = None;
                self.sleep()
            }
            _ => vec![],
        }
    }

    /// The current screen (built once, then shared until it changes).
    pub fn view(&self) -> Rc<ScreenView> { self.built().0 }

    fn built(&self) -> (Rc<ScreenView>, Option<String>) {
        self.cache.borrow_mut().get_or_insert_with(|| {
            let state = self.nav.top();
            let parent = self.nav.parent().map(|p| self.screens[&p.id].view(&self.ctx, p).title);
            let view = match &self.keyboard {
                Some((kb, _)) if state.id == screens::KEYBOARD => keyboard_view(kb),
                _ => self.screens[&state.id].view(&self.ctx, state),
            };
            (Rc::new(view), parent)
        }).clone()
    }

    /// The focused row of `view()`: the one the user left the focus on, if it is still there and
    /// focusable, else the first focusable row (0 if none).
    pub fn focus(&self) -> usize {
        let v = self.view();
        if let Some((kb, _)) = &self.keyboard && self.nav.top().id == screens::KEYBOARD {
            let (line, index) = kb.focus();
            return kb.lines()[..line].iter().map(Vec::len).sum::<usize>() + index;
        }
        self.nav.top().focus.as_ref().and_then(|id| v.rows.iter().position(|r| &r.id == id))
            .filter(|&i| v.rows[i].kind.focusable())
            .or_else(|| list::first(&v.rows)).unwrap_or(0)
    }

    /// Forgets the built screen, after anything that may change what it shows.
    fn changed(&self) { self.cache.borrow_mut().take(); }

    /// Feeds one message; returns the effects the caller must carry out, in order.
    pub fn handle(&mut self, msg: Msg) -> Vec<Effect> {
        // Taken before the message is handled: a game this message quits (from its menu) still
        // holds the sound output.
        let in_game = self.playing.is_some();
        let mut e = self.handle_msg(msg);
        e.extend(self.reconcile_time_zone());
        e.extend(self.reconcile_radios());
        e.extend(self.reconcile_transfer());
        self.drop_unheard_sounds(&mut e, in_game);
        self.hold_settings_saves(&mut e);
        e
    }

    /// Removes the sounds from `effects` that mustn't play (see `Effect::Sound`), and ticks too
    /// close to the last (`TICK_GAP`). The setting and the volume are read after the message, so
    /// switching sounds on is heard and switching them off isn't (whether a game is being played is
    /// taken from before it; see `handle`).
    fn drop_unheard_sounds(&mut self, effects: &mut Vec<Effect>, in_game: bool) {
        let heard = self.ctx.settings.on(builtin::INTERFACE_SOUNDS) && !in_game && !self.muted && self.volume().1 > 0;
        effects.retain(|e| match e {
            Effect::Sound(_) if !heard => false,
            Effect::Sound(Sound::Move) => {
                if self.last_tick.is_some_and(|t| self.now < t + TICK_GAP) { return false; }
                self.last_tick = Some(self.now);
                true
            }
            _ => true,
        });
    }

    fn handle_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Button(e) => {
                self.now = self.now.max(e.time);
                self.idle_since = Some(e.time);
                // A switch, not a key: it counts as activity but is never repeated.
                if e.button == Button::Mute { return self.set_muted(e.pressed); }
                if e.button == Button::Power { return self.power_button(&e); }
                self.repeater.on_event(&e).map(|p| self.press(p)).unwrap_or_default()
            }
            Msg::Tick(now) => {
                self.now = self.now.max(now);
                let mut e = self.repeater.poll(now).map(|p| self.press(p)).unwrap_or_default();
                if self.power_down.take_if(|t| now >= *t + POWER_HOLD).is_some() { e.extend(self.open_power_menu()); }
                e.extend(self.sleep_if_idle(now));
                e
            }
            Msg::Library(games) => {
                if !self.ctx.card_in() { return vec![]; }
                self.ctx.games = games;
                self.ctx.scanned = true;
                self.changed();
                vec![Effect::Redraw(None)]
            }
            Msg::Cores { systems, settings, turned_away } => {
                self.ctx.systems = systems;
                self.ctx.settings.declare(settings);
                self.ctx.turned_away = turned_away;
                self.changed();
                // The running game's core may have had its options learned: it is given the values
                // set for them.
                let options = self.playing.is_some().then(|| Effect::CoreOptions(self.core_options()));
                std::iter::once(Effect::Redraw(None)).chain(options).collect()
            }
            Msg::Card(card) => self.card_changed(card),
            Msg::CardReadOnly => self.card_read_only(),
            // A game slept through stays paused, with its menu open.
            Msg::Woke => {
                self.asleep = false;
                self.repeater.reset();
                self.idle_since = None;
                self.power_down = None;
                if self.input_target() == InputTarget::Game { self.open_game_menu() } else { vec![] }
            }
            Msg::GameEnded(why) => {
                if self.playing.take().is_none() { return vec![]; }
                self.idle_since = None;
                self.nav.close(&screens::CARD_WAIT);
                self.nav.close(&screens::GAME_MENU);
                self.changed();
                why.map(|w| Effect::Notice(format!("The game couldn't start: {w}"))).into_iter().chain([Effect::Redraw(None)]).collect()
            }
            Msg::GameStopped(why) => {
                if self.playing.take().is_none() { return vec![]; }
                self.idle_since = None;
                self.nav.close(&screens::GAME_MENU);
                self.changed();
                vec![Effect::QuitGame, Effect::Notice(format!("The game stopped: {why}")), Effect::Redraw(None)]
            }
            Msg::LastGameStopped(r) => {
                let title = crate::library::display_title(r.game.file_stem().unwrap_or_default().to_str().unwrap_or_default());
                vec![Effect::Notice(match r.reason {
                    Some(why) => format!("{} was stopped while playing {title}: {why}", r.core),
                    None => format!("{} stopped unexpectedly while playing {title}", r.core),
                })]
            }
            Msg::Battery(b) => {
                let changed = self.ctx.battery != Some(b);
                self.ctx.battery = Some(b);
                if changed { vec![Effect::Redraw(None)] } else { vec![] }
            }
            Msg::SettingsLoaded(layer, values) => self.layer_loaded(layer, values),
            Msg::Wifi(e) => self.wifi_event(e),
            Msg::Bluetooth(e) => self.bluetooth_event(e),
            Msg::WifiNeed { need, wanted, time } => { self.now = self.now.max(time); self.wifi_need(need, wanted); vec![] }
            Msg::SoundFailed(output) => self.sound_failed(output),
            Msg::Headphones(plugged) => { self.headphones = plugged; vec![] } // the output follows (`reconcile_radios`)
            Msg::InstallOffer(offer) => self.install_offer(offer),
            Msg::InstallProgress(p) => self.install_progress(p),
            Msg::FileTransfer { start, result } => self.transfer_done(start, result),
            Msg::Ssh { cmd, result } => self.ssh_done(cmd, result),
            Msg::Discs(d) => {
                if self.playing.is_none() { return vec![]; }
                self.ctx.discs = d;
                self.changed();
                vec![Effect::Redraw(None)]
            }
            Msg::TimeZone(e) => self.time_zone_event(e),
        }
    }

    /// A settings layer was read: once the global one is, every setting is applied.
    fn layer_loaded(&mut self, layer: Layer, values: serde_json::Map<String, serde_json::Value>) -> Vec<Effect> {
        // A change made before the file was loaded was saved without the file's values (and
        // replaced it; the load was asked for first, so it read the file before): save the two
        // together.
        let save = (!self.ctx.settings.layer(&layer).is_empty()).then(|| Effect::SaveSettings(layer.clone()));
        let global = layer == Layer::Global;
        self.ctx.settings.set_layer(layer, values);
        self.changed();
        // The other layers are loaded before the games they are for are listed, so before any of
        // those can be played: nothing in them applies yet.
        let applied: Vec<Effect> = if global {
            self.settings_loaded = true;
            self.apply_all(|_| true)
        } else { vec![] };
        std::iter::once(Effect::Redraw(None)).chain(applied).chain(save).collect()
    }

    fn press(&mut self, p: Press) -> Vec<Effect> {
        match (self.input_target(), p.button) {
            (_, Button::VolUp | Button::VolDown) => return self.step_volume(if p.button == Button::VolUp { 1 } else { -1 }),
            (InputTarget::Game, Button::Menu) => return self.open_game_menu(),
            (InputTarget::Game, _) => return vec![], // the UI gives these to the game
            _ => {}
        }
        // The keyboard moves its own focus (its lines differ in length).
        if self.nav.top().id == screens::KEYBOARD && p.button != Button::Menu { return self.keyboard(p); }
        let (view, focus) = (self.view(), self.focus());
        // Focus moves change nothing on the screen itself, so its cache stays. In a grid, left and
        // right move one cell, up and down a row of cells and the shoulder buttons two rows; in a
        // list, up and down move one row and the shoulder buttons six.
        let grid = view.layout == Layout::Grid;
        let (row, page) = if grid { (self.grid_columns as isize, 2 * self.grid_columns as isize) } else { (1, 6) };
        let delta = match p.button {
            Button::Up => Some(-row), Button::Down => Some(row), Button::L1 => Some(-page), Button::R1 => Some(page),
            Button::Left if grid => Some(-1), Button::Right if grid => Some(1),
            _ => None,
        };
        if let Some(delta) = delta {
            let to = list::step(&view.rows, focus, delta, p.repeat);
            if to == focus { return vec![]; }
            self.nav.top_mut().focus = view.rows.get(to).map(|r| r.id.clone());
            return vec![Effect::Sound(Sound::Move), Effect::Redraw(None)];
        }
        let id = self.nav.top().id.clone();
        if matches!(p.button, Button::L2 | Button::R2) && self.screens[&id].letter_index() && !view.rows.is_empty() {
            let to = list::jump(&view.rows, focus, p.button == Button::R2, p.repeat, |r| library::index_letter(&r.label));
            let letter = Effect::ShowLetter(library::index_letter(&view.rows[to].label));
            if to == focus { return vec![letter]; }
            self.nav.top_mut().focus = Some(view.rows[to].id.clone());
            return vec![Effect::Sound(Sound::Move), Effect::Redraw(None), letter];
        }
        let focused = view.rows.get(focus).filter(|r| r.kind.focusable());
        let row = focused.map(|r| r.id.0.as_str());
        if id == screens::GAME_MENU { return self.game_menu(p.button, row); }
        if id == screens::POWER_MENU { return self.power_menu(p.button, row); }
        if let Some(e) = self.install_press(&id, p.button, row) { return e; }
        if id == screens::ZONE_CONSENT { return self.zone_consent(p.button, row); }
        if [screens::ZONE_AREAS, screens::ZONE_CITIES].contains(&id) && p.button == Button::A
            && let Some(zone) = row.and_then(|r| r.strip_prefix("zone:")) { return self.pick_zone(zone); }
        if id == screens::CARD_WAIT { return self.card_wait(p.button, row); }
        // (A, B and Menu never repeat.) Over a paused game, Menu resumes it.
        if p.button == Button::Menu && self.nav.contains(&screens::GAME_MENU) { return self.resume(); }
        if p.button == Button::Menu { return self.go(|nav| nav.home()); }
        let outcome = self.screens[&id].press(&mut self.ctx, self.nav.top_mut(), focused, p);
        self.outcome(outcome)
    }

    /// Carries out what a press on the current screen (or text typed for it) led to.
    fn outcome(&mut self, outcome: Outcome) -> Vec<Effect> {
        match outcome {
            Outcome::None => vec![],
            Outcome::Changed => { self.changed(); vec![Effect::Redraw(None)] }
            Outcome::Push(s) => {
                let settings = s.id == screens::SETTINGS;
                let mut e = self.go(|nav| Some(nav.push(s)));
                // Settings shows what the installer can do now, and SSH's state.
                if settings {
                    e.extend(self.ctx.install.as_ref().map(|_| Effect::Installer(InstallCmd::Offer)));
                    e.extend(self.ctx.ssh.as_ref().map(|_| Effect::Ssh(SshCmd::Status)));
                }
                e
            }
            Outcome::Back => self.go(|nav| nav.pop()),
            Outcome::BackWith(mut e) => { e.extend(self.go(|nav| nav.pop())); e }
            Outcome::Type { prompt, masked, purpose } => {
                self.keyboard = Some((Keyboard::new(prompt, masked), purpose));
                self.go(|nav| Some(nav.push(ScreenState::new(screens::KEYBOARD, None))))
            }
            Outcome::Launch(game) => {
                self.playing = Some(game.clone());
                self.ctx.discs = Discs::default();
                self.repeater.reset();
                // The settings that may differ for this game.
                let per_game = self.apply_all(|d| d.deepest > crate::settings::Scope::Global);
                let unsaved = (!self.ctx.card_writable()).then(|| Effect::Notice("The SD card is read-only: progress in this game won't be saved".into()));
                std::iter::once(Effect::Launch(game)).chain(per_game).chain([Effect::Redraw(None)]).chain(unsaved).collect()
            }
            Outcome::Effects(e) => { self.changed(); e }
            Outcome::SettingChanged(layer, key) => {
                self.changed();
                // Switches sound their new state; numbers and choices tick as they step.
                let on = self.ctx.settings.get(key, &layer).and_then(|(v, _)| v.as_bool());
                let mut e = vec![Effect::Sound(on.map_or(Sound::Move, Sound::Toggle)), Effect::Redraw(None), Effect::SaveSettings(layer)];
                e.extend(self.apply(key));
                e
            }
        }
    }

    /// The volume setting of the output sound goes to: its key and level (0 if unset).
    fn volume(&self) -> (&'static str, i64) {
        let key = self.radios.output.volume_setting();
        (key, self.ctx.settings.get(key, &Layer::Global).and_then(|(v, _)| v.as_i64()).unwrap_or(0))
    }

    fn show_volume(&self, level: i64) -> Effect {
        Effect::ShowVolume { level, muted: self.muted, output: self.radios.output.clone() }
    }

    /// The volume keys, on every screen and in games. The level is shown even when it is already
    /// at an end and doesn't change. While muted they still change the level, which is heard once
    /// the sound is switched back on.
    fn step_volume(&mut self, delta: i64) -> Vec<Effect> {
        let (key, now) = self.volume();
        let new = (now + delta).clamp(0, builtin::VOLUME_MAX);
        let show = self.show_volume(new);
        if new == now { return vec![show]; }
        let _ = self.ctx.settings.set(key, &Layer::Global, new.into());
        self.changed(); // the settings screen shows the volume too
        vec![Effect::Apply { key, value: new.into() }, Effect::SaveSettings(Layer::Global), show, Effect::Redraw(None)]
    }

    /// The mute switch. Not saved: the switch holds its own position, and the device reports it at
    /// startup (a switch already on then counts as a change).
    fn set_muted(&mut self, muted: bool) -> Vec<Effect> {
        if muted == self.muted { return vec![]; }
        self.muted = muted;
        vec![Effect::Mute(muted), self.show_volume(self.volume().1)]
    }

    /// The power button, on every screen and in games: a tap (released within `POWER_HOLD`)
    /// sleeps; holding it opens the power menu once the hold is due (see `Msg::Tick`).
    fn power_button(&mut self, e: &ButtonEvent) -> Vec<Effect> {
        // Nothing may interrupt the installer (see app::install).
        if self.task_running() {
            self.power_down = None;
            return if e.pressed { vec![Effect::Notice(self.task_wait_notice())] } else { vec![] };
        }
        if e.pressed {
            self.repeater.reset();
            self.power_down = Some(e.time);
            return vec![];
        }
        // None: held into the power menu, or a press the app didn't see.
        if self.power_down.take().is_some() { self.sleep() } else { vec![] }
    }

    /// Opens the power menu, over the game menu if a game is running (pausing it).
    fn open_power_menu(&mut self) -> Vec<Effect> {
        if self.nav.top().id == screens::POWER_MENU { return vec![]; }
        let mut e = if self.input_target() == InputTarget::Game { self.open_game_menu() } else { vec![] };
        e.extend(self.go(|nav| Some(nav.push(ScreenState::new(screens::POWER_MENU, None)))));
        e
    }

    /// A press in the power menu: B and Menu close it, A does the focused row's action. Powering
    /// off or restarting quits a running game first, which writes its saves.
    fn power_menu(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        let (power, notice) = match (button, row) {
            (Button::B | Button::Menu, _) | (Button::A, Some("cancel")) => return self.go(|nav| nav.pop()),
            (Button::A, Some("off")) => (Effect::PowerOff, "Turning off…"),
            (Button::A, Some("restart")) => (Effect::Restart, "Restarting…"),
            _ => return vec![],
        };
        self.end_game().into_iter().chain([Effect::Notice(notice.into()), power]).collect()
    }

    /// Ends the game being played, if any: with its saves written if the card can take them.
    fn end_game(&mut self) -> Option<Effect> {
        self.playing.take()?;
        Some(if self.ctx.card_writable() { Effect::QuitGame } else { Effect::AbandonGame })
    }

    fn open_game_menu(&mut self) -> Vec<Effect> {
        let mut e = vec![Effect::PauseGame(true)];
        e.extend(self.go(|nav| Some(nav.push(ScreenState::new(screens::GAME_MENU, None)))));
        e
    }

    /// A press in the in-game menu: B and Menu resume, A does the focused row's action, and on the
    /// Disc row, right (or A) and left change to the next or previous disc, wrapping around. Every
    /// action closes the menu, resuming the game unless it was quit.
    fn game_menu(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        let mut e = match (button, row) {
            (Button::B | Button::Menu, _) | (Button::A, Some("resume")) => vec![Effect::PauseGame(false)],
            (Button::Left | Button::Right | Button::A, Some("disc")) => {
                let Discs { count, current } = self.ctx.discs;
                let back = button == Button::Left;
                // With no disc in, right puts the first in and left the last.
                let to = current.map_or(if back { count - 1 } else { 0 }, |c| (c + if back { count - 1 } else { 1 }) % count);
                vec![Effect::ChangeDisc(to), Effect::PauseGame(false)]
            }
            (Button::A, Some("save")) if !self.ctx.card_writable() => return vec![Effect::Notice("The SD card is read-only: the state can't be saved".into())],
            (Button::A, Some("save")) => vec![Effect::SaveState, Effect::PauseGame(false)],
            (Button::A, Some("load")) => vec![Effect::LoadState, Effect::PauseGame(false)],
            (Button::A, Some("reset")) => vec![Effect::ResetGame, Effect::PauseGame(false)],
            (Button::A, Some("settings")) => {
                let Some(game) = &self.playing else { return vec![] };
                let path = game.path.to_string_lossy().into_owned();
                return self.go(|nav| Some(nav.push(ScreenState::new(screens::GAME_SETTINGS, Some(path)))));
            }
            (Button::A, Some("quit")) => self.end_game().into_iter().collect(),
            _ => return vec![],
        };
        e.extend(self.go(|nav| nav.pop()));
        e
    }

    /// Closes the game menu, and any screen opened over it, resuming the game.
    fn resume(&mut self) -> Vec<Effect> {
        let mut e = vec![Effect::PauseGame(false)];
        e.extend(self.go(|nav| nav.close(&screens::GAME_MENU)));
        e
    }

    /// A press on the keyboard: B on empty text and Menu leave it; Start (or its done key) gives
    /// the text to the screen below, which may refuse it with a message.
    fn keyboard(&mut self, p: Press) -> Vec<Effect> {
        let Some((kb, _)) = &mut self.keyboard else { return self.go(|nav| nav.pop()) };
        match kb.press(p) {
            KeyOutcome::None => vec![],
            KeyOutcome::Changed => { self.changed(); vec![Effect::Sound(Sound::Move), Effect::Redraw(None)] }
            KeyOutcome::Cancel => self.go(|nav| nav.pop()),
            KeyOutcome::Done(text) => {
                let purpose = self.keyboard.as_ref().map(|(_, p)| p.clone()).unwrap_or_default();
                let below = self.nav.parent().cloned();
                let Some(mut below) = below else { return vec![] };
                match self.screens[&below.id].entered(&mut self.ctx, &mut below, &purpose, text) {
                    Err(message) => vec![Effect::Notice(message)],
                    // A screen opened replaces the keyboard in one push, rather than a pop then a push.
                    Ok(Outcome::Push(s)) => self.go(|nav| { nav.pop(); *nav.top_mut() = below; Some(nav.push(s)) }),
                    Ok(outcome) => {
                        let mut e = self.go(|nav| { nav.pop(); *nav.top_mut() = below; Some(Transition::Pop) });
                        e.extend(self.outcome(outcome));
                        e
                    }
                }
            }
        }
    }

    /// Changes screen (if `f` does), with a redraw that animates the change.
    fn go(&mut self, f: impl FnOnce(&mut NavStack) -> Option<Transition>) -> Vec<Effect> {
        let Some(t) = f(&mut self.nav) else { return vec![] };
        if !self.nav.contains(&screens::KEYBOARD) { self.keyboard = None; }
        self.repeater.reset();
        self.changed();
        vec![Effect::Sound(if t == Transition::Push { Sound::Open } else { Sound::Back }), Effect::Redraw(Some(t))]
    }
}

/// Home: the systems that have games, and Search when there are any; then settings.
struct Home;

impl Screen for Home {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let mut rows = card::home_notes(ctx);
        rows.push(Row::new("header:systems", "Systems", RowKind::Header));
        for s in &ctx.systems {
            let n = ctx.games.iter().filter(|g| g.system == s.id).count();
            if n > 0 { rows.push(Row::new(format!("system:{}", s.id), s.name, RowKind::Text { detail: Some(n.to_string()) })); }
        }
        if ctx.games.is_empty() {
            if let Some(card) = ctx.card.as_ref().filter(|c| !c.readable()) {
                rows.push(Row::note("note:card", card.text()));
            } else if !ctx.scanned {
                rows.push(Row::note("note:scanning", "Looking for games…"));
            } else {
                rows.push(Row::note("note:none", "No games found"));
                rows.push(Row::note("note:where", "Put them in roms/ on the SD card"));
            }
        } else {
            rows.push(action("search", "Search"));
        }
        rows.push(Row::new("header:device", "Device", RowKind::Header));
        rows.push(Row::new("settings", "Settings", RowKind::Action { value: None }));
        ScreenView { title: "Library".into(), layout: Layout::List, rows, hints: vec![(Button::A, "Open".into())], entry: None }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        let (Button::A, Some(row)) = (p.button, focused) else { return Outcome::None };
        if row.id.0 == card::DISMISS { ctx.card_note = false; return Outcome::Changed; }
        match row.id.0.strip_prefix("system:") {
            Some(system) => Outcome::Push(ScreenState::new(screens::GAMES, Some(system.to_string()))),
            None if row.id.0 == "search" => type_search(),
            None if row.id.0 == "settings" => Outcome::Push(ScreenState::new(screens::SETTINGS, None)),
            None => Outcome::None,
        }
    }

    fn entered(&self, _: &mut Ctx, _: &mut ScreenState, _: &str, text: String) -> Result<Outcome, String> { Ok(open_search(&text)) }
}

/// Opens the keyboard to type a search; the screen's `entered` passes the text to `open_search`.
fn type_search() -> Outcome { Outcome::Type { prompt: "Search".into(), masked: false, purpose: String::new() } }

/// Opens the results screen for text typed; blank text just closes the keyboard.
fn open_search(text: &str) -> Outcome {
    let query = text.trim();
    if query.is_empty() { Outcome::None } else { Outcome::Push(ScreenState::new(screens::SEARCH, Some(query.into()))) }
}

/// A game's row, with its picture and `detail` beside the title. Its id is the game's path under
/// `roms/`. `art` is each system's kind of picture and its shape (`art_by_system`).
fn game_row(g: &Game, detail: Option<String>, art: &HashMap<&str, (ArtKind, (u32, u32))>) -> Row {
    let (kind, shape) = art.get(g.system).copied().unwrap_or((ArtKind::BoxArt, (1, 1)));
    Row::new(g.path.to_string_lossy(), &g.title, RowKind::Image { image: ImageRef::Art { system: g.system, rom: g.path.clone(), kind, shape }, detail })
}

/// Each system's `ArtKind` and its pictures' shape, looked up once per screen rather than per game.
fn art_by_system(ctx: &Ctx) -> HashMap<&'static str, (ArtKind, (u32, u32))> {
    ctx.systems.iter().map(|s| { let kind = art::kind(&ctx.settings, s.id); (s.id, (kind, kind.shape(s))) }).collect()
}

/// Whether games are shown as a grid of pictures (else as a list beside the focused game's picture).
fn games_grid(ctx: &Ctx) -> bool { ctx.settings.get(builtin::LIBRARY_VIEW, &Layer::Global).is_some_and(|(v, _)| v == "grid") }

/// A screen of games, as a list or a grid (`games_grid`).
fn games_view(ctx: &Ctx, title: String, rows: Vec<Row>) -> ScreenView {
    let grid = games_grid(ctx);
    ScreenView {
        title, layout: if grid { Layout::Grid } else { Layout::ListWithPicture }, rows,
        hints: vec![(Button::Start, "Settings".into()), (Button::Y, if grid { "List" } else { "Grid" }.into()), (Button::X, "Search".into()),
                    (Button::B, "Back".into()), (Button::A, "Play".into())],
        entry: None,
    }
}

/// A press on a screen of games: A plays the focused one, Start opens its settings, Y switches
/// between list and grid, X opens the keyboard to search, B goes back.
fn games_press(ctx: &mut Ctx, focused: Option<&Row>, p: Press) -> Outcome {
    match (p.button, focused) {
        (Button::B, _) => Outcome::Back,
        (Button::Start, Some(row)) if !p.repeat => Outcome::Push(ScreenState::new(screens::GAME_SETTINGS, Some(row.id.0.clone()))),
        (Button::X, _) => type_search(),
        (Button::Y, _) if !p.repeat => {
            let value = if games_grid(ctx) { "list" } else { "grid" };
            let _ = ctx.settings.set(builtin::LIBRARY_VIEW, &Layer::Global, value.into());
            Outcome::Effects(vec![Effect::Redraw(None), Effect::SaveSettings(Layer::Global)])
        }
        (Button::A, Some(row)) => ctx.games.iter().find(|g| g.path.as_os_str() == row.id.0.as_str())
            .map_or(Outcome::None, |g| Outcome::Launch(g.clone())),
        _ => Outcome::None,
    }
}

/// One system's games (the system's id is the screen's params), by title, with a letter index.
struct Games;

impl Screen for Games {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let system = state.params.as_deref().unwrap_or_default();
        let title = ctx.systems.iter().find(|s| s.id == system).map_or(system, |s| s.name).to_string();
        let art = art_by_system(ctx);
        games_view(ctx, title, ctx.games.iter().filter(|g| g.system == system).map(|g| game_row(g, None, &art)).collect())
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome { games_press(ctx, focused, p) }

    fn entered(&self, _: &mut Ctx, _: &mut ScreenState, _: &str, text: String) -> Result<Outcome, String> { Ok(open_search(&text)) }

    fn letter_index(&self) -> bool { true }
}

/// The games of every system matching a search (the query in the params), best first
/// (`library::search`), each with its system. A new search (X) replaces this one. No letter
/// index: the results are ranked, not in title order.
struct Search;

impl Screen for Search {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let query = state.params.as_deref().unwrap_or_default();
        let art = art_by_system(ctx);
        let rows: Vec<Row> = library::search(&ctx.games, query).into_iter().map(|g| game_row(g, Some(g.system.to_uppercase()), &art)).collect();
        let title = format!("“{query}”");
        if !rows.is_empty() { return games_view(ctx, title, rows); }
        // No matches: a plain list, as a grid or the picture beside the list would be empty.
        ScreenView {
            title, layout: Layout::List, rows: vec![Row::note("note:none", "No games match this search")],
            hints: vec![(Button::X, "Search".into()), (Button::B, "Back".into())], entry: None,
        }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome { games_press(ctx, focused, p) }

    fn entered(&self, _: &mut Ctx, state: &mut ScreenState, _: &str, text: String) -> Result<Outcome, String> {
        if let Outcome::Push(results) = open_search(&text) { *state = results; }
        Ok(Outcome::None)
    }
}

/// The global settings, generated from the declarations.
struct SettingsScreen;

impl Screen for SettingsScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let connections: Vec<Row> = wireless::settings_rows(ctx).into_iter().chain(transfer::settings_row(ctx)).collect();
        let mut rows = vec![];
        if !connections.is_empty() { rows.push(Row::new("section:connections", "Connections", RowKind::Header)); }
        rows.extend(connections);
        rows.extend(timezone::settings_rows(ctx));
        rows.extend(ctx.settings.rows(&Layer::Global, ctx.visibility(), None));
        // Credits, last in the System section.
        let system_end = rows.iter().position(|r| r.id.0 == builtin::DEVELOPER_DETAILS).map_or(rows.len(), |i| i + 1);
        rows.insert(system_end, credits::settings_row());
        if ctx.visibility() == Visibility::Developer {
            // Appended to the Developer section, the last settings section.
            rows.extend(ssh::settings_rows(ctx));
            rows.extend([action("open:cores", "Cores"), action("open:icons", "Icons")]);
        }
        rows.extend(install::settings_rows(ctx));
        ScreenView::list("Settings", rows)
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { return Outcome::Back; }
        if let Some(open) = focused.and_then(|row| wireless::settings_press(row, p)) { return open; }
        if p.button == Button::A && let Some(row) = focused {
            match row.id.0.as_str() {
                "open:icons" => return Outcome::Push(ScreenState::new(screens::ICONS, None)),
                "open:cores" => return Outcome::Push(ScreenState::new(screens::CORES, None)),
                "open:credits" => return Outcome::Push(ScreenState::new(screens::CREDITS, None)),
                "open:file transfer" => return Outcome::Push(ScreenState::new(screens::FILE_TRANSFER, None)),
                "open:time zone" => return Outcome::Push(ScreenState::new(screens::TIME_ZONE, None)),
                _ => {}
            }
        }
        if let Some(o) = focused.and_then(|row| ssh::settings_press(ctx, row, p)) { return o; }
        if let Some(open) = focused.and_then(|row| install::settings_press(ctx, row, p)) { return open; }
        focused.map_or(Outcome::None, |row| setting_press(ctx, row, p))
    }
}

/// A press on a global setting's row: changes it (`Outcome::SettingChanged` has the app save and
/// apply it).
fn setting_press(ctx: &mut Ctx, row: &Row, p: Press) -> Outcome {
    ctx.settings.press(&row.id.0, &Layer::Global, p).map_or(Outcome::None, |(key, _)| Outcome::SettingChanged(Layer::Global, key))
}

/// The keyboard's screen: one row per key, line by line (`Layout::Keys`), its text above them.
/// Keys' row ids are their line and column, so the focus is the keyboard's own. Masked text has
/// a hint for Select, which shows or hides it.
fn keyboard_view(kb: &Keyboard) -> ScreenView {
    let rows = kb.lines().iter().enumerate().flat_map(|(line, keys)| keys.iter().map(move |k| {
        Row::new(format!("key:{line}:{}", k.column), kb.label(k.key), RowKind::Key { line, column: k.column, span: k.span })
    })).collect();
    let (shown, caret) = kb.shown();
    let reveal = kb.revealed().map(|shown| (Button::Select, if shown { "Hide" } else { "Show" }));
    let hints = reveal.into_iter().chain([(Button::Y, "Shift"), (Button::X, "Symbols"), (Button::B, "Delete"), (Button::Start, "Done")]);
    ScreenView {
        title: kb.prompt.clone(), layout: Layout::Keys, rows,
        hints: hints.map(|(b, label)| (b, label.into())).collect(),
        entry: Some(TextEntry { text: shown, caret }),
    }
}

/// A menu row that does something when pressed.
fn action(id: &str, label: &str) -> Row { Row::new(id, label, RowKind::Action { value: None }) }

/// Power off or restart. The app handles its presses (`App::power_menu`), since they may quit
/// the game.
struct PowerMenu;

impl Screen for PowerMenu {
    fn view(&self, _: &Ctx, _: &ScreenState) -> ScreenView {
        ScreenView {
            title: "Power".into(), layout: Layout::List,
            rows: vec![action("off", "Power off"), action("restart", "Restart"), action("cancel", "Cancel")],
            hints: vec![(Button::B, "Cancel".into()), (Button::A, "Select".into())],
            entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

/// The menu over a paused game. The app handles its presses (`App::game_menu`), since they
/// control the game itself.
struct GameMenu;

impl Screen for GameMenu {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let mut rows = vec![action("resume", "Resume")];
        rows.extend(disc_row(ctx.discs));
        rows.extend([action("save", "Save state"), action("load", "Load state"), action("reset", "Restart"),
                     action("settings", "Game settings"), action("quit", "Quit game")]);
        ScreenView {
            title: "Paused".into(), layout: Layout::List, rows,
            hints: vec![(Button::B, "Resume".into()), (Button::A, "Select".into())],
            entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

/// The game menu's Disc row, for a game on more than one disc: "Disc 1 of 3", …, and "No disc"
/// while none is in.
fn disc_row(discs: Discs) -> Option<Row> {
    let Discs { count, current } = discs;
    if count < 2 { return None; }
    let mut options: Vec<String> = (1..=count).map(|i| format!("Disc {i} of {count}")).collect();
    if current.is_none() { options.push("No disc".into()); }
    let selected = current.unwrap_or(count);
    Some(Row::new("disc", "Disc", RowKind::Choice { options, selected, swatch: None }))
}

#[cfg(test)]
mod tests;
