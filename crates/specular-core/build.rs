// Writes `core_options.rs` (included by src/settings/core_options.rs) from the cores' option lists
// in data/cores/<core>.json: for each core a module of that name, with its list (`LIST`) and a
// constant for each option's key and each of its values, so Specular's code can name them. A
// constant that would clash with another is left out, with a warning.
use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;

use serde_json::Value;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/cores");
    println!("cargo::rerun-if-changed={}", dir.display());
    let mut files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "json"));
    files.sort();
    let mut out = String::new();
    for path in files {
        let core = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let list: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}; `make cores` writes it again", path.display()));
        module(&mut out, &core, &path.canonicalize().unwrap(), &list);
    }
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("core_options.rs"), out).unwrap();
}

fn module(out: &mut String, core: &str, path: &Path, list: &Value) {
    let w = &mut *out;
    writeln!(w, "/// The options of the core `{core}` (`data/cores/{core}.json`), and their values.").unwrap();
    writeln!(w, "pub mod {} {{", module_name(core)).unwrap();
    writeln!(w, "    /// Its option list, as `make cores` wrote it (`CoreDecl::options`).").unwrap();
    writeln!(w, "    pub const LIST: &str = include_str!({:?});", path.display().to_string()).unwrap();
    let mut names = HashSet::from(["LIST".to_string()]);
    for o in list["options"].as_array().unwrap_or_else(|| panic!("data/cores/{core}.json has no \"options\"")) {
        let (key, label) = (o["key"].as_str().unwrap(), o["label"].as_str().unwrap_or_default());
        // Without the core's prefix: mgba::IDLE_OPTIMIZATION rather than mgba::MGBA_IDLE_OPTIMIZATION.
        let short = key.get(core.len() + 1..).filter(|_| key.to_lowercase().starts_with(&format!("{core}_"))).unwrap_or(key);
        let name = constant(short);
        if !names.insert(name.clone()) { println!("cargo::warning=data/cores/{core}.json: no constant for {key}: {core}::{name} is taken"); continue; }
        writeln!(w, "    /// {}", doc(label)).unwrap();
        writeln!(w, "    pub const {name}: &str = {key:?};").unwrap();
        writeln!(w, "    /// The values of `{name}`.").unwrap();
        writeln!(w, "    pub mod {} {{", module_name(short)).unwrap();
        let mut values = HashSet::new();
        for v in o["values"].as_array().into_iter().flatten() {
            let value = v["value"].as_str().unwrap();
            let name = constant(value);
            if !values.insert(name.clone()) { println!("cargo::warning=data/cores/{core}.json: no constant for {key}'s value {value:?}: {name} is taken"); continue; }
            writeln!(w, "        /// {}", doc(v["label"].as_str().unwrap_or(value))).unwrap();
            writeln!(w, "        pub const {name}: &str = {value:?};").unwrap();
        }
        writeln!(w, "    }}").unwrap();
    }
    writeln!(w, "}}").unwrap();
}

/// An upper-case Rust name for `s`: letters and digits, the rest turned into single underscores
/// (apostrophes dropped: "Don't Remove" is DONT_REMOVE), with a V before a leading digit. A sign
/// in front is spelt out ("-1" is MINUS_1).
fn constant(s: &str) -> String {
    let s = s.strip_prefix('-').map(|n| format!("minus {n}")).or_else(|| s.strip_prefix('+').map(|n| format!("plus {n}"))).unwrap_or(s.into());
    let mut name = String::new();
    for c in s.chars().filter(|&c| c != '\'') {
        if c.is_ascii_alphanumeric() { name.push(c.to_ascii_uppercase()); } else if !name.is_empty() && !name.ends_with('_') { name.push('_'); }
    }
    let name = name.trim_end_matches('_');
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) { format!("V{name}") } else { name.to_string() }
}

/// A lower-case module name for `s`, with an underscore after one that is a Rust keyword.
fn module_name(s: &str) -> String {
    const KEYWORDS: &[&str] = &["as", "async", "await", "box", "break", "const", "continue", "crate", "do", "dyn", "else", "enum", "extern",
        "false", "final", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv",
        "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
        "virtual", "where", "while", "yield"];
    let name = constant(s).to_lowercase();
    if KEYWORDS.contains(&name.as_str()) { name + "_" } else { name }
}

/// Text for a one-line doc comment.
fn doc(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") }
