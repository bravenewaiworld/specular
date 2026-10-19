//! Moving the focus through a screen's rows, and laying them out: the same rules on every list
//! and grid.

use crate::model::{ImageRef, Row, RowKind};

/// The focus after moving `delta` focusable rows from `focus` (negative = up). Headers are
/// skipped. `focus` may be a row the focus can't land on (a note being read, see `read_step`):
/// the move counts from where it is. At either end, a real press wraps around to the other end; a
/// repeat (a held button) stops at the end. Returns `focus` unchanged if no other row can take it.
pub fn step(rows: &[Row], focus: usize, delta: isize, repeat: bool) -> usize {
    let focusable: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].kind.focusable()).collect();
    if focusable.is_empty() { return focus; }
    // From a row between two focusable ones, count from the one above going down and from the
    // one below going up, so that one step reaches the nearest in that direction.
    let pos = focusable.partition_point(|&i| i < focus) as isize;
    let from = if delta > 0 && focusable.get(pos as usize) != Some(&focus) { pos - 1 } else { pos };
    let n = focusable.len() as isize;
    let target = from + delta;
    let target = if (0..n).contains(&target) {
        target
    } else if repeat || delta.abs() > 1 {
        // A held button, or a jump of several rows (paging), stops at the end, never moving the
        // focus backwards (as it would from a note past the last focusable row)…
        let to = focusable[target.clamp(0, n - 1) as usize];
        return if to != focus && (to < focus) == (delta > 0) { focus } else { to };
    } else {
        // …a single real press wraps.
        target.rem_euclid(n)
    };
    focusable[target as usize]
}

/// How far a press scrolls a note being read (u): three of its lines.
const READ_STEP: f32 = 3.0 * NOTE_LINE;

/// The focus after one press of Down (`down`) or Up in a list shown in a view `width` × `height`
/// u. If `focus` is a note being read, `reading` is how far the list is scrolled (u). Returns the
/// new focus and, when that is a note, how far to scroll the list to read it.
///
/// Notes take the focus only so that they can be read: a note the focus would pass stops it if
/// the note is wholly in view neither before the move nor after it (such as one too tall to show
/// whole together with the row next to it, or one above the first focusable row that is out of
/// view with the focus there). While a note has the focus, each press scrolls the list by
/// `READ_STEP`, until the row below the note (going down) or the note's own top (going up) is
/// wholly in view; the next press moves the focus on.
pub fn read_step(rows: &[Row], focus: usize, reading: Option<f32>, down: bool, repeat: bool, width: f32, height: f32) -> (usize, Option<f32>) {
    let rects = list_rects(rows, width);
    let max = max_scroll(&rects, height);
    let now = list_scroll(rows, &rects, focus, reading, height);
    if rows.get(focus).is_some_and(is_note) {
        let r = rects[focus];
        let to = if down {
            let below = rects.get(focus + 1).unwrap_or(&r);
            (now + READ_STEP).min(below.y + below.height - height)
        } else {
            (now - READ_STEP).max(r.y)
        }.clamp(0.0, max);
        let scrolls = if down { to > now + 0.5 } else { to < now - 0.5 };
        if scrolls { return (focus, Some(to)); }
    }
    let d = if down { 1 } else { -1 };
    let to = step(rows, focus, d, repeat);
    let after = if to == focus { now } else { list_scroll(rows, &rects, to, None, height) };
    // Check the rows passed on the way, nearest first, carrying on round the end of the list
    // when the press wraps.
    let n = rows.len() as isize;
    for k in 1..n {
        let i = focus as isize + k * d;
        if repeat && !(0..n).contains(&i) { break; }
        let i = i.rem_euclid(n) as usize;
        if i == to { break; }
        if is_note(&rows[i]) && !in_view(rects[i], now, height) && !in_view(rects[i], after, height) {
            return (i, Some(arrive(rects[i], now, height, down).clamp(0.0, max)));
        }
    }
    if to == focus { (focus, reading) } else { (to, None) }
}

