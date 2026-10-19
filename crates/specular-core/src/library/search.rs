//! Finding games by name, in every system at once.

use super::Game;
use super::names::push_folded;

/// The games whose titles match `query`, best first: titles that start with it, then titles with a
/// word that starts with it (or, for a query of several words, a word starting with each, in any
/// order), then titles that contain it anywhere, even across words; by title within each. Case,
/// accents, spaces and punctuation are ignored, so "pokemon fire" finds "Pokémon FireRed" and
/// "spiderman" finds "Spider-Man". Nothing matches a query without a letter or digit.
pub fn search<'a>(games: &'a [Game], query: &str) -> Vec<&'a Game> {
    let mut q = Folded::default();
    q.set(query);
    if q.text.is_empty() { return vec![]; }
    let words: Vec<&str> = q.words().collect();
    // One buffer for every title: this runs on the UI thread, over the whole library.
    let mut title = Folded::default();
    let mut found: Vec<(u8, &Game)> = games.iter().filter_map(|g| {
        title.set(&g.title);
        title.rank(&q.text, &words).map(|rank| (rank, g))
    }).collect();
    // (A stable sort: the same title in several systems keeps the order of `games`, by system.)
    found.sort_by(|(r1, g1), (r2, g2)| (r1, &g1.sort_key).cmp(&(r2, &g2.sort_key)));
    found.into_iter().map(|(_, g)| g).collect()
}

/// A name as it is matched: folded (`push_folded`), only its letters and digits, and where each
/// of its words starts in that text.
#[derive(Default)]
struct Folded { text: String, starts: Vec<usize> }

impl Folded {
    fn set(&mut self, name: &str) {
        self.text.clear();
        self.starts.clear();
        let mut in_word = false;
        for c in name.chars() {
            // An apostrophe doesn't end a word ("Kirby's").
            if c == '\'' || c == '’' { continue; }
            let at = self.text.len();
            push_folded(&mut self.text, c);
            if self.text.len() == at { continue; } // a combining accent
            if self.text[at..].chars().all(char::is_alphanumeric) {
                if !in_word { self.starts.push(at); }
                in_word = true;
            } else {
                self.text.truncate(at);
                in_word = false;
            }
        }
    }

    fn words(&self) -> impl Iterator<Item = &str> {
        self.starts.iter().enumerate().map(|(i, &s)| &self.text[s..self.starts.get(i + 1).copied().unwrap_or(self.text.len())])
    }

    /// How well this title matches the query `q` (folded, and split into `words`): 0 if the title
    /// starts with it, 1 if a word does (or each query word starts some word), 2 if it is
    /// anywhere; None if not at all.
    fn rank(&self, q: &str, words: &[&str]) -> Option<u8> {
        let word_starts_with = |w: &str| self.starts.iter().any(|&s| self.text[s..].starts_with(w));
        if self.text.starts_with(q) { Some(0) }
        else if word_starts_with(q) || (words.len() > 1 && words.iter().all(|w| word_starts_with(w))) { Some(1) }
        else if self.text.contains(q) { Some(2) }
        else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(games: &[Game], query: &str) -> Vec<String> { search(games, query).iter().map(|g| g.title.clone()).collect() }

    fn games(titles: &[(&'static str, &str)]) -> Vec<Game> {
        titles.iter().map(|&(system, t)| Game { system, path: format!("{system}/{t}").into(), title: t.into(), sort_key: super::super::sort_key(t) }).collect()
    }

    #[test]
    fn best_matches_come_first() {
        let g = games(&[("gba", "Advance Wars"), ("gba", "Mario Kart - Super Circuit"), ("gba", "Super Mario Advance"), ("gb", "Wario Land")]);
        // A title starting with it, then a word starting with it, then anywhere in a word.
        assert_eq!(titles(&g, "mario"), ["Mario Kart - Super Circuit", "Super Mario Advance"]);
        assert_eq!(titles(&g, "ario"), ["Mario Kart - Super Circuit", "Super Mario Advance", "Wario Land"]);
        assert_eq!(titles(&g, "adv"), ["Advance Wars", "Super Mario Advance"]);
        assert_eq!(titles(&g, "kart mario"), ["Mario Kart - Super Circuit"]); // every word, any order
        assert_eq!(titles(&g, "zelda"), Vec::<String>::new());
        assert_eq!(titles(&g, " -- "), Vec::<String>::new());
    }

    #[test]
    fn case_accents_spaces_and_punctuation_are_ignored() {
        let g = games(&[("gba", "Pokémon - FireRed Version"), ("gba", "Spider-Man"), ("gb", "Kirby's Dream Land"), ("gba", "Pokemon Pinball")]);
        assert_eq!(titles(&g, "POKEMON fire"), ["Pokémon - FireRed Version"]);
        assert_eq!(titles(&g, "pokemon"), ["Pokémon - FireRed Version", "Pokemon Pinball"]);
        assert_eq!(titles(&g, "spiderman"), ["Spider-Man"]);
        assert_eq!(titles(&g, "spider man"), ["Spider-Man"]);
        assert_eq!(titles(&g, "kirbys"), ["Kirby's Dream Land"]);
        // Accents written as separate marks (as macOS writes file names).
        assert_eq!(titles(&games(&[("gba", "Poke\u{301}mon")]), "pokémon"), ["Poke\u{301}mon"]);
    }

    #[test]
    fn same_titles_keep_the_system_order() {
        let g = games(&[("gb", "Tetris"), ("gbc", "Tetris"), ("gba", "Tetris Worlds")]);
        let found: Vec<&str> = search(&g, "tetris").iter().map(|g| g.system).collect();
        assert_eq!(found, ["gb", "gbc", "gba"]);
    }

    /// Not a test: how long a search of 3,000 games takes (run with --release -- --ignored
    /// --nocapture).
    #[test]
    #[ignore]
    fn time_a_search_of_a_long_list() {
        let g: Vec<Game> = (0..3000).map(|i| Game { system: "gba", path: format!("{i}").into(), title: format!("Pokémon Adventure - Chapter {i}"), sort_key: format!("{i}") }).collect();
        // The fastest of 100, which other work on the machine disturbs least.
        let fastest = (0..100).map(|_| {
            let t = std::time::Instant::now();
            std::hint::black_box(search(&g, "chapter 1"));
            t.elapsed()
        }).min().unwrap();
        println!("a search of 3,000 titles: {fastest:?}");
    }
}
