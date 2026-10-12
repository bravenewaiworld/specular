//! Screens as data. A screen is a title, a layout and rows; the UI draws each row kind with one
//! prefab component, so every screen looks and moves the same way.

use std::borrow::Cow;
use std::path::PathBuf;

use crate::input::Button;

/// A screen's identity. A string, so plugins can register screens of their own later.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScreenId(pub Cow<'static, str>);

/// A row's identity within its screen (a setting's key, a game's path, an action's name). Stable
/// across rebuilds of the screen, so focus can be kept on the same row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowId(pub String);

/// A picture the UI loads (off its thread) and shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ImageRef {
    /// A built-in icon, by name.
    Icon(&'static str),
    /// An image file, e.g. cached box art.
    File(PathBuf),
}

/// What a row is, and the value it shows.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    /// A section title; never focused.
    Header,
    /// A label, with optional detail on the right (a count, a status).
    Text { detail: Option<String> },
    /// A row with a picture (a game with its box art).
    Image { image: ImageRef, detail: Option<String> },
    /// On or off; A, left or right switches it.
    Toggle { on: bool },
    /// One of a few options; left and right step through them. Long lists open a picker.
    Choice { options: Vec<String>, selected: usize },
    /// A number in a range; left and right step it.
    Number { value: i32, min: i32, max: i32, step: i32, unit: &'static str },
    /// Does something or opens a screen when A is pressed; may show its current value.
    Action { value: Option<String> },
}

impl RowKind {
    /// Whether the focus can land on the row.
    pub fn focusable(&self) -> bool { !matches!(self, RowKind::Header) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: RowId,
    pub label: String,
    pub kind: RowKind,
    /// Shown dimmed and inert when false.
    pub enabled: bool,
    /// Its value overrides the layer below (e.g. a per-game setting); the UI marks it.
    pub overridden: bool,
}

impl Row {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: RowKind) -> Self {
        Self { id: RowId(id.into()), label: label.into(), kind, enabled: true, overridden: false }
    }
}

/// How a screen's rows are arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    List,
    /// Box art in columns; the UI picks the column count from the screen's width.
    Grid,
    /// A picker over the current screen (e.g. the language list).
    Sheet,
}

/// Everything the UI needs to draw one screen.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenView {
    pub title: String,
    pub layout: Layout,
    pub rows: Vec<Row>,
    /// The focused row's index; always a focusable row while there is one.
    pub focus: usize,
    /// Button hints for the bottom bar. The UI shows only buttons the device has.
    pub hints: Vec<(Button, String)>,
}
