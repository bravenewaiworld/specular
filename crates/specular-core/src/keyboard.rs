//! An on-screen keyboard: the text being typed, and keys in lines that the focus moves over.
//! Every line is `WIDTH` columns wide; a key spans one or more columns, so moving up or down
//! lands on the key under the middle of the one left. It types printable ASCII only, which is
//! also all a WiFi password may hold.
//!
//! Buttons: the d-pad moves (wrapping around), A presses the focused key, B deletes the character
//! before the caret, Y is shift, X switches to symbols and back, Start is done, Select shows or
//! hides masked text, and L1 and R1 move the caret.

use crate::input::{Button, Press};

/// How many columns each line has.
pub const WIDTH: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    /// Upper case on or off (stays until pressed again: passwords mix cases freely).
    Shift,
    /// Switches between letters and symbols.
    Symbols,
    Backspace,
    Done,
}

/// A key in its line: what it is, and its first column and width in columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed { pub key: Key, pub column: usize, pub span: usize }

/// Characters, then the keys of the last line, which differ between the two layers. Upper case
/// is the letters layer's `Char`s uppercased.
const LETTERS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl@", "zxcvbnm.-_"];
const SYMBOLS: [&str; 4] = ["1234567890", "!#$%^&*()'", "\"+=[]{}\\|~", ";:,<>/?`  "];

/// The last line: (key, span) pairs, `WIDTH` columns in all.
const LAST_LINE: [(Key, usize); 4] = [(Key::Shift, 2), (Key::Symbols, 2), (Key::Char(' '), 4), (Key::Done, 2)];

/// What a press on the keyboard did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Nothing visible changed.
    None,
    Changed,
    /// Done was pressed, with this text.
    Done(String),
    /// B on empty text: leave without typing anything.
    Cancel,
}

/// The keyboard's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyboard {
    /// What the text is for ("Password for Office").
    pub prompt: String,
    text: Vec<char>,
    /// Where the next character goes, 0..=text.len().
    caret: usize,
    /// Shown as dots unless revealed (Select).
    masked: bool,
    revealed: bool,
    upper: bool,
    symbols: bool,
    /// The focused key, as (line, column of the key's first column).
    focus: (usize, usize),
}

impl Keyboard {
    pub fn new(prompt: impl Into<String>, masked: bool) -> Self {
        Self { prompt: prompt.into(), text: Vec::new(), caret: 0, masked, revealed: false, upper: false, symbols: false, focus: (1, 0) }
    }

    /// The keys, line by line, as they are now (letters or symbols).
    pub fn lines(&self) -> Vec<Vec<Placed>> {
        let chars = if self.symbols { SYMBOLS } else { LETTERS };
        let mut lines: Vec<Vec<Placed>> = chars.iter().map(|l| {
            // Blanks pad a line out to the full width; they aren't keys.
            l.chars().enumerate().filter(|&(i, c)| c != ' ' || i < l.trim_end().len()).map(|(column, c)| Placed { key: Key::Char(c), column, span: 1 }).collect()
        }).collect();
        let mut column = 0;
        lines.push(LAST_LINE.iter().map(|&(key, span)| { column += span; Placed { key, column: column - span, span } }).collect());
        lines
    }

    /// The focused key's line and index within it.
    pub fn focus(&self) -> (usize, usize) {
        let lines = self.lines();
        let line = self.focus.0.min(lines.len() - 1);
        // The key over the focus's column, or the last one if the line is shorter (symbols).
        let c = self.focus.1;
        (line, lines[line].iter().position(|p| (p.column..p.column + p.span).contains(&c)).unwrap_or(lines[line].len() - 1))
    }

    /// Whether letters show (and type) in upper case.
    pub fn upper(&self) -> bool { self.upper && !self.symbols }

    /// A key's label as the keyboard is now: its character, or a word for the others. Shift's
    /// is in capitals while it is on; the layer key names the layer it switches to.
    pub fn label(&self, key: Key) -> String {
        match key {
            Key::Char(' ') => "space".into(),
            Key::Char(c) if self.upper() => c.to_ascii_uppercase().to_string(),
            Key::Char(c) => c.to_string(),
            Key::Shift if self.upper => "SHIFT".into(),
            Key::Shift => "shift".into(),
            Key::Symbols if self.symbols => "abc".into(),
            Key::Symbols => "#+=".into(),
            Key::Backspace => "delete".into(),
            Key::Done => "done".into(),
        }
    }

    /// The text as shown: dots for masked text unless revealed, with the caret's position.
    pub fn shown(&self) -> (String, usize) {
        let text = if self.masked && !self.revealed { "•".repeat(self.text.len()) } else { self.text.iter().collect() };
        (text, self.caret)
    }

    pub fn text(&self) -> String { self.text.iter().collect() }