/// The row the focus starts on in a list shown in a view `width` × `height` u: the first
/// focusable row, unless a note above it would not be wholly in view with the focus there; then
/// the first such note, read from the top of the list (scrolled to 0). With no focusable row, the
/// first note not wholly in view at the top of the list, if any.
pub fn first_stop(rows: &[Row], width: f32, height: f32) -> Option<usize> {
    let row = first(rows);
    let rects = list_rects(rows, width);
    let scroll = row.map_or(0.0, |r| list_scroll(rows, &rects, r, None, height));
    (0..row.unwrap_or(rows.len())).find(|&i| is_note(&rows[i]) && !in_view(rects[i], scroll, height)).or(row)
}

/// How far to scroll the list to bring note `r` into view from scroll `now`: the least change
/// that shows all of it, or, if it is taller than the view, the scroll that shows its top when
/// moving `down` and its bottom when moving up.
fn arrive(r: Rect, now: f32, height: f32, down: bool) -> f32 {
    let (bottom_in, top_in) = (r.y + r.height - height, r.y);
    if bottom_in <= top_in { now.clamp(bottom_in, top_in) } else if down { top_in } else { bottom_in }
}

/// Whether `r` is wholly in a view `height` u tall scrolled to `scroll` (to within half a u, so
/// rounding doesn't count).
fn in_view(r: Rect, scroll: f32, height: f32) -> bool { r.y >= scroll - 0.5 && r.y + r.height <= scroll + height + 0.5 }

/// Whether `row` is a note (`RowKind::Note`).
pub fn is_note(row: &Row) -> bool { matches!(row.kind, RowKind::Note { .. }) }

/// The focus after jumping to the first row of the next (`forward`) or previous run of rows that
/// share a `letter`: from anywhere among the M's, forward to the first N, back to the first L.
/// Rows must be grouped by letter (sorted by title). As with `step`, at either end a fresh press
/// wraps around and a repeat stops. Only focusable rows count.
pub fn jump(rows: &[Row], focus: usize, forward: bool, repeat: bool, letter: impl Fn(&Row) -> char) -> usize {
    let mut starts = vec![];
    let mut last = None;
    for (i, r) in rows.iter().enumerate().filter(|(_, r)| r.kind.focusable()) {
        let l = letter(r);
        if last != Some(l) { starts.push(i); }
        last = Some(l);
    }
    // The run the focus is in.
    let Some(run) = starts.iter().rposition(|&s| s <= focus) else { return first(rows).unwrap_or(focus) };
    let target = if forward { starts.get(run + 1) } else { run.checked_sub(1).map(|r| &starts[r]) };
    match target {
        Some(&to) => to,
        None if repeat => focus,
        None => if forward { starts[0] } else { *starts.last().unwrap() },
    }
}

/// The first focusable row.
pub fn first(rows: &[Row]) -> Option<usize> { rows.iter().position(|r| r.kind.focusable()) }

/// Heights of rows in a list, in the UI's base unit (1 u = one Brick panel pixel).
pub const ROW_HEIGHT: f32 = 92.0;
pub const HEADER_HEIGHT: f32 = 76.0;

/// Notes (`RowKind::Note`): the height of a line of their text, and the space above and below the
/// note (u; must match the UI's note padding in app.slint). One line makes a note as tall as a
/// header.
pub const NOTE_LINE: f32 = 44.0;
pub const NOTE_PADDING: f32 = 16.0;
/// The UI's left and right margins of list rows (its `Theme.gutter`), and the gap between a
/// note's text and its detail (app.slint's row spacing), u.
const GUTTER: f32 = 48.0;
const DETAIL_GAP: f32 = 20.0;
/// Assumed average character widths, in u: of a note's text (`Theme.secondary`, 33 u) and of its
/// detail (monospaced). On the wide side, so lines come out short rather than overflowing.
const NOTE_CHAR: f32 = 18.0;
const DETAIL_CHAR: f32 = 20.0;

/// A note's text in lines that fit a row `width` u wide (beside its detail, if any), split
/// between words (a word too long for a line is split where the line ends). The UI draws these
/// lines as they are, so the list layout knows the note's height.
pub fn note_lines(text: &str, detail: Option<&str>, width: f32) -> Vec<String> {
    let beside = detail.map_or(0.0, |d| d.chars().count() as f32 * DETAIL_CHAR + DETAIL_GAP);
    let max = ((width - 2.0 * GUTTER - beside) / NOTE_CHAR).max(8.0) as usize;
    let mut lines: Vec<String> = vec![];
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        match lines.last_mut() {
            Some(l) if l.chars().count() + 1 + word.len() <= max => { l.push(' '); l.extend(&word); continue; }
            _ => {}
        }
        while word.len() > max { lines.push(word.drain(..max).collect()); }
        lines.push(word.into_iter().collect());
    }
    if lines.is_empty() { lines.push(String::new()); }
    lines
}

