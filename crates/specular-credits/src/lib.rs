//! The credits of a Specular binary, worked out when it is built: every project it ships and the
//! licence it is used under. A device crate's build script calls `generate`, which writes to
//! `$OUT_DIR`:
//!
//! - `credits.rs`: a `&[specular_core::app::Credit]` expression, which the binary `include!`s and
//!   gives to the credits screen;
//! - `licences.txt`: the projects, then each distinct licence text once, headed by the projects it
//!   covers.
//!   It is also copied into the folder the binary is built in, where Specular looks for it (beside
//!   itself) and `make install` takes it from.
//!
//! The projects are the Rust crates linked into the binary (normal dependencies, for the platform
//! being built: build scripts and procedural macros only run during the build, so ship nothing),
//! and what `data/credits.tsv` lists (Specular itself, the cores, fonts, data, Slint). Every core
//! `tools/build-cores.sh` builds must be listed there.
//!
//! Problems in the repository's own files (a core not listed, a text missing, a bad line) fail the
//! build. A crate that ships no licence text only warns: updating a dependency shouldn't break a
//! device's build, and the warning names the crate and the fix.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// What a project is; the credits screen groups projects by it, in this order. The variants must
/// be named as `specular_core::app::CreditKind`'s are: `credits.rs` names them by their Debug form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind { Specular, Core, Font, Data, Library }

impl Kind {
    fn heading(self) -> &'static str {
        match self { Kind::Specular => "Specular", Kind::Core => "Emulator cores", Kind::Font => "Fonts", Kind::Data => "Data", Kind::Library => "Libraries" }
    }
}

/// One project shipped.
#[derive(Debug)]
struct Project {
    kind: Kind,
    name: String,
    /// A crate's version; a core's commit, abbreviated.
    version: Option<String>,
    /// The licence it is used under, as an SPDX expression.
    licence: String,
    /// What a crate itself declares, when `data/credits.tsv` chose one of the licences it offers.
    declared: Option<String>,
    url: Option<String>,
    /// For a core: where the source of the build we ship is.
    source: Option<String>,
    texts: Vec<PathBuf>,
}

impl Project {
    fn label(&self) -> String { self.version.as_ref().map_or(self.name.clone(), |v| format!("{} {v}", self.name)) }
}

/// `data/credits.tsv`, read (its header explains its lines).
#[derive(Debug, Default)]
struct Listed {
    projects: Vec<Project>,
    /// Each `core:<name>` line's name in `tools/build-cores.sh`, with its index in `projects`.
    cores: Vec<(String, usize)>,
    /// `crate` lines, by crate: the licence it is used under, and its texts.
    crates: HashMap<String, (Option<String>, Vec<PathBuf>)>,
}

