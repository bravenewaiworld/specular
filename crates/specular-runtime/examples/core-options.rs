//! Writes the options each core declares to `<out-dir>/<core>.json` (`<core>` is its library's
//! name less `_libretro.so`), as `specular_core::settings::core_options::CoreOptions`. Run by
//! `make cores` on this PC's builds, into `data/cores/`, which Specular compiles in. A file is only
//! rewritten if what it says changed, so builds that depend on it stay up to date.
//! Usage: core-options <out-dir> <core.so>...

use std::path::Path;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, cores @ ..] = &args[..] else { return Err("usage: core-options <out-dir> <core.so>...".into()) };
    for core in cores {
        let name = Path::new(core).file_name().and_then(|n| n.to_str()).ok_or(format!("{core}: not a file name"))?;
        let id = name.strip_suffix(".so").unwrap_or(name);
        let id = id.strip_suffix("_libretro").unwrap_or(id);
        let options = specular_runtime::declared_options(Path::new(core)).map_err(|e| format!("{core}: {e}"))?;
        let text = serde_json::to_string_pretty(&options).map_err(|e| e.to_string())? + "\n";
        let path = Path::new(out).join(format!("{id}.json"));
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) { continue; }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("core-options: wrote {} ({} options)", path.display(), options.options.len());
    }
    Ok(())
}
