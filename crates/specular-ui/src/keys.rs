//! Where the on-screen keyboard's parts sit (`Layout::Keys`), in u (1 u = one pixel of the
//! Brick's panel), within the area a list gets (between the gutters, under the title): the text
//! entry along the top, then the keys in lines filling the rest, each key `span` of
//! `keyboard::WIDTH` columns wide.

use specular_core::keyboard::WIDTH;
use specular_core::list::{ListLayout, Rect};
use specular_core::model::{Row, RowKind};

/// The text entry's height, from the top of the area.
pub const ENTRY_HEIGHT: f32 = 84.0;
/// Space between keys; twice this separates the entry from the first line.
const GAP: f32 = 12.0;
/// Keys are no taller than this, however much room there is (a desktop window can be tall).
const MAX_KEY_HEIGHT: f32 = 96.0;

/// Lays out the keys of an area `width` × `height` u. Every key is drawn; nothing scrolls.
pub fn layout(rows: &[Row], width: f32, height: f32) -> ListLayout {
    let lines = rows.iter().filter_map(|r| match r.kind { RowKind::Key { line, .. } => Some(line + 1), _ => None }).max().unwrap_or(1);
    let top = ENTRY_HEIGHT + 2.0 * GAP;
    let pitch = ((height - top) / lines as f32).min(MAX_KEY_HEIGHT + GAP);
    let column = (width + GAP) / WIDTH as f32;
    let rects = rows.iter().map(|r| match r.kind {
        RowKind::Key { line, column: first, span } => Rect {
            x: first as f32 * column, y: top + line as f32 * pitch, width: span as f32 * column - GAP, height: pitch - GAP,
        },
        _ => Rect::default(),
    }).collect();
    ListLayout { rects, scroll: 0.0, drawn: 0..rows.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_fill_the_width_and_fit_below_the_entry() {
        let key = |line, column, span| Row::new(format!("{line}:{column}"), "k", RowKind::Key { line, column, span });
        let rows = vec![key(0, 0, 1), key(0, 9, 1), key(1, 0, 2), key(1, 8, 2)];
        // The Brick's list area: 928 × 512 u.
        let l = layout(&rows, 928.0, 512.0);
        assert_eq!(l.rects[0].x, 0.0);
        assert!((l.rects[1].x + l.rects[1].width - 928.0).abs() < 0.01);
        assert!((l.rects[3].x + l.rects[3].width - 928.0).abs() < 0.01);
        assert!(l.rects[0].y >= ENTRY_HEIGHT);
        assert!(l.rects[3].y + l.rects[3].height <= 512.0);
        assert_eq!(l.rects[2].height, MAX_KEY_HEIGHT); // two lines leave room to spare
    }
}
