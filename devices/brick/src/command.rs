//! Running the system's programs, with every run and its result logged.

use std::process::Command;

/// Runs `program` with `args`, logging the command line, its exit status and what it printed,
/// prefixed by `area` (e.g. "transfer"). Ok: what it printed on stdout. Err: what it printed on
/// stderr (else stdout, else its exit status), or why it couldn't run.
pub fn run(area: &str, program: &str, args: &[&str]) -> Result<String, String> {
    let line = std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
    let out = match Command::new(program).args(args).output() {
        Ok(out) => out,
        Err(e) => { eprintln!("{area}: {line}: {e}"); return Err(format!("{program}: {e}")); }
    };
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    // Quoted, so line breaks stay on the log's one line.
    eprintln!("{area}: {line}: {}, stdout {stdout:?}, stderr {stderr:?}", out.status);
    if out.status.success() { return Ok(stdout); }
    Err([stderr, stdout, out.status.to_string()].into_iter().find(|s| !s.is_empty()).unwrap_or_default())
}