    pub fn press(&mut self, p: Press) -> KeyOutcome {
        let lines = self.lines();
        let (line, index) = self.focus();
        let moved = |kb: &mut Self, to: (usize, usize)| { kb.focus = (to.0, lines[to.0][to.1].column); KeyOutcome::Changed };
        match p.button {
            Button::Left | Button::Right => {
                let n = lines[line].len();
                let delta = if p.button == Button::Left { n - 1 } else { 1 };
                moved(self, (line, (index + delta) % n))
            }
            Button::Up | Button::Down => {
                let n = lines.len();
                let to = (line + if p.button == Button::Up { n - 1 } else { 1 }) % n;
                // The key under the middle of this one (or the last, past a short line's end).
                let here = lines[line][index];
                let middle = 2 * here.column + here.span; // in half columns
                let i = lines[to].iter().position(|k| middle < 2 * (k.column + k.span)).unwrap_or(lines[to].len() - 1);
                moved(self, (to, i))
            }
            Button::A => self.key(lines[line][index].key),
            Button::B => self.key(Key::Backspace),
            Button::Y => self.key(Key::Shift),
            Button::X => self.key(Key::Symbols),
            Button::Start => self.key(Key::Done),
            Button::Select if self.masked => { self.revealed = !self.revealed; KeyOutcome::Changed }
            Button::L1 if self.caret > 0 => { self.caret -= 1; KeyOutcome::Changed }
            Button::R1 if self.caret < self.text.len() => { self.caret += 1; KeyOutcome::Changed }
            _ => KeyOutcome::None,
        }
    }

    fn key(&mut self, key: Key) -> KeyOutcome {
        match key {
            Key::Char(c) => {
                self.text.insert(self.caret, if self.upper() { c.to_ascii_uppercase() } else { c });
                self.caret += 1;
            }
            Key::Backspace if self.text.is_empty() => return KeyOutcome::Cancel,
            Key::Backspace if self.caret == 0 => return KeyOutcome::None,
            Key::Backspace => { self.caret -= 1; self.text.remove(self.caret); }
            Key::Shift => self.upper = !self.upper,
            // The focus stays on the same line and column (the lines have the same shape).
            Key::Symbols => self.symbols = !self.symbols,
            Key::Done => return KeyOutcome::Done(self.text()),
        }
        KeyOutcome::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(kb: &mut Keyboard, button: Button) -> KeyOutcome { kb.press(Press { button, repeat: false }) }

    fn focused(kb: &Keyboard) -> Key { let (l, i) = kb.focus(); kb.lines()[l][i].key }

    #[test]
    fn every_line_fills_the_width() {
        for symbols in [false, true] {
            let kb = Keyboard { symbols, ..Keyboard::new("", false) };
            for line in kb.lines() {
                let end = line.last().unwrap();
                assert!(end.column + end.span <= WIDTH);
                assert!(line.windows(2).all(|w| w[0].column + w[0].span == w[1].column));
            }
        }
    }

    #[test]
    fn types_with_shift_caret_and_backspace() {
        let mut kb = Keyboard::new("Password", true);
        assert_eq!(focused(&kb), Key::Char('q'));
        press(&mut kb, Button::A);
        press(&mut kb, Button::Y);
        press(&mut kb, Button::Right);
        press(&mut kb, Button::A); // W
        assert_eq!(kb.shown(), ("••".into(), 2));
        press(&mut kb, Button::Select);
        assert_eq!(kb.shown(), ("qW".into(), 2));
        press(&mut kb, Button::L1);
        press(&mut kb, Button::B); // deletes the q
        assert_eq!(kb.shown(), ("W".into(), 0));
        assert_eq!(press(&mut kb, Button::B), KeyOutcome::None); // nothing before the caret
        assert_eq!(press(&mut kb, Button::Start), KeyOutcome::Done("W".into()));
    }

    #[test]
    fn moving_wraps_and_lands_under_the_middle() {
        let mut kb = Keyboard::new("", false);
        press(&mut kb, Button::Left);
        assert_eq!(focused(&kb), Key::Char('p'));
        press(&mut kb, Button::Down);
        press(&mut kb, Button::Down);
        press(&mut kb, Button::Down); // from '_' (column 9) to done (columns 8–9)
        assert_eq!(focused(&kb), Key::Done);
        press(&mut kb, Button::Left);
        press(&mut kb, Button::Up); // from space (columns 4–7): the key over its middle, 'm'
        assert_eq!(focused(&kb), Key::Char('m'));
        press(&mut kb, Button::Up);
        press(&mut kb, Button::Up);
        press(&mut kb, Button::Up);
        press(&mut kb, Button::Up); // wraps to the last line
        assert_eq!(focused(&kb), Key::Char(' '));
    }

    #[test]
    fn symbols_and_cancel() {
        let mut kb = Keyboard::new("", false);
        assert_eq!(press(&mut kb, Button::B), KeyOutcome::Cancel);
        press(&mut kb, Button::X);
        press(&mut kb, Button::A);
        assert_eq!(kb.text(), "!");
        press(&mut kb, Button::Y); // shift doesn't change symbols
        press(&mut kb, Button::A);
        assert_eq!(kb.text(), "!!");
    }

    #[test]
    fn every_printable_character_can_be_typed() {
        let typable: std::collections::HashSet<char> = LETTERS.iter().chain(&SYMBOLS).flat_map(|l| l.chars())
            .flat_map(|c| [c, c.to_ascii_uppercase()]).collect();
        let missing: String = (' '..='~').filter(|c| !typable.contains(c)).collect();
        assert_eq!(missing, "");
    }
}
