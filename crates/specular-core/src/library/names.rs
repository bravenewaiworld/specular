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

/// The key games sort by: the display title in lower case without accents, without a leading
/// "The ", with numbers compared by value ("Mega Man 2" before "Mega Man 10").
pub fn sort_key(stem: &str) -> String {
    let mut title = String::new();
    for c in display_title(stem).chars() { push_folded(&mut title, c); }
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

/// The letter a game is filed under in the letter index (L2 and R2 jump between letters): the
/// first letter of its display title `title` in upper case, without a leading "The " or an
/// accent, so it agrees with `sort_key`'s order; '#' for a digit, a symbol or a letter outside
/// A–Z.
pub fn index_letter(title: &str) -> char {
    let title = title.trim_start();
    let title = if title.get(..4).is_some_and(|the| the.eq_ignore_ascii_case("the ")) { &title[4..] } else { title };
    let mut first = String::new();
    // (A combining accent folds to nothing.)
    for c in title.chars() { push_folded(&mut first, c); if !first.is_empty() { break; } }
    match first.chars().next() { Some(c) if c.is_ascii_lowercase() => c.to_ascii_uppercase(), _ => '#' }
}

/// Pushes `c` lower-cased and without its accent ("É" → "e", "Æ" → "ae"), so names sort and match
/// as people read them. Covers the Latin letters game names use, and drops combining accents (file
/// names written by macOS have them); other characters are only lower-cased.
pub(crate) fn push_folded(out: &mut String, c: char) {
    // (Most names are ASCII: this halves the time a search takes.)
    if c.is_ascii() { return out.push(c.to_ascii_lowercase()); }
    for c in c.to_lowercase() {
        let base = match c {
            '\u{300}'..='\u{36f}' => continue,
            'à'..='å' | 'ā'..='ą' => 'a',
            'ç' | 'ć'..='č' => 'c',
            'ð' | 'ď'..='đ' => 'd',
            'è'..='ë' | 'ē'..='ě' => 'e',
            'ĝ'..='ģ' => 'g',
            'ĥ'..='ħ' => 'h',
            'ì'..='ï' | 'ĩ'..='ı' => 'i',
            'ĵ' => 'j',
            'ķ'..='ĸ' => 'k',
            'ĺ'..='ł' => 'l',
            'ñ' | 'ń'..='ŋ' => 'n',
            'ò'..='ö' | 'ø' | 'ō'..='ő' => 'o',
            'ŕ'..='ř' => 'r',
            'ś'..='š' => 's',
            'ţ'..='ŧ' => 't',
            'ù'..='ü' | 'ũ'..='ų' => 'u',
            'ŵ' => 'w',
            'ý' | 'ÿ' | 'ŷ' => 'y',
            'ź'..='ž' => 'z',
            'æ' => { out.push_str("ae"); continue; }
            'œ' => { out.push_str("oe"); continue; }
            'ß' => { out.push_str("ss"); continue; }
            'þ' => { out.push_str("th"); continue; }
            c => c,
        };
        out.push(base);
    }
}
