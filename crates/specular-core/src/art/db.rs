//! A system's game table, trimmed from libretro-database by `tools/trim-db.py`: a `#` line naming
//! its source, then one line per ROM, `CRC32<TAB>serial<TAB>name`.

use std::collections::HashMap;
use std::fmt;

use super::Method;
use crate::library::display_title;

/// One system's games, to look a ROM up by checksum, serial or file name.
#[derive(Default)]
pub struct GameDb {
    /// The table's `#` line (its source and commit): identifications made with another table
    /// may be out of date.
    pub version: String,
    /// Every ROM's game name; the maps below hold indexes into it.
    names: Vec<Box<str>>,
    by_crc: HashMap<u32, u32>,
    by_code: HashMap<Box<str>, Vec<u32>>,
    /// By `normalise`d name, which is a game's title: its variants (regions, revisions, discs,
    /// betas…) share one.
    by_name: HashMap<String, Vec<u32>>,
}

/// Why `GameDb::by_name` found no game, for logs.
#[derive(Debug, PartialEq, Eq)]
pub enum NoMatch {
    /// No title holds every word of the name.
    Unknown,
    /// Several titles with the same numbers hold its words: how many, and the first three.
    Ambiguous(usize, Vec<String>),
    /// Only titles with other numbers hold its words (such as a sequel's): the first of them.
    Numbers(String),
}

impl fmt::Display for NoMatch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NoMatch::Unknown => write!(f, "no similar name"),
            NoMatch::Ambiguous(n, some) => write!(f, "ambiguous, {n} similar names (\"{}\"…)", some.join("\", \"")),
            NoMatch::Numbers(name) => write!(f, "numbers differ from \"{name}\""),
        }
    }
}

impl GameDb {
    /// Reads a table; malformed lines are skipped. A checksum listed twice keeps its last game.
    pub fn parse(text: &str) -> Self {
        let mut db = GameDb::default();
        for line in text.lines() {
            if let Some(comment) = line.strip_prefix('#') {
                if db.version.is_empty() { db.version = comment.trim().to_string(); }
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(crc), Some(code), Some(name)) = (fields.next(), fields.next(), fields.next()) else { continue };
            let Ok(crc) = u32::from_str_radix(crc, 16) else { continue };
            let i = db.names.len() as u32;
            db.names.push(name.into());
            db.by_crc.insert(crc, i);
            if !code.is_empty() { db.by_code.entry(code.into()).or_default().push(i); }
            db.by_name.entry(normalise(name)).or_default().push(i);
        }
        db
    }

    /// The game whose ROM has this CRC32.
    pub fn by_crc(&self, crc: u32) -> Option<&str> { self.by_crc.get(&crc).map(|&i| self.name(i)) }

    /// The preferred game with this serial: a ROM header's game code or a disc's serial
    /// (several revisions, regions and discs often share one).
    pub fn by_code(&self, code: &str) -> Option<&str> { self.best(self.by_code.get(code)?, &[]) }

    /// The game named exactly `stem`, else the preferred one with its `normalise`d name
    /// (`Method::Name`), else the preferred variant of the one title holding its words, in order,
    /// with the same numbers (`Method::FuzzyName`; see `fuzzy`). Variants are chosen by
    /// `preference`, with the ROM's own regions read from `stem`'s tags.
    pub fn by_name(&self, stem: &str) -> Result<(&str, Method), NoMatch> {
        let own = regions(stem);
        let key = normalise(stem);
        if let Some(variants) = self.by_name.get(&key) {
            let exact = variants.iter().map(|&i| self.name(i)).find(|n| *n == stem);
            return Ok((exact.or_else(|| self.best(variants, &own)).unwrap_or_default(), Method::Name));
        }
        let title = self.fuzzy(&key)?;
        Ok((self.best(&self.by_name[title], &own).unwrap_or_default(), Method::FuzzyName))
    }