/// Writes the credits of the package whose build script calls it (see the crate's docs). Fails
/// the build, with what to fix, if they can't be worked out.
pub fn generate() {
    match run() {
        Ok(warnings) => for w in warnings { println!("cargo::warning=credits: {w}"); },
        Err(e) => {
            eprintln!("error: credits: {e}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<Vec<String>, String> {
    let env = |k: &str| std::env::var(k).map_err(|_| format!("${k} isn't set (run this from a build script)"));
    let (cargo, package, target, out) = (env("CARGO")?, env("CARGO_PKG_NAME")?, env("TARGET")?, PathBuf::from(env("OUT_DIR")?));
    let manifest = format!("{}/Cargo.toml", env("CARGO_MANIFEST_DIR")?);
    let metadata: Value = serde_json::from_str(&cargo_output(&cargo, &["metadata", "--format-version", "1", "--offline", "--manifest-path", &manifest])?)
        .map_err(|e| format!("cargo metadata's output: {e}"))?;
    let root = PathBuf::from(metadata["workspace_root"].as_str().ok_or("cargo metadata gave no workspace_root")?);
    let read = |path: &str| std::fs::read_to_string(root.join(path)).map_err(|e| format!("can't read {path}: {e}"));

    let mut listed = parse_listed(&read("data/credits.tsv")?, &root)?;
    add_cores(&mut listed, &cores_built(&read("tools/build-cores.sh")?))?;
    let linked = linked_crates(&cargo, &manifest, &package, &target)?;
    let mut warnings = Vec::new();
    let mut projects = std::mem::take(&mut listed.projects);
    projects.sort_by_key(|p| p.kind); // stable: the file's order within a kind
    projects.extend(crates(&metadata, &linked, &listed, &mut warnings)?);

    std::fs::write(out.join("credits.rs"), credits_rs(&projects)).map_err(|e| format!("can't write credits.rs: {e}"))?;
    let licences = licences_txt(&projects)?;
    std::fs::write(out.join("licences.txt"), &licences).map_err(|e| format!("can't write licences.txt: {e}"))?;
    match binary_dir(&out) {
        // Through a file of its own, then renamed: another device's build script may be writing
        // the same file (they share the folder when built for the same platform; the last one
        // built wins there, as it does for the `specular` binary itself).
        Some(dir) => {
            let temp = dir.join(format!("licences.txt.{package}"));
            std::fs::write(&temp, &licences).and_then(|()| std::fs::rename(&temp, dir.join("licences.txt")))
                .map_err(|e| format!("can't write licences.txt in {}: {e}", dir.display()))?;
        }
        None => warnings.push(format!("can't tell which folder the binary is built in from $OUT_DIR ({}), so licences.txt is only there", out.display())),
    }

    // Registry crates' files never change. The repository's may, and the manifests and lock file
    // decide what is linked.
    let mut watched: BTreeSet<PathBuf> = ["data/credits.tsv", "tools/build-cores.sh", "Cargo.lock", "Cargo.toml"].iter().map(|p| root.join(p)).collect();
    let members: HashSet<&str> = metadata["workspace_members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let packages = metadata["packages"].as_array().into_iter().flatten();
    watched.extend(packages.filter(|p| p["id"].as_str().is_some_and(|id| members.contains(id))).filter_map(|p| p["manifest_path"].as_str()).map(PathBuf::from));
    watched.extend(projects.iter().flat_map(|p| &p.texts).filter(|t| t.starts_with(&root)).cloned());
    for path in watched { println!("cargo::rerun-if-changed={}", path.display()); }
    Ok(warnings)
}

/// Runs cargo with `args`; its output.
fn cargo_output(cargo: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cargo).args(args).output().map_err(|e| format!("can't run {cargo}: {e}"))?;
    if !out.status.success() { return Err(format!("`cargo {}` failed:\n{}", args.join(" "), String::from_utf8_lossy(&out.stderr))); }
    String::from_utf8(out.stdout).map_err(|e| format!("`cargo {}` wrote something other than UTF-8: {e}", args.join(" ")))
}

/// The crates linked into `package` when built for `target`, as (name, version). `cargo tree`,
/// unlike `cargo metadata`, resolves features as building just `package` does, so a dependency
/// only another device enables isn't counted.
fn linked_crates(cargo: &str, manifest: &str, package: &str, target: &str) -> Result<HashSet<(String, String)>, String> {
    let tree = cargo_output(cargo, &["tree", "--offline", "--manifest-path", manifest, "-p", package, "--target", target,
                                     "-e", "normal,no-proc-macro", "--prefix", "none"])?;
    // Lines are "name vX.Y.Z", maybe followed by its source and "(*)" for one shown before.
    Ok(tree.lines().filter_map(|l| {
        let mut words = l.split_whitespace();
        Some((words.next()?.to_string(), words.next()?.strip_prefix('v')?.to_string()))
    }).collect())
}

/// Reads `data/credits.tsv`; text paths are relative to `root`.
fn parse_listed(tsv: &str, root: &Path) -> Result<Listed, String> {
    let mut listed = Listed::default();
    for (n, line) in tsv.lines().enumerate().filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#')) {
        let given = |s: &str| (s != "-").then(|| s.to_string());
        let texts = |s: &str| s.split(' ').filter(|p| !p.is_empty() && *p != "-").map(|p| root.join(p)).collect::<Vec<_>>();
        match line.split('\t').collect::<Vec<_>>()[..] {
            ["crate", name, licence, files] => { listed.crates.insert(name.into(), (given(licence), texts(files))); }
            [kind, name, licence, url, files] => {
                let kind = match kind {
                    "specular" => Kind::Specular, "font" => Kind::Font, "data" => Kind::Data, "library" => Kind::Library,
                    _ => match kind.strip_prefix("core:") {
                        Some(core) => { listed.cores.push((core.into(), listed.projects.len())); Kind::Core }
                        None => return Err(format!("data/credits.tsv:{}: unknown kind {kind:?}", n + 1)),
                    },
                };
                listed.projects.push(Project { kind, name: name.into(), version: None, licence: licence.into(), declared: None, url: given(url), source: None, texts: texts(files) });
            }
            _ => return Err(format!("data/credits.tsv:{}: expected 5 columns separated by tabs (4 on a crate line)", n + 1)),
        }
    }
    Ok(listed)
}

/// The cores `tools/build-cores.sh` builds, as (name, URL, commit): its `patched_src` calls, each
/// on a line of its own with nothing else on it.
fn cores_built(script: &str) -> Vec<(String, String, String)> {
    script.lines().filter_map(|l| match l.split_whitespace().collect::<Vec<_>>()[..] {
        ["patched_src", core, url, commit] => Some((core.into(), url.into(), commit.into())),
        _ => None,
    }).collect()
}

/// Gives each listed core its version and source from `tools/build-cores.sh`; every core built
/// must be listed, and every core listed built.
fn add_cores(listed: &mut Listed, built: &[(String, String, String)]) -> Result<(), String> {
    for (core, url, commit) in built {
        let Some(&(_, i)) = listed.cores.iter().find(|(c, _)| c == core) else {
            return Err(format!("tools/build-cores.sh builds the core {core}, but data/credits.tsv has no core:{core} line: add one, with its licence"));
        };
        let p = &mut listed.projects[i];
        p.version = Some(commit.chars().take(12).collect());
        p.url = Some(url.trim_end_matches(".git").into());
        p.source = Some(format!("{url} at commit {commit}, with Specular's patches (tools/core-patches/{core}/ in Specular's source)"));
    }
    match listed.cores.iter().find(|(c, _)| !built.iter().any(|(b, ..)| b == c)) {
        Some((core, _)) => Err(format!("data/credits.tsv lists core:{core}, which tools/build-cores.sh doesn't build: remove the line")),
        None => Ok(()),
    }
}

/// The crates in `linked`, except Specular's own, sorted by name.
fn crates(metadata: &Value, linked: &HashSet<(String, String)>, listed: &Listed, warnings: &mut Vec<String>) -> Result<Vec<Project>, String> {
    let members: HashSet<&str> = metadata["workspace_members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut projects = Vec::new();
    for p in metadata["packages"].as_array().into_iter().flatten() {
        let s = |k: &str| p[k].as_str();
        let (Some(name), Some(version)) = (s("name"), s("version")) else { continue };
        if !linked.contains(&(name.to_string(), version.to_string())) || s("id").is_some_and(|id| members.contains(id)) { continue; }
        let dir = Path::new(s("manifest_path").unwrap_or_default()).parent().unwrap_or(Path::new("."));
        let mut texts = licence_files(dir, s("license_file"));
        let (chosen, given) = listed.crates.get(name).cloned().unwrap_or_default();
        if !given.is_empty() {
            if texts.is_empty() { texts = given; } else { warnings.push(format!("data/credits.tsv gives licence texts for the crate {name}, whose package now has its own: remove them from its line")); }
        }
        // A crate with only a licence file has terms of its own, which no SPDX expression names.
        let declared = s("license").map(str::to_string).or_else(|| s("license_file").map(|_| "custom".into()));
        let Some(licence) = chosen.clone().or(declared.clone()) else {
            return Err(format!("the crate {name} {version} declares no licence: check its terms, then give it a crate line in data/credits.tsv (or stop using it)"));
        };
        if texts.is_empty() {
            warnings.push(format!("the crate {name} {version} ({licence}) ships no licence text: put its text in data/licences/ and name it on a crate line in data/credits.tsv"));
        }
        let url = s("repository").or(s("homepage")).map(str::to_string);
        projects.push(Project { kind: Kind::Library, name: name.into(), version: Some(version.into()), licence, declared: chosen.and(declared), url, source: None, texts });
    }
    projects.sort_by(|a, b| (a.name.to_lowercase(), &a.version).cmp(&(b.name.to_lowercase(), &b.version)));
    Ok(projects)
}

/// A crate's licence texts: the file its manifest names, and the files at the top of its package
/// whose names start like a licence's (and those in such a folder, as REUSE's `LICENSES/`).
fn licence_files(dir: &Path, declared: Option<&str>) -> Vec<PathBuf> {
    const NAMES: [&str; 6] = ["LICENSE", "LICENCE", "COPYING", "UNLICENSE", "COPYRIGHT", "NOTICE"];
    let files_in = |d: &Path| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).collect::<Vec<_>>();
    let mut files: BTreeSet<PathBuf> = declared.map(|f| dir.join(f)).filter(|f| f.is_file()).into_iter().collect();
    for path in files_in(dir) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_uppercase();
        if !NAMES.iter().any(|n| name.starts_with(n)) { continue; }
        if path.is_dir() { files.extend(files_in(&path).into_iter().filter(|p| p.is_file())); } else { files.insert(path); }
    }
    files.into_iter().collect()
}

/// The folder the binary is built in: `$OUT_DIR` is `<that folder>/build/<package>-<hash>/out`.
fn binary_dir(out: &Path) -> Option<&Path> {
    let build = out.parent()?.parent()?;
    if build.file_name()? == "build" { build.parent() } else { None }
}

fn credits_rs(projects: &[Project]) -> String {
    let mut rs = String::from("&[\n");
    for p in projects {
        // Debug-formatted strings and options are Rust literals.
        let _ = writeln!(rs, "    specular_core::app::Credit {{ kind: specular_core::app::CreditKind::{:?}, name: {:?}, version: {:?}, licence: {:?}, url: {:?} }},",
                         p.kind, p.name, p.version, p.licence, p.url);
    }
    rs + "]\n"
}

/// A licence text as compared and written: line endings and trailing spaces evened out, so copies
/// that differ only in those count as one.
fn normalised(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
}

fn licences_txt(projects: &[Project]) -> Result<String, String> {
    const RULE: &str = "================================================================================";
    // Each distinct text, with the projects it covers, in the order first seen.
    let mut texts: Vec<(String, Vec<usize>)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, p) in projects.iter().enumerate() {
        for path in &p.texts {
            let bytes = std::fs::read(path).map_err(|e| format!("can't read {}, {}'s licence text: {e}", path.display(), p.name))?;
            let text = normalised(&String::from_utf8_lossy(&bytes));
            let k = *seen.entry(text.clone()).or_insert_with(|| { texts.push((text, Vec::new())); texts.len() - 1 });
            if !texts[k].1.contains(&i) { texts[k].1.push(i); }
        }
    }
    let mut out = String::from("Specular and the projects it ships, with their licences. The licence texts follow the list,\neach after the projects it covers.\n");
    let mut kind = None;
    for p in projects {
        if kind != Some(p.kind) { kind = Some(p.kind); let _ = write!(out, "\n{}\n", p.kind.heading()); }
        let _ = write!(out, "  {}: {}", p.label(), p.licence);
        if let Some(d) = &p.declared { let _ = write!(out, " (chosen from {d})"); }
        if let Some(u) = &p.url { let _ = write!(out, ", {u}"); }
        out.push('\n');
        if let Some(s) = &p.source { let _ = writeln!(out, "    Source: {s}"); }
    }
    let missing: Vec<String> = projects.iter().filter(|p| p.texts.is_empty()).map(Project::label).collect();
    if !missing.is_empty() { let _ = write!(out, "\nNo licence text was found for: {}\n", missing.join(", ")); }
    for (text, users) in texts {
        let names: Vec<String> = users.iter().map(|&i| projects[i].label()).collect();
        let _ = write!(out, "\n\n{RULE}\n{}\n{RULE}\n\n{text}\n", names.join(", "));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("../..") }

    #[test]
    fn the_repositorys_list_covers_every_core_built_and_its_texts_exist() {
        let read = |p: &str| std::fs::read_to_string(root().join(p)).unwrap();
        let mut listed = parse_listed(&read("data/credits.tsv"), &root()).unwrap();
        let built = cores_built(&read("tools/build-cores.sh"));
        assert!(!built.is_empty());
        add_cores(&mut listed, &built).unwrap();
        for path in listed.projects.iter().flat_map(|p| &p.texts).chain(listed.crates.values().flat_map(|(_, t)| t)) {
            assert!(path.is_file(), "{} is missing", path.display());
        }
    }

    #[test]
    fn cores_must_be_listed_and_built() {
        let script = "# patched_src CORE URL COMMIT: …\npatched_src() {\n\td=$OUT/src/$1\n}\npatched_src mgba https://example.com/mgba.git 0123456789abcdef\n";
        let built = cores_built(script);
        assert_eq!(built, [("mgba".into(), "https://example.com/mgba.git".into(), "0123456789abcdef".into())]);
        let tsv = |lines: &str| parse_listed(lines, Path::new("/")).unwrap();
        let mut listed = tsv("core:mgba\tmGBA\tMPL-2.0\t-\tdata/licences/MPL-2.0.txt\n");
        add_cores(&mut listed, &built).unwrap();
        let p = &listed.projects[0];
        assert_eq!((p.version.as_deref(), p.url.as_deref()), (Some("0123456789ab"), Some("https://example.com/mgba")));
        assert!(add_cores(&mut tsv(""), &built).unwrap_err().contains("no core:mgba line"));
        assert!(add_cores(&mut tsv("core:nes\tNES\tGPL-2.0\t-\t-\n"), &[]).unwrap_err().contains("doesn't build"));
    }

    #[test]
    fn bad_lines_are_named() {
        assert!(parse_listed("# a comment\n\nfont\tGeist\n", Path::new("/")).unwrap_err().starts_with("data/credits.tsv:3:"));
        assert!(parse_listed("fonts\tGeist\tOFL-1.1\t-\t-\n", Path::new("/")).unwrap_err().contains("unknown kind"));
    }

    #[test]
    fn texts_differing_only_in_line_endings_and_spacing_are_one() {
        assert_eq!(normalised("\nMIT License  \r\n\r\nCopyright\r\n\n"), normalised("MIT License\n\nCopyright"));
    }
}
