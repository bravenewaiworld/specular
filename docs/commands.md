# Commands

Status: In progress · Updated: 2026-10-27

Controlling a running Specular from outside it, for development, tests and scripts: start a game, quit it, press buttons, ask what is showing. Things that are awkward or impossible over a plain SSH shell, since the UI and the running game live inside Specular's process.

## Decisions

- **A Unix socket, one line per command and one line back** (2026-10-27). `ok [text]` when done, `error <why>` when not; `status` answers with JSON on its line. Plain lines are easy to send from anything and to read in a log. Rejected: a TCP port (it would need its own authentication; SSH already gives that); a FIFO (no answers); signals (no arguments, no answers).
- **Only the user Specular runs as may send commands** (2026-10-27). The socket is made `0600`; on a device that is root, so a command means an SSH login first. Rejected: a dev-mode switch (the socket reaches nothing the SSH login doesn't already).
- **The Specular binary is its own client: `specular cmd <command>`** (2026-10-27). A device has nothing else to rely on (stock BusyBox builds often lack `nc -U` and `socat`). Its exit status says whether the command was done (0), refused (1), or Specular couldn't be reached (2).
- **Commands go through the app like any other input** (2026-10-27). `App::command` takes one and returns its answer and effects, so a command does what the same action does from the screen, under the same rules (no game while asleep or while the installer works), and is tested without a socket. A button press is the exception: the UI carries it out, as it has to wait for the release, and it goes in exactly as the device's own buttons do (to the game while one plays).
- **`play` starts a game over whatever screen is showing** (2026-10-27). Quitting it comes back to that screen. Rejected: going to the game's list first (a script asking for a game wants the game, not the list).

## Design

```rust
// specular_core::command
pub enum Command { Play(String), Quit, Status, Press(Button, Duration), Help }
impl FromStr for Command { type Err = String; }   // Err: why the line isn't a command

// specular_core::app
impl App {
    /// The answer (Ok text, or Err why not) and the effects to carry out.
    pub fn command(&mut self, cmd: &Command) -> (Result<String, String>, Vec<Effect>);
}
```

| Command | Does |
|---|---|
| `play <path under roms/ \| title>` | Starts the game; a title must name only one game (in any case). Refused while a game plays |
| `quit` | Quits the running game, writing its saves, and closes its menu if open |
| `status` | `{"screen": "games", "params": "gba", "playing": "gba/Golden Sun.gba", "asleep": false}` |
| `press <button> [hold ms]` | Presses a button (by `Button::NAMES`: `a`, `down`, `menu`, `power`, …), released after the hold (default 50 ms, at most 10 s); answered after the release |
| `help` | Lists the commands |

The socket is `$SPECULAR_SOCKET`, else `specular.sock` in `$XDG_RUNTIME_DIR`, else `/tmp/specular.sock`; client and server find it by the same rule. At start-up a socket left by an earlier run is replaced; if another Specular answers on it, this one doesn't listen. A thread accepts connections and serves each on a thread of its own; a command is handed to the UI thread, and its answer awaited for 5 s (plus a press's hold). Each command and its answer is logged (`commands: "play mother 3": ok starting Mother 3`).

Adding a command: a `Command` variant and its line in `FromStr` and `HELP`, then its arm in `App::command` (or, if it has to wait on time or the UI, in `Runner::command`).

## Open questions

- Which other commands are worth having: a screenshot (the window's snapshot, as `SPECULAR_SCRIPT` takes), saving and loading the state, sleep and wake, going home.
