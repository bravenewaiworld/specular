//! Moving the focus through a screen's rows: the same rules on every list and grid.

use crate::model::Row;

/// The focus after moving `delta` focusable rows from `focus` (negative = up). Headers are
/// skipped. At either end, a real press wraps around to the other end; a repeat (a held button)
/// stops at the end. Returns `focus` unchanged if no other row can take it.
pub fn step(rows: &[Row], focus: usize, delta: isize, repeat: bool) -> usize {
    let focusable: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].kind.focusable()).collect();
    let Some(pos) = focusable.iter().position(|&i| i == focus) else { return first(rows).unwrap_or(focus) };
    let n = focusable.len() as isize;
    let target = pos as isize + delta;
    let target = if (0..n).contains(&target) {
        target
    } else if repeat || delta.abs() > 1 {
        // A held button, or a jump of several rows (paging), stops at the end…
        target.clamp(0, n - 1)
    } else {
        // …a single real press wraps.
        target.rem_euclid(n)
    };
    focusable[target as usize]
}

/// The first focusable row.
pub fn first(rows: &[Row]) -> Option<usize> { rows.iter().position(|r| r.kind.focusable()) }

/// Heights of rows in a list, in the UI's base unit (1 u = one Brick panel pixel).
pub const ROW_HEIGHT: f32 = 92.0;
pub const HEADER_HEIGHT: f32 = 76.0;

/// Where each row of a list sits, how far the list is scrolled, and which rows are near enough
/// to the view to be drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLayout {
    /// Each row's top, from the top of the list (u).
    pub tops: Vec<f32>,
    pub heights: Vec<f32>,
    /// How far the list is scrolled up (u).
    pub scroll: f32,
    /// The rows within one view height above or below the view: those the UI must have ready
    /// (the margin covers the scroll animation between two positions).
    pub visible: std::ops::Range<usize>,
}

/// How many rows can ever be in `ListLayout::visible` for a view `view_height` u tall: a UI can
/// keep this many row elements and reuse them.
pub fn max_visible(view_height: f32) -> usize { (view_height * 3.0 / HEADER_HEIGHT).ceil() as usize + 2 }

/// Lays out `rows` in a view `view_height` u tall so the focused row sits about 40 % of the way
/// down, without scrolling past either end.
pub fn layout(rows: &[Row], focus: usize, view_height: f32) -> ListLayout {
    let heights: Vec<f32> = rows.iter().map(|r| if r.kind.focusable() { ROW_HEIGHT } else { HEADER_HEIGHT }).collect();
    let tops: Vec<f32> = heights.iter().scan(0.0, |y, h| { let top = *y; *y += h; Some(top) }).collect();
    let total: f32 = heights.iter().sum();
    let scroll = match (tops.get(focus), heights.get(focus)) {
        (Some(top), Some(h)) => (top + h / 2.0 - view_height * 0.4).clamp(0.0, (total - view_height).max(0.0)),
        _ => 0.0,
    };
    let (from, to) = (scroll - view_height, scroll + 2.0 * view_height);
    let first = tops.iter().zip(&heights).position(|(t, h)| t + h > from).unwrap_or(rows.len());
    let end = tops[first..].iter().position(|&t| t >= to).map_or(rows.len(), |n| first + n);
    ListLayout { tops, heights, scroll, visible: first..end }
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
    fn focus_on_a_header_moves_to_the_first_row() {
        assert_eq!(step(&rows(), 0, 1, false), 1);
        assert_eq!(first(&rows()), Some(1));
        assert_eq!(step(&[], 0, 1, false), 0);
    }

    #[test]
    fn layout_keeps_the_focus_in_view_and_clamps() {
        let r: Vec<Row> = (0..20).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        let l = layout(&r, 0, 500.0);
        assert_eq!((l.tops[1], l.scroll), (92.0, 0.0));
        let l = layout(&r, 10, 500.0);
        assert_eq!(l.scroll, 10.0 * 92.0 + 46.0 - 200.0);
        let l = layout(&r, 19, 500.0);
        assert_eq!(l.scroll, 20.0 * 92.0 - 500.0);
        assert_eq!(layout(&r[..3], 2, 500.0).scroll, 0.0); // short lists don't scroll
    }

    #[test]
    fn visible_rows_cover_the_view_and_a_margin() {
        let r: Vec<Row> = (0..3000).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        let l = layout(&r, 1500, 500.0);
        let (top, bottom) = (l.scroll, l.scroll + 500.0);
        let v = l.visible.clone();
        assert!(l.tops[v.start] <= top - 500.0 + 92.0 && l.tops[v.end - 1] + 92.0 >= bottom + 500.0);
        assert!(v.len() <= max_visible(500.0));
        assert_eq!(layout(&r, 0, 500.0).visible.start, 0);
        assert_eq!(layout(&r, 2999, 500.0).visible.end, 3000);
    }
}
