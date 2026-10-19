//! File transfer (Settings → File transfer): while its screen is open, the device runs a server
//! that PCs reach with a browser, reading and writing the SD card with no password. Leaving the
//! screen stops it. So does sleeping, and it starts again on waking if the screen is still open;
//! the device doesn't sleep by itself while the screen is open, as a transfer may be running.
//! WiFi on Auto counts the open screen as a need (see app::wireless); a PC connected by a USB
//! cable reaches the server over the cable's network link, without WiFi.

use super::*;
use crate::wireless::wifi::WifiLink;

/// What the app knows of file transfer.
pub struct TransferState {
    /// As last reported by the device.
    pub server: Server,
    /// Whether the server was last told to run (`Effect::FileTransfer`).
    told: bool,
}

/// The file transfer server, as last reported.
#[derive(Clone, Debug, PartialEq)]
pub enum Server {
    Stopped,
    Starting,
    Running,
    /// It didn't start, and why.
    Failed(String),
}

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::FILE_TRANSFER, Box::new(TransferScreen));
}

impl App {
    /// Says whether the device has file transfer (none until told). Call before the first
    /// message.
    pub fn has_file_transfer(&mut self, yes: bool) {
        self.ctx.transfer = yes.then_some(TransferState { server: Server::Stopped, told: false });
    }

    /// Whether the file transfer screen is open (it may be under another, such as Wi-Fi's).
    pub(super) fn transfer_open(&self) -> bool { self.nav.contains(&screens::FILE_TRANSFER) }

    /// Starts or stops the server to match what is wanted now: running while its screen is open,
    /// the SD card is in and the device is awake. Called after every message, and before sleeping.
    pub(super) fn reconcile_transfer(&mut self) -> Vec<Effect> {
        let wanted = self.transfer_open() && self.ctx.card_found && !self.asleep;
        let Some(t) = &mut self.ctx.transfer else { return vec![] };
        if t.told == wanted { return vec![]; }
        t.told = wanted;
        if wanted { t.server = Server::Starting; }
        self.changed();
        vec![Effect::FileTransfer(wanted), Effect::Redraw(None)]
    }

    /// A start (`start` true) or stop has finished. One overtaken by the opposite command changes
    /// nothing, except that a failed stop is always shown as a notice: the card may still be open
    /// to the network.
    pub(super) fn transfer_done(&mut self, start: bool, result: Result<(), String>) -> Vec<Effect> {
        let Some(t) = &mut self.ctx.transfer else { return vec![] };
        match result {
            Err(why) if !start => return vec![Effect::Notice(format!("File transfer may still be on: {why}"))],
            _ if start != t.told => return vec![],
            Ok(()) => t.server = if start { Server::Running } else { Server::Stopped },
            Err(why) => t.server = Server::Failed(why),
        }
        self.changed();
        vec![Effect::Redraw(None)]
    }
}

/// Settings' row for file transfer, if the device has it.
pub(super) fn settings_row(ctx: &Ctx) -> Option<Row> {
    ctx.transfer.as_ref().map(|_| action("open:file transfer", "File transfer"))
}

/// "http://10.0.0.2/".
fn url(address: &str) -> String { format!("http://{address}/") }

/// The file transfer screen: a warning, then the addresses to open in a browser (WiFi's, and the
/// device's other connected network interfaces, as the WiFi service reports them), or the server's
/// state. A on Wi-Fi's row opens the Wi-Fi screen, with the server still running.
struct TransferScreen;

impl Screen for TransferScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        if !ctx.card_found {
            let rows = vec![Row::note("note:none", "No SD card"), Row::note("note:where", "Insert one to transfer files to it.")];
            return ScreenView::list("File transfer", rows);
        }
        let mut rows = vec![Row::note("note:warning", "While this screen is open, anyone on the same network can read and change everything on the SD card. Leave it to stop.")];
        match ctx.transfer.as_ref().map(|t| &t.server) {
            Some(Server::Running) => {
                rows.push(Row::new("header:open", "Open in a browser on a PC", RowKind::Header));
                if let Some(w) = &ctx.wifi {
                    let value = match (&w.link, wireless::wifi_mode(ctx)) {
                        (WifiLink::Connected { address, .. }, _) => url(address),
                        (_, "off") => "Off".into(),
                        (WifiLink::Connecting { .. }, _) => "Connecting…".into(),
                        (WifiLink::Off, _) => "Starting…".into(),
                        _ => "Not connected".into(),
                    };
                    rows.push(Row::new("open:wifi", "Wi-Fi", RowKind::Action { value: Some(value) }));
                }
                // The other connected network interfaces, such as a USB cable's to a PC.
                for i in ctx.wifi.iter().flat_map(|w| &w.interfaces) {
                    let label = if i.name.starts_with("usb") { "USB cable" } else { &i.name };
                    rows.push(Row::new(format!("address:{}:{}", i.name, i.address), label, RowKind::Note { detail: Some(url(&i.address)) }));
                }
            }
            Some(Server::Failed(why)) => {
                rows.push(Row::note("note:failed", format!("File transfer didn't start: {why}")));
                rows.push(action("retry", "Try again"));
            }
            _ => rows.push(Row::note("note:starting", "Starting…")),
        }
        let mut view = ScreenView::list("File transfer", rows);
        view.hints.push((Button::A, "Select".into()));
        view
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { return Outcome::Back; }
        if let Some(open) = focused.and_then(|row| wireless::settings_press(row, p)) { return open; }
        if p.button == Button::A && !p.repeat && focused.is_some_and(|r| r.id.0 == "retry") && let Some(t) = &mut ctx.transfer {
            t.told = false; // so the next reconcile starts it again
            return Outcome::Changed;
        }
        Outcome::None
    }
}
