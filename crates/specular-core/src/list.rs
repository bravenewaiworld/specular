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
}