/// How tall `row` is in a list `width` u wide.
fn row_height(row: &Row, width: f32) -> f32 {
    match &row.kind {
        RowKind::Header => HEADER_HEIGHT,
        RowKind::Note { detail } => note_lines(&row.label, detail.as_deref(), width).len() as f32 * NOTE_LINE + 2.0 * NOTE_PADDING,
        _ => ROW_HEIGHT,
    }
}

/// Grid cells: pictures at least this wide, this far apart, with a label underneath (u).
pub const CELL_MIN_WIDTH: f32 = 210.0;
pub const CELL_GAP: f32 = 28.0;
pub const CELL_LABEL_HEIGHT: f32 = 52.0;

/// How many grid columns fit in `width` u.
pub fn grid_columns(width: f32) -> usize { (((width + CELL_GAP) / (CELL_MIN_WIDTH + CELL_GAP)) as usize).max(1) }

/// How tall the pictures of `rows` are drawn, as a fraction of their width (in a grid's cells,
/// and beside a list): their `ImageRef::Art` shape, if they all have the same, else square; never
/// taller than square.
pub fn picture_shape(rows: &[Row]) -> f32 {
    let mut shapes = rows.iter().filter_map(|r| match &r.kind { RowKind::Image { image: ImageRef::Art { shape, .. }, .. } => Some(*shape), _ => None });
    let Some(first) = shapes.next() else { return 1.0 };
    if shapes.any(|s| s != first) { return 1.0; }
    (first.1 as f32 / first.0 as f32).min(1.0)
}

/// Where a row or cell sits, in u from the top left of the unscrolled list.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }

/// Where each row of a list (or cell of a grid) sits, how far it is scrolled, and which rows are
/// near enough to the view to be drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLayout {
    /// Each row's place, by row index.
    pub rects: Vec<Rect>,
    /// How far the list is scrolled up (u).
    pub scroll: f32,
    /// The rows to draw: those on screen and within one view height above or below, so a scroll
    /// animation of up to a view height never shows gaps (the UI jumps for longer scrolls).
    pub drawn: std::ops::Range<usize>,
}

/// How many rows can ever be in `ListLayout::drawn` for a view `width` × `height` u, so a UI can
/// keep this many row elements and reuse them. `grid` as for `layout`.
pub fn max_drawn(width: f32, height: f32, grid: Option<f32>) -> usize {
    if let Some(shape) = grid {
        let cols = grid_columns(width);
        let cell = (width - (cols - 1) as f32 * CELL_GAP) / cols as f32;
        cols * ((height * 3.0 / (cell * shape + CELL_LABEL_HEIGHT + CELL_GAP)).ceil() as usize + 2)
    } else {
        // No row is shorter than a header (a one-line note is as tall).
        (height * 3.0 / HEADER_HEIGHT).ceil() as usize + 2
    }
}

/// Lays out `rows` in a view `width` × `height` u, as a list, or as a grid of `grid_columns(width)`
/// columns whose pictures are `Some(shape)` (`picture_shape`) times as tall as wide. The focused
/// row sits about 40 % of the way down (see `list_scroll` for lists), without scrolling past
/// either end; with the focus on a note being read, a list is scrolled to `reading` instead
/// (see `read_step`; grids ignore it). In a grid every row is a cell, headers included (grid
/// screens have none).
pub fn layout(rows: &[Row], focus: usize, reading: Option<f32>, width: f32, height: f32, grid: Option<f32>) -> ListLayout {
    let (rects, scroll) = if let Some(shape) = grid {
        let cols = grid_columns(width);
        let cell = (width - (cols - 1) as f32 * CELL_GAP) / cols as f32;
        let pitch = cell * shape + CELL_LABEL_HEIGHT + CELL_GAP;
        let rects: Vec<Rect> = (0..rows.len()).map(|i| Rect {
            x: (i % cols) as f32 * (cell + CELL_GAP), y: CELL_GAP + (i / cols) as f32 * pitch, width: cell, height: cell * shape + CELL_LABEL_HEIGHT,
        }).collect();
        // A grid has a gap above and below its cells, room for the focused cell to lift into.
        let total = rects.last().map_or(0.0, |r| r.y + r.height) + CELL_GAP;
        let scroll = rects.get(focus).map_or(0.0, |r| (r.y + r.height / 2.0 - height * 0.4).clamp(0.0, (total - height).max(0.0)));
        (rects, scroll)
    } else {
        let rects = list_rects(rows, width);
        let scroll = list_scroll(rows, &rects, focus, reading, height);
        (rects, scroll)
    };
    let (from, to) = (scroll - height, scroll + 2.0 * height);
    let first = rects.iter().position(|r| r.y + r.height > from).unwrap_or(rects.len());
    let end = rects[first..].iter().position(|r| r.y >= to).map_or(rects.len(), |n| first + n);
    ListLayout { rects, scroll, drawn: first..end }
}

