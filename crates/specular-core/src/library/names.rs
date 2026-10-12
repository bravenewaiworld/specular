//! Game names from file names, which mostly follow No-Intro and Redump naming, e.g.
//! "Legend of Zelda, The - The Minish Cap (USA) (Rev 1)".

/// The name to show: everything from the first bracket on removed, and a trailing ", The" moved
/// to the front.
/// "Legend of Zelda, The - The Minish Cap (USA)" → "The Legend of Zelda - The Minish Cap".
pub fn display_title(stem: &str) -> String {
    // Cut at the first tag; a name that is nothing but tags is kept whole.
    let base = stem[..stem.find(['(', '[']).unwrap_or(stem.len())].trim();
    move_article(if base.is_empty() { stem.trim() } else { base })
}

/// Moves a trailing ", The" (of the first part of the title) to the front.
fn move_article(title: &str) -> String {
    let (head, tail) = match title.find(" - ") { Some(i) => (&title[..i], &title[i..]), None => (title, "") };
    match head.strip_suffix(", The") {
        Some(h) => format!("The {h}{tail}"),
        None => title.to_string(),
    }
}

/// The key games sort by: the display title, case-insensitive, without a leading "The ", with
/// numbers compared by value ("Mega Man 2" before "Mega Man 10").
pub fn sort_key(stem: &str) -> String {
    let title = display_title(stem).to_lowercase();
    let title = title.strip_prefix("the ").unwrap_or(&title);
    // Pad each run of digits to a fixed width so text order matches number order.
    let mut key = String::with_capacity(title.len() + 8);
    let mut digits = String::new();
    for c in title.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_digit() { digits.push(c); continue; }
        if !digits.is_empty() { key.push_str(&format!("{digits:0>8}")); digits.clear(); }
        if c != '\0' { key.push(c); }
    }
    key
}
