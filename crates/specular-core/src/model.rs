//! Screens as data. A screen is a title, a layout and rows; the UI draws each row kind with one
//! prefab component, so every screen looks and moves the same way.

use std::borrow::Cow;
use std::path::PathBuf;

use crate::art::ArtKind;
use crate::device::{Battery, Rgb};
use crate::input::Button;
use crate::wireless::bluetooth::BtKind;

/// A screen's identity. A string, so plugins can register screens of their own later.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScreenId(pub Cow<'static, str>);

/// A row's identity within its screen (a setting's key, a game's path, an action's name). Stable
/// across rebuilds of the screen, so focus can be kept on the same row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowId(pub String);

/// A picture for a row, loaded by the UI off its thread.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ImageRef {
    /// A game's picture of `kind` (else its box art), by its system and its ROM's path under
    /// `roms/`. The UI's art service finds it; while there is none, the UI draws a generated
    /// cover. `shape` (width, height) is the shape of the place it is drawn in
    /// (`ArtKind::shape`).
    Art { system: &'static str, rom: PathBuf, kind: ArtKind, shape: (u32, u32) },
}

/// A small symbol, drawn by the UI in the accent colours: beside a row's label, or in the status
/// bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// WiFi's arcs, one lit per bar, 0–4 (None: strength unknown, drawn full), with a padlock
    /// for a network that needs a password.
    Wifi { bars: Option<u8>, locked: bool },
    /// What a Bluetooth device is. Dimmed unless `lit`: the device is connected (in the status
    /// bar: any device is). `BtKind::Other` draws the bare Bluetooth mark.
    Bluetooth { kind: BtKind, lit: bool },
    /// A battery with its charge inside; turns to the warning colour when `Battery::low`.
    Battery(Battery),
}

/// What a row is, and the value it shows.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    /// A section title; never focused.
    Header,
    /// Text that explains something, with an optional detail on the right (a step's status);
    /// never focused, and as many lines tall as its text needs (see `list::note_lines`).
    Note { detail: Option<String> },
    /// A label, with optional detail on the right (a count, a status).
    Text { detail: Option<String> },
    /// A row with a picture (a game with its box art).
    Image { image: ImageRef, detail: Option<String> },
    /// On or off; A, left or right switches it.
    Toggle { on: bool },
    /// One of a few options; left and right step through them. A colour choice also shows the
    /// colour it stands for.
    Choice { options: Vec<String>, selected: usize, swatch: Option<Rgb> },
    /// A number in a range; left and right step it.
    Number { value: i64, min: i64, max: i64, step: i64, unit: &'static str },
    /// Does something or opens a screen when A is pressed; may show its current value.
    Action { value: Option<String> },
    /// A key of the on-screen keyboard (`Layout::Keys`): its line, and its first column and width
    /// in columns, out of `keyboard::WIDTH`.
    Key { line: usize, column: usize, span: usize },
}

impl RowKind {
    /// Whether the focus can land on the row.
    pub fn focusable(&self) -> bool { !matches!(self, RowKind::Header | RowKind::Note { .. }) }
}

/// One row of a screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: RowId,
    pub label: String,
    pub kind: RowKind,
    /// Its value overrides the layer below (e.g. a per-game setting); the UI marks it.
    pub overridden: bool,
    /// Drawn before the label.
    pub icon: Option<Icon>,
    /// A button hint shown, before the screen's own, while the row is focused (X: Reset on a
    /// setting that overrides).
    pub hint: Option<(Button, String)>,
}

impl Row {
    /// A row that overrides nothing.
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: RowKind) -> Self {
        Self { id: RowId(id.into()), label: label.into(), kind, overridden: false, icon: None, hint: None }
    }

    /// A note: text that explains something (see `RowKind::Note`).
    pub fn note(id: impl Into<String>, text: impl Into<String>) -> Self { Self::new(id, text, RowKind::Note { detail: None }) }

    /// This row with `icon` drawn before its label.
    pub fn icon(mut self, icon: Icon) -> Self { self.icon = Some(icon); self }
}

/// How a screen's rows are arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    List,
    /// A list beside the focused row's picture (for rows with one).
    ListWithPicture,
    /// Pictures in a grid, each with its label underneath; see `list::grid_columns`.
    Grid,
    /// The on-screen keyboard: rows are keys (`RowKind::Key`) under the screen's text entry.
    Keys,
}

/// Everything the UI needs to draw one screen.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenView {
    pub title: String,
    pub layout: Layout,
    pub rows: Vec<Row>,
    /// Button hints for the bottom bar. The UI shows only buttons the device has.
    pub hints: Vec<(Button, String)>,
    /// Text being typed, shown above the rows (the keyboard's).
    pub entry: Option<TextEntry>,
}

/// Text being typed: as shown (masked text as dots), and the caret's place in characters.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEntry { pub text: String, pub caret: usize }
