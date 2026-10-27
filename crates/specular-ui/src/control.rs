//! The command socket: a Unix socket the running Specular takes commands on, one line each,
//! answering each with one line (`ok [text]` or `error <why>`); and the client that sends one
//! (`specular cmd …`). See docs/commands.md.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use specular_core::command::{Command, MAX_HOLD};

/// Where a command's answer goes: Ok (with text, or none) or Err (why it failed).
pub type Reply = mpsc::Sender<Result<String, String>>;

/// How long a command may take to be answered, beyond a press's hold. The UI thread answers at
/// once unless it is busy; much longer means it is stuck.
const ANSWER_TIME: Duration = Duration::from_secs(5);

/// The socket's path: `$SPECULAR_SOCKET`, else `specular.sock` in `$XDG_RUNTIME_DIR`, else
/// /tmp/specular.sock. The client finds the server by the same rule, so both need the same
/// environment (on a device, neither variable is usually set).
pub fn socket_path() -> PathBuf {
    std::env::var_os("SPECULAR_SOCKET").map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("specular.sock")))
        .unwrap_or_else(|| "/tmp/specular.sock".into())
}

/// Listens on the socket on a thread of its own, with a thread for each connection. Each command
/// is handed to `carry_out` with where to send its answer. Nothing listens if another Specular
/// already is, or the socket can't be made (logged).
pub fn listen(carry_out: impl Fn(Command, Reply) + Send + Sync + Copy + 'static) {
    let path = socket_path();
    if UnixStream::connect(&path).is_ok() {
        eprintln!("commands: another Specular is taking commands at {}; this one won't", path.display());
        return;
    }
    // A socket left by an earlier run that ended without removing it.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => { eprintln!("commands: can't listen at {}: {e}", path.display()); return; }
    };
    // Only the user Specular runs as (root, on a device) may send commands.
    if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        eprintln!("commands: can't make {} private, so not listening: {e}", path.display());
        return;
    }
    eprintln!("commands: listening at {}", path.display());
    std::thread::spawn(move || for conn in listener.incoming() {
        match conn {
            Ok(conn) => { std::thread::spawn(move || serve(conn, carry_out)); }
            Err(e) => eprintln!("commands: accepting a connection: {e}"),
        }
    });
}

/// Answers each line `conn` sends, until it closes.
fn serve(conn: UnixStream, carry_out: impl Fn(Command, Reply)) {
    let Ok(mut out) = conn.try_clone() else { return };
    for line in BufReader::new(conn).lines() {
        let Ok(line) = line else { return };
        let answer = match line.parse::<Command>() {
            Ok(cmd) => {
                let wait = ANSWER_TIME + if let Command::Press(_, hold) = cmd { hold } else { Duration::ZERO };
                let (tx, rx) = mpsc::channel();
                carry_out(cmd, tx);
                rx.recv_timeout(wait).unwrap_or_else(|e| Err(match e {
                    mpsc::RecvTimeoutError::Timeout => format!("no answer within {} s: Specular may be stuck", wait.as_secs()),
                    mpsc::RecvTimeoutError::Disconnected => "Specular is quitting".into(),
                }))
            }
            Err(why) => Err(why),
        };
        let answer = match answer {
            Ok(text) if text.is_empty() => "ok".to_string(),
            Ok(text) => format!("ok {text}"),
            Err(why) => format!("error {why}"),
        };
        eprintln!("commands: {line:?}: {answer}");
        if writeln!(out, "{answer}").is_err() { return; }
    }
}

/// `specular cmd <words>`: sends the words as one command to the running Specular and prints its
/// answer: on stdout if it was done (exit status 0), else on stderr (1). 2: Specular couldn't be
/// reached, or didn't answer.
pub fn send(words: &[String]) -> i32 {
    let path = socket_path();
    let answer = UnixStream::connect(&path).and_then(|mut conn| {
        // A press is answered when its button is released.
        conn.set_read_timeout(Some(ANSWER_TIME * 2 + MAX_HOLD))?;
        writeln!(conn, "{}", words.join(" "))?;
        let mut answer = String::new();
        BufReader::new(conn).read_line(&mut answer)?;
        Ok(answer)
    });
    match answer.as_deref().map(str::trim_end) {
        Ok("") => { eprintln!("Specular closed the connection without answering"); 2 }
        Ok(a) if a == "ok" || a.starts_with("ok ") => { if a.len() > 3 { println!("{}", &a[3..]); } 0 }
        Ok(a) => { eprintln!("{}", a.strip_prefix("error ").unwrap_or(a)); 1 }
        Err(e) => { eprintln!("can't reach Specular at {}: {e}", path.display()); 2 }
    }
}