/// Where each row of a list `width` u wide sits.
fn list_rects(rows: &[Row], width: f32) -> Vec<Rect> {
    rows.iter().scan(0.0, |y, r| {
        let h = row_height(r, width);
        let rect = Rect { x: 0.0, y: *y, width, height: h };
        *y += h;
        Some(rect)
    }).collect()
}

/// How far a list laid out in `rects` can scroll in a view `height` u tall.
fn max_scroll(rects: &[Rect], height: f32) -> f32 { (rects.last().map_or(0.0, |r| r.y + r.height) - height).max(0.0) }

/// How far a list laid out in `rects` is scrolled in a view `height` u tall, with the focus on
/// `focus`: `reading` if it is a note being read, else so the focused row sits about 40 % of the
/// way down. The notes right above a focused row (which usually explain it) are kept in view as
/// far as they can be while the row itself stays wholly in view.
fn list_scroll(rows: &[Row], rects: &[Rect], focus: usize, reading: Option<f32>, height: f32) -> f32 {
    let max = max_scroll(rects, height);
    if let Some(s) = reading { return s.clamp(0.0, max); }
    let Some(r) = rects.get(focus) else { return 0.0 };
    let notes_top = (0..focus).rev().take_while(|&i| is_note(&rows[i])).last().map_or(r.y, |i| rects[i].y);
    let centred = r.y + r.height / 2.0 - height * 0.4;
    centred.min(notes_top.max(r.y + r.height - height)).clamp(0.0, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RowKind;

    fn rows() -> Vec<Row> {
        let text = |id: &str| Row::new(id, id, RowKind::Text { detail: None });
        vec![Row::new("h1", "Systems", RowKind::Header), text("a"), text("b"), Row::new("h2", "Device", RowKind::Header), text("c")]
    }

    #[test]
    fn skips_headers() {
        let r = rows();
        assert_eq!(step(&r, 1, 1, false), 2);
        assert_eq!(step(&r, 2, 1, false), 4);
        assert_eq!(step(&r, 4, -1, false), 2);
    }

    #[test]
    fn wraps_on_press_not_on_repeat() {
        let r = rows();
        assert_eq!(step(&r, 4, 1, false), 1);
        assert_eq!(step(&r, 4, 1, true), 4);
        assert_eq!(step(&r, 1, -1, false), 4);
        assert_eq!(step(&r, 1, -1, true), 1);
    }

    #[test]
    fn pages_stop_at_the_ends() {
        let r = rows();
        assert_eq!(step(&r, 1, 10, false), 4);
        assert_eq!(step(&r, 4, -10, false), 1);
    }

    #[test]
    fn jumps_between_letters_wrapping_on_press_not_on_repeat() {
        let r: Vec<Row> = ["Advance Wars", "Asterix", "Bomberman", "Castlevania", "Contra"].iter().map(|t| Row::new(*t, *t, RowKind::Text { detail: None })).collect();
        let letter = |r: &Row| r.label.chars().next().unwrap();
        assert_eq!(jump(&r, 1, true, false, letter), 2);
        assert_eq!(jump(&r, 2, true, false, letter), 3);
        assert_eq!(jump(&r, 4, false, false, letter), 2); // from the middle of the C's, back to the B
        assert_eq!(jump(&r, 4, true, true, letter), 4); // held: no wrap
        assert_eq!(jump(&r, 4, true, false, letter), 0);
        assert_eq!(jump(&r, 1, false, true, letter), 1);
        assert_eq!(jump(&r, 1, false, false, letter), 3);
        assert_eq!(jump(&r[..2], 1, true, false, letter), 0); // one letter: back to its start
        assert_eq!(jump(&[], 0, true, false, letter), 0);
    }

    #[test]
    fn focus_on_a_header_moves_to_the_first_row() {
        assert_eq!(step(&rows(), 0, 1, false), 1);
        assert_eq!(first(&rows()), Some(1));
        assert_eq!(step(&[], 0, 1, false), 0);
    }

    #[test]
    fn layout_keeps_the_focus_in_view_and_clamps() {
        let r: Vec<Row> = (0..20).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        let l = layout(&r, 0, None, 900.0, 500.0, None);
        assert_eq!((l.rects[1].y, l.scroll), (92.0, 0.0));
        let l = layout(&r, 10, None, 900.0, 500.0, None);
        assert_eq!(l.scroll, 10.0 * 92.0 + 46.0 - 200.0);
        let l = layout(&r, 19, None, 900.0, 500.0, None);
        assert_eq!(l.scroll, 20.0 * 92.0 - 500.0);
        assert_eq!(layout(&r[..3], 2, None, 900.0, 500.0, None).scroll, 0.0); // short lists don't scroll
    }

    #[test]
    fn visible_rows_cover_the_view_and_a_margin() {
        let r: Vec<Row> = (0..3000).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        for grid in [None, Some(1.0), Some(0.5)] {
            let l = layout(&r, 1500, None, 928.0, 500.0, grid);
            let (top, bottom) = (l.scroll, l.scroll + 500.0);
            let v = l.drawn.clone();
            let (first, last) = (l.rects[v.start], l.rects[v.end - 1]);
            assert!(first.y <= top - 500.0 + first.height && last.y + last.height >= bottom + 500.0);
            assert!(v.len() <= max_drawn(928.0, 500.0, grid), "{grid:?}");
            assert_eq!(layout(&r, 0, None, 928.0, 500.0, grid).drawn.start, 0);
            assert_eq!(layout(&r, 2999, None, 928.0, 500.0, grid).drawn.end, 3000);
        }
    }

    #[test]
    fn notes_wrap_between_words_and_are_as_tall_as_their_lines() {
        // 1024 u wide (the Brick): (1024 - 96) / 18 = 51 characters a line.
        let text = "Erases everything on the internal storage, including TrimUI's original system.";
        let lines = note_lines(text, None, 1024.0);
        assert_eq!(lines, ["Erases everything on the internal storage,", "including TrimUI's original system."]);
        // 400 u: 16 characters a line.
        assert_eq!(note_lines("a/long/path/without/spaces then", None, 400.0), ["a/long/path/with", "out/spaces then"]);
        assert_eq!(note_lines("Checking the internal storage", Some("Done"), 600.0), ["Checking the internal", "storage"]);
        assert_eq!(note_lines("", None, 1024.0), [""]);
        let r = [Row::note("a", text), Row::new("b", "x", RowKind::Action { value: None })];
        let l = layout(&r, 1, None, 1024.0, 500.0, None);
        assert_eq!(l.rects[1].y, 2.0 * NOTE_LINE + 2.0 * NOTE_PADDING);
        assert_eq!(NOTE_LINE + 2.0 * NOTE_PADDING, HEADER_HEIGHT); // see max_drawn
        assert_eq!(first(&r), Some(1)); // notes never take the focus
    }

    /// A note of `lines` lines, 1024 u wide (ten four-letter words a line).
    fn note(id: &str, lines: usize) -> Row { Row::note(id, vec!["word"; 10 * lines].join(" ")) }
    fn action(id: &str) -> Row { Row::new(id, id, RowKind::Action { value: None }) }

    #[test]
    fn a_note_above_the_first_row_is_read_before_it() {
        // An 8-line note (384 u) and its button, in a view 400 u tall: with the button focused,
        // the note's top is out of view.
        let mut r = vec![note("n", 8), action("ok")];
        r.extend((0..10).map(|i| action(&i.to_string())));
        let (w, h) = (1024.0, 400.0);
        assert_eq!(layout(&r, 1, None, w, h, None).scroll, 76.0);
        assert_eq!(first_stop(&r, w, h), Some(0));
        // Down scrolls until the button is wholly in view, then moves to it without scrolling.
        assert_eq!(read_step(&r, 0, Some(0.0), true, false, w, h), (0, Some(76.0)));
        assert_eq!(read_step(&r, 0, Some(76.0), true, false, w, h), (1, None));
        // Up from the button stops on the note (all of it shows), and the next press wraps.
        assert_eq!(read_step(&r, 1, None, false, false, w, h), (0, Some(0.0)));
        assert_eq!(read_step(&r, 0, Some(0.0), false, false, w, h), (11, None));
        assert_eq!(read_step(&r, 0, Some(0.0), false, true, w, h), (0, Some(0.0))); // held: stays
        assert_eq!(step(&r, 0, -1, true), 0);
        // In a view tall enough to show it whole with its button, it is passed.
        assert_eq!(first_stop(&r, w, 500.0), Some(1));
        assert_eq!(read_step(&r, 1, None, false, false, w, 500.0), (11, None));
    }

    #[test]
    fn a_note_taller_than_the_view_scrolls_a_few_lines_a_press() {
        let r = [action("a"), note("n", 15), action("b")];
        let (w, h) = (1024.0, 400.0);
        let mut at = (0, None);
        let mut seen = vec![];
        while at.0 != 2 {
            at = read_step(&r, at.0, at.1, true, false, w, h);
            seen.push(at);
        }
        assert_eq!(seen, [(1, Some(92.0)), (1, Some(224.0)), (1, Some(356.0)), (1, Some(476.0)), (2, None)]);
        assert_eq!(layout(&r, 2, None, w, h, None).scroll, 476.0);
        // Coming up, it is read from its bottom.
        assert_eq!(read_step(&r, 2, None, false, false, w, h), (1, Some(384.0)));
    }

    #[test]
    fn notes_seen_whole_are_passed_and_stay_above_their_row() {
        let text = |id: &str| Row::new(id, id, RowKind::Text { detail: None });
        let r = [text("a"), note("n", 1), text("b")];
        assert_eq!(read_step(&r, 0, None, true, false, 1024.0, 500.0), (2, None));
        // A 4-line note right above the focused row stays in view, though the row sits lower
        // than 40 % of the way down.
        let mut r: Vec<Row> = (0..10).map(|i| text(&i.to_string())).collect();
        r.extend([note("n", 4), action("ok")]);
        r.extend((0..10).map(|i| text(&format!("b{i}"))));
        assert_eq!(layout(&r, 11, None, 1024.0, 500.0, None).scroll, 920.0);
    }

    #[test]
    fn pictures_take_the_shape_their_rows_share() {
        let art = |shape| Row::new("g", "g", RowKind::Image { image: ImageRef::Art { system: "gba", rom: "g".into(), kind: crate::art::ArtKind::TitleScreen, shape }, detail: None });
        assert_eq!(picture_shape(&[art((240, 160)), art((240, 160))]), 160.0 / 240.0);
        assert_eq!(picture_shape(&[art((240, 160)), art((1, 1))]), 1.0); // mixed: square
        assert_eq!(picture_shape(&[art((224, 288))]), 1.0); // never taller than square
        assert_eq!(picture_shape(&rows()), 1.0);
        let l = layout(&[art((240, 160))], 0, None, 928.0, 500.0, Some(0.5));
        assert_eq!(l.rects[0].height, l.rects[0].width * 0.5 + CELL_LABEL_HEIGHT);
    }

    #[test]
    fn grid_cells_fill_the_width() {
        let r: Vec<Row> = (0..9).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        assert_eq!(grid_columns(928.0), 4); // the Brick: 1024 u less two 48 u gutters
        let l = layout(&r, 0, None, 928.0, 500.0, Some(1.0));
        let cols = grid_columns(928.0);
        let last_in_row = l.rects[cols - 1];
        assert!((last_in_row.x + last_in_row.width - 928.0).abs() < 0.01);
        assert_eq!(l.rects[cols].x, 0.0);
        assert!(l.rects[cols].y > l.rects[0].y + l.rects[0].height);
    }
}