    /// The one title holding every word of `key` (a `normalise`d name), in order, with the same
    /// numbers: so "zelda minish cap" finds "legend of zelda minish cap", but "final fantasy"
    /// never finds "final fantasy 2".
    fn fuzzy(&self, key: &str) -> Result<&str, NoMatch> {
        let words: Vec<&str> = key.split(' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() { return Err(NoMatch::Unknown); }
        let holds_words = |title: &str| { let mut t = title.split(' '); words.iter().all(|w| t.any(|x| x == *w)) };
        let (mut same, other): (Vec<&str>, Vec<&str>) =
            self.by_name.keys().map(String::as_str).filter(|t| holds_words(t)).partition(|t| numbers(t).eq(numbers(key)));
        if let [title] = same[..] { return Ok(title); }
        let shown = |title: &str| self.best(&self.by_name[title], &[]).unwrap_or_default().to_string();
        // Sorted, so logs are the same each time.
        same.sort_unstable();
        Err(match (same.len(), other.iter().min()) {
            (0, None) => NoMatch::Unknown,
            (0, Some(t)) => NoMatch::Numbers(shown(t)),
            (n, _) => NoMatch::Ambiguous(n, same.iter().take(3).map(|t| shown(t)).collect()),
        })
    }

    /// The most preferred of `candidates` (see `preference`); the shortest name breaks ties, as
    /// it has the fewest extra tags.
    fn best(&self, candidates: &[u32], own_regions: &[&str]) -> Option<&str> {
        candidates.iter().map(|&i| self.name(i)).min_by_key(|n| (preference(n, own_regions), n.len()))
    }

    fn name(&self, i: u32) -> &str { &self.names[i as usize] }
}

/// A name reduced to what two spellings of the same game share: no tags such as "(USA)" or
/// "[!]", every word "the" dropped, lower case, "&" as "and", roman numerals up to 39 as digits,
/// and only letters and digits between single spaces.
/// "Legend of Zelda, The - Minish Cap (USA)" → "legend of zelda minish cap";
/// "Mega Man II" → "mega man 2".
pub fn normalise(name: &str) -> String {
    let title = display_title(name).to_lowercase().replace('&', " and ");
    let words: Vec<String> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && *w != "the")
        .map(|w| roman(w).map_or_else(|| w.to_string(), |n| n.to_string())).collect();
    words.join(" ")
}

/// The value of a lower-case roman numeral from i to xxxix, written the usual way ("iiii" is
/// not one). Larger ones are left alone: words such as "mix" and "dc" are rarely numbers. A
/// lone "x" or "i" counts too, so "Mega Man X" would be the same name as "Mega Man 10".
fn roman(word: &str) -> Option<u32> {
    const ONES: [&str; 10] = ["", "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix"];
    let rest = word.trim_start_matches('x');
    let (tens, ones) = (word.len() - rest.len(), ONES.iter().position(|o| *o == rest)?);
    (tens <= 3 && tens + ones > 0).then_some((tens * 10 + ones) as u32)
}

/// The runs of digits in a `normalise`d name, where roman numerals are already digits: "3d",
/// "x2" and "64" each hold one.
fn numbers(key: &str) -> impl Iterator<Item = &str> { key.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()) }

/// The regions a name's tags give: No-Intro's ("(USA, Europe)"), or GoodTools' letters ("(U)",
/// "(UE)").
fn regions(name: &str) -> Vec<&'static str> {
    const NAMES: [&str; 22] = ["USA", "World", "Europe", "Japan", "Asia", "Australia", "Brazil", "Canada", "China", "France",
        "Germany", "Hong Kong", "Italy", "Korea", "Latin America", "Netherlands", "Russia", "Scandinavia", "Spain",
        "Sweden", "Taiwan", "UK"];
    let letter = |c| match c { 'U' => Some("USA"), 'E' => Some("Europe"), 'J' => Some("Japan"), 'W' => Some("World"), _ => None };
    let mut found = Vec::new();
    for tag in name.split('(').skip(1).filter_map(|t| t.split_once(')').map(|(t, _)| t)) {
        for word in tag.split(',').map(str::trim) {
            if let Some(r) = NAMES.iter().find(|r| **r == word) { found.push(*r); }
            else if let Some(rs) = word.chars().map(letter).collect::<Option<Vec<_>>>() { found.extend(rs); }
        }
    }
    found
}

/// How much to prefer a name among games that match equally well (lower is better): released
/// games before betas, prototypes, demos and unlicensed ones, then those of `own_regions`, then
/// by region (USA, World, Europe, Japan, then the rest), then originals before revisions.
fn preference(name: &str, own_regions: &[&str]) -> (bool, bool, usize, bool) {
    let tags = &name[name.find('(').unwrap_or(name.len())..];
    let unreleased = ["(Beta", "(Proto", "(Demo", "(Sample", "(Pirate", "(Unl)"].iter().any(|t| tags.contains(t));
    let foreign = !own_regions.is_empty() && !regions(name).iter().any(|r| own_regions.contains(r));
    let region = ["USA", "World", "Europe", "Japan"].iter().position(|r| tags.contains(r)).unwrap_or(4);
    (unreleased, foreign, region, tags.contains("(Rev"))
}
