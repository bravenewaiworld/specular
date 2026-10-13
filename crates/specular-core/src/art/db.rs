//! A system's game table, trimmed from libretro-database by `tools/trim-db.py`: a `#` line naming
//! its source, then one line per ROM, `CRC32<TAB>serial<TAB>name`.

use std::collections::HashMap;

use crate::library::display_title;

/// One system's games, to look a ROM up by checksum, header code or file name.
#[derive(Default)]
pub struct GameDb {
    /// The table's `#` line (its source and commit): identifications made with another table
    /// may be out of date.
    pub version: String,
    /// Every ROM's game name; the maps below hold indexes into it.
    names: Vec<Box<str>>,
    by_crc: HashMap<u32, u32>,
    by_code: HashMap<Box<str>, Vec<u32>>,
    /// By `normalise`d name.
    by_name: HashMap<String, Vec<u32>>,
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

    /// The preferred game with this serial, which is what a ROM header's game code is called in
    /// the table (several revisions and regions often share one).
    pub fn by_code(&self, code: &str) -> Option<&str> { self.best(self.by_code.get(code)?) }

    /// The game named exactly `stem`, else the preferred one whose name matches it once
    /// `normalise`d.
    pub fn by_name(&self, stem: &str) -> Option<&str> {
        let candidates = self.by_name.get(&normalise(stem))?;
        candidates.iter().map(|&i| self.name(i)).find(|n| *n == stem).or_else(|| self.best(candidates))
    }

    /// The most preferred of `candidates` (see `preference`); the shortest name breaks ties, as
    /// it has the fewest extra tags.
    fn best(&self, candidates: &[u32]) -> Option<&str> {
        candidates.iter().map(|&i| self.name(i)).min_by_key(|n| (preference(n), n.len()))
    }

    fn name(&self, i: u32) -> &str { &self.names[i as usize] }
}

/// A name reduced to what two spellings of the same game share: no tags such as "(USA)" or
/// "[!]", every word "the" dropped, lower case, "&" as "and", and only letters and digits
/// between single spaces. "Legend of Zelda, The - Minish Cap (USA)" → "legend of zelda minish cap".
pub fn normalise(name: &str) -> String {
    let title = display_title(name).to_lowercase().replace('&', " and ");
    let words: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && *w != "the").collect();
    words.join(" ")
}

/// How much to prefer a name among games that match equally well (lower is better): released
/// games before betas, prototypes, demos and unlicensed ones, then by region (USA, World, Europe,
/// Japan, then the rest), then originals before revisions.
fn preference(name: &str) -> (bool, usize, bool) {
    let tags = &name[name.find('(').unwrap_or(name.len())..];
    let unreleased = ["(Beta", "(Proto", "(Demo", "(Sample", "(Pirate", "(Unl)"].iter().any(|t| tags.contains(t));
    let region = ["USA", "World", "Europe", "Japan"].iter().position(|r| tags.contains(r)).unwrap_or(4);
    (unreleased, region, tags.contains("(Rev"))
}
