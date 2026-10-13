//! Moving the focus through a screen's rows, and laying them out: the same rules on every list
//! and grid.

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

/// Grid cells: square pictures at least this wide, this far apart, with a label underneath (u).
pub const CELL_MIN_WIDTH: f32 = 210.0;
pub const CELL_GAP: f32 = 28.0;
pub const CELL_LABEL_HEIGHT: f32 = 52.0;

/// How many grid columns fit in `width` u.
pub fn grid_columns(width: f32) -> usize { (((width + CELL_GAP) / (CELL_MIN_WIDTH + CELL_GAP)) as usize).max(1) }

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
/// keep this many row elements and reuse them.
pub fn max_drawn(width: f32, height: f32, grid: bool) -> usize {
    if grid {
        let cols = grid_columns(width);
        let cell = (width - (cols - 1) as f32 * CELL_GAP) / cols as f32;
        cols * ((height * 3.0 / (cell + CELL_LABEL_HEIGHT + CELL_GAP)).ceil() as usize + 2)
    } else {
        // Assumes no row is shorter than a header.
        (height * 3.0 / HEADER_HEIGHT).ceil() as usize + 2
    }
}

/// Lays out `rows` in a view `width` × `height` u, as a list or as a grid of `grid_columns(width)`
/// columns, so the focused row sits about 40 % of the way down, without scrolling past either
/// end. In a grid every row is a cell, headers included (grid screens have none).
pub fn layout(rows: &[Row], focus: usize, width: f32, height: f32, grid: bool) -> ListLayout {
    let rects: Vec<Rect> = if grid {
        let cols = grid_columns(width);
        let cell = (width - (cols - 1) as f32 * CELL_GAP) / cols as f32;
        let pitch = cell + CELL_LABEL_HEIGHT + CELL_GAP;
        (0..rows.len()).map(|i| Rect {
            x: (i % cols) as f32 * (cell + CELL_GAP), y: CELL_GAP + (i / cols) as f32 * pitch, width: cell, height: cell + CELL_LABEL_HEIGHT,
        }).collect()
    } else {
        rows.iter().scan(0.0, |y, r| {
            let h = if r.kind.focusable() { ROW_HEIGHT } else { HEADER_HEIGHT };
            let rect = Rect { x: 0.0, y: *y, width, height: h };
            *y += h;
            Some(rect)
        }).collect()
    };
    // A grid has a gap above and below its cells, room for the focused cell to lift into.
    let total = rects.last().map_or(0.0, |r| r.y + r.height) + if grid { CELL_GAP } else { 0.0 };
    let scroll = rects.get(focus).map_or(0.0, |r| (r.y + r.height / 2.0 - height * 0.4).clamp(0.0, (total - height).max(0.0)));
    let (from, to) = (scroll - height, scroll + 2.0 * height);
    let first = rects.iter().position(|r| r.y + r.height > from).unwrap_or(rects.len());
    let end = rects[first..].iter().position(|r| r.y >= to).map_or(rects.len(), |n| first + n);
    ListLayout { rects, scroll, drawn: first..end }
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
        let l = layout(&r, 0, 900.0, 500.0, false);
        assert_eq!((l.rects[1].y, l.scroll), (92.0, 0.0));
        let l = layout(&r, 10, 900.0, 500.0, false);
        assert_eq!(l.scroll, 10.0 * 92.0 + 46.0 - 200.0);
        let l = layout(&r, 19, 900.0, 500.0, false);
        assert_eq!(l.scroll, 20.0 * 92.0 - 500.0);
        assert_eq!(layout(&r[..3], 2, 900.0, 500.0, false).scroll, 0.0); // short lists don't scroll
    }

    #[test]
    fn visible_rows_cover_the_view_and_a_margin() {
        let r: Vec<Row> = (0..3000).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        for grid in [false, true] {
            let l = layout(&r, 1500, 928.0, 500.0, grid);
            let (top, bottom) = (l.scroll, l.scroll + 500.0);
            let v = l.drawn.clone();
            let (first, last) = (l.rects[v.start], l.rects[v.end - 1]);
            assert!(first.y <= top - 500.0 + first.height && last.y + last.height >= bottom + 500.0);
            assert!(v.len() <= max_drawn(928.0, 500.0, grid), "{grid}");
            assert_eq!(layout(&r, 0, 928.0, 500.0, grid).drawn.start, 0);
            assert_eq!(layout(&r, 2999, 928.0, 500.0, grid).drawn.end, 3000);
        }
    }

    #[test]
    fn grid_cells_fill_the_width() {
        let r: Vec<Row> = (0..9).map(|i| Row::new(i.to_string(), "x", RowKind::Text { detail: None })).collect();
        assert_eq!(grid_columns(928.0), 4); // the Brick: 1024 u less two 48 u gutters
        let l = layout(&r, 0, 928.0, 500.0, true);
        let cols = grid_columns(928.0);
        let last_in_row = l.rects[cols - 1];
        assert!((last_in_row.x + last_in_row.width - 928.0).abs() < 0.01);
        assert_eq!(l.rects[cols].x, 0.0);
        assert!(l.rects[cols].y > l.rects[0].y + l.rects[0].height);
    }
}
