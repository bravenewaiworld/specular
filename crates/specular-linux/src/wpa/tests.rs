use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::{Mutex, mpsc};

use super::*;

/// SCAN_RESULTS as wpa_supplicant 2.11 gave it on the Brick (a pair of access points named
/// U+1F35D, one of them WPA3-only, hidden networks, and one named with 30 zero bytes).
const SCAN: &str = "bssid / frequency / signal level / flags / ssid
7e:7f:f0:76:08:8c\t2432\t-25\t[WPA2--CCMP][SAE-H2E][ESS]\t\\xf0\\x9f\\x8d\\x9d MLO
7e:7f:f0:76:08:8e\t2432\t-25\t[WPA2-PSK-CCMP][ESS]\t\\xf0\\x9f\\x8d\\x9d
16:98:8f:bf:49:5e\t2462\t-67\t[WPA2-PSK-CCMP][ESS]\tVRP
d8:07:b6:6e:f7:0a\t2422\t-74\t[WPA2-PSK+FT/PSK-CCMP][ESS]\tCingular
e2:07:b6:6e:f7:0a\t2422\t-74\t[WPA2-PSK-CCMP][ESS]\t
b8:fb:b3:3d:4a:d2\t2452\t-72\t[WPA2-PSK-CCMP][ESS]\t\\xf0\\x9f\\x8d\\x9d
82:eb:b6:0c:41:91\t2447\t-74\t[WPA2-PSK-CCMP][ESS]\t\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00
3c:28:6d:5f:1c:9d\t2412\t-79\t[WPA2-PSK-CCMP][ESS]\tLanzillotta
fa:8f:ca:8f:d8:8c\t2452\t-75\t[ESS]\t
00:11:22:33:44:55\t2412\t-80\t[ESS]\tCafe \\\"Free\\\"
00:11:22:33:44:56\t24";

const PASTA: &str = "\u{1f35d}";

#[test]
fn scan_results_one_per_name_strongest_first_without_hidden_ones() {
    let seen = parse_scan_results(SCAN);
    let names: Vec<(&str, i32, bool, bool)> = seen.iter().map(|s| (s.ssid.as_str(), s.dbm, s.secured, s.joinable)).collect();
    assert_eq!(names, [
        (&*format!("{PASTA} MLO"), -25, true, false),
        (PASTA, -25, true, true),
        ("VRP", -67, true, true),
        ("Cingular", -74, true, true),
        ("Lanzillotta", -79, true, true),
        ("Cafe \"Free\"", -80, false, true),
    ]);
}

#[test]
fn saved_networks_join_those_in_range_and_follow_them() {
    let list = "network id / ssid / bssid / flags\n0\t\\xf0\\x9f\\x8d\\x9d\tany\t[TEMP-DISABLED]\n3\tOffice\tany\t[DISABLED]\n";
    assert_eq!(parse_saved(list), [(0, PASTA.to_string()), (3, "Office".to_string())]);
    let saved = [Saved { id: 0, ssid: PASTA.into(), open: false }, Saved { id: 3, ssid: "Office".into(), open: true }];
    let n = networks(&parse_scan_results(SCAN), &saved);
    assert_eq!(n[1], Network { ssid: PASTA.into(), bars: 4, secured: true, saved: true, in_range: true, supported: true });
    assert_eq!(n[2], Network { ssid: "VRP".into(), bars: 2, secured: true, saved: false, in_range: true, supported: true });
    assert_eq!(n.last(), Some(&Network { ssid: "Office".into(), bars: 0, secured: false, saved: true, in_range: false, supported: true }));
    assert_eq!(n.len(), 7);
}

#[test]
fn escapes() {
    assert_eq!(unescape(r#"a\\b\"c\e\n\r\t\x41\xzz\"#), b"a\\b\"c\x1b\n\r\tA\\xzz\\");
    assert_eq!(decode_ssid(r"\xf0\x9f\x8d\x9d"), PASTA);
}

#[test]
fn messages_as_recorded() {
    let m = |s| parse_message(s);
    assert_eq!(m("<3>CTRL-EVENT-SCAN-STARTED "), Message::ScanStarted);
    assert_eq!(m("<3>CTRL-EVENT-SCAN-RESULTS "), Message::ScanResults);
    assert_eq!(m("<3>CTRL-EVENT-NETWORK-NOT-FOUND "), Message::NotFound);
    assert_eq!(m("<3>SME: Trying to authenticate with 7e:7f:f0:76:08:8e (SSID='\\xf0\\x9f\\x8d\\x9d' freq=2432 MHz)"), Message::Trying { ssid: Some(PASTA.into()) });
    assert_eq!(m("<3>Trying to associate with 7e:7f:f0:76:08:8e (SSID='it's' freq=2432 MHz)"), Message::Trying { ssid: Some("it's".into()) });
    assert_eq!(m("<3>Associated with 7e:7f:f0:76:08:8e"), Message::Associated);
    assert_eq!(m("<3>CTRL-EVENT-CONNECTED - Connection to 7e:7f:f0:76:08:8e completed [id=0 id_str=]"), Message::Connected);
    assert_eq!(m("<3>CTRL-EVENT-DISCONNECTED bssid=7e:7f:f0:76:08:8e reason=15"), Message::Disconnected);
    assert_eq!(m("<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid=\"\\xf0\\x9f\\x8d\\x9d\" auth_failures=1 duration=10 reason=WRONG_KEY"),
               Message::TempDisabled { id: 0, reason: "WRONG_KEY".into() });
    assert_eq!(m("<3>WPA: 4-Way Handshake failed - pre-shared key may be incorrect"), Message::Other);
    assert_eq!(m("<2>CTRL-EVENT-TERMINATING "), Message::Terminating);
}

#[test]
fn link_from_status() {
    let link = |s: &str| status_link(s);
    assert_eq!(link("wpa_state=SCANNING\naddress=16:e5:71:99:1e:50\n"), WifiLink::Idle);
    assert_eq!(link("bssid=7e:7f:f0:76:08:8e\nssid=VRP\nwpa_state=4WAY_HANDSHAKE\n"), WifiLink::Connecting { ssid: "VRP".into(), step: JoinStep::Authenticating });
    assert_eq!(link("ssid=VRP\nid=0\nwpa_state=COMPLETED\n"), WifiLink::Connecting { ssid: "VRP".into(), step: JoinStep::GettingAddress });
    assert_eq!(link("ssid=VRP\nwpa_state=COMPLETED\nip_address=192.168.1.7\n"), WifiLink::Connected { ssid: "VRP".into(), address: "192.168.1.7".into() });
}

#[test]
fn secrets_stay_out_of_the_trace() {
    assert_eq!(redact("SET_NETWORK 0 psk \"hunter22\""), "SET_NETWORK 0 psk <secret>");
    assert_eq!(redact("SET_NETWORK 0 ssid 56525"), "SET_NETWORK 0 ssid 56525");
    let key = "ab".repeat(32);
    assert_eq!(network_settings("a\"b", Some(&Secret::new("pass word\""))), [("ssid", "612262".into()), ("psk", "\"pass word\"\"".into())]);
    assert_eq!(network_settings("x", Some(&Secret::new(key.clone()))), [("ssid", "78".into()), ("psk", key)]);
    assert_eq!(network_settings("x", None), [("ssid", "78".into()), ("key_mgmt", "NONE".into())]);
}

// --- Against a pretend wpa_supplicant --------------------------------------------------------

/// A pretend wpa_supplicant on a socket in a temporary directory: answers commands with
/// `answer`, records them, and sends messages to whoever attached.
struct Fake { dir: tempfile::TempDir, sent: Arc<Mutex<Vec<String>>>, attached: Arc<Mutex<Option<SocketAddr>>>, socket: Arc<UnixDatagram> }

impl Fake {
    fn new(answer: impl Fn(&str) -> String + Send + 'static) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let socket = Arc::new(UnixDatagram::bind(dir.path().join("wlan0")).unwrap());
        let (sent, attached) = (Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(None)));
        let (s, log, att) = (socket.clone(), sent.clone(), attached.clone());
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                let cmd = String::from_utf8_lossy(&buf[..n]).to_string();
                let reply = if cmd == "ATTACH" { *att.lock().unwrap() = Some(from.clone()); "OK\n".into() } else { answer(&cmd) };
                log.lock().unwrap().push(cmd);
                let _ = s.send_to_addr(reply.as_bytes(), &from);
            }
        });
        Fake { dir, sent, attached, socket }
    }

    fn say(&self, message: &str) {
        let to = self.attached.lock().unwrap().clone().expect("attached");
        self.socket.send_to_addr(message.as_bytes(), &to).unwrap();
    }

    fn sent(&self) -> Vec<String> { self.sent.lock().unwrap().clone() }
}

/// The device: always powers on (the fake is already there).
struct NoDevice;
impl WifiDevice for NoDevice { fn set_power(&mut self, _: bool) -> io::Result<()> { Ok(()) } }

/// wpa_supplicant's replies for a device with no saved networks, STATUS giving `status`.
fn answers(status: Arc<Mutex<&'static str>>) -> impl Fn(&str) -> String + Send {
    move |cmd| match cmd {
        "LIST_NETWORKS" => "network id / ssid / bssid / flags\n".into(),
        "SCAN_RESULTS" => SCAN.into(),
        "STATUS" => status.lock().unwrap().to_string(),
        "ADD_NETWORK" => "4\n".into(),
        _ => "OK\n".into(),
    }
}

const SCANNING: &str = "wpa_state=SCANNING\n";

fn started(fake: &Fake) -> (WpaSupplicant, mpsc::Receiver<WifiEvent>) {
    let mut wifi = WpaSupplicant::new(fake.dir.path().join("wlan0"), Box::new(NoDevice));
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    wifi.start(Box::new(move |e| { let _ = tx.lock().unwrap().send(e); }));
    (wifi, rx)
}

fn next(rx: &mpsc::Receiver<WifiEvent>) -> WifiEvent { rx.recv_timeout(Duration::from_secs(2)).expect("an event") }

#[test]
fn a_wrong_password_reports_and_removes_the_network() {
    let fake = Fake::new(answers(Arc::new(Mutex::new(SCANNING))));
    let (mut wifi, rx) = started(&fake);
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Idle));
    assert!(matches!(next(&rx), WifiEvent::Networks(n) if n.len() == 6));
    wifi.send(WifiCmd::Join { ssid: "VRP".into(), password: Some(Secret::new("wrongpassword1")) });
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Connecting { ssid: "VRP".into(), step: JoinStep::Associating }));
    fake.say("<3>Associated with 16:98:8f:bf:49:5e");
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Connecting { ssid: "VRP".into(), step: JoinStep::Authenticating }));
    fake.say("<3>CTRL-EVENT-DISCONNECTED bssid=16:98:8f:bf:49:5e reason=15");
    fake.say("<3>CTRL-EVENT-SSID-TEMP-DISABLED id=4 ssid=\"VRP\" auth_failures=1 duration=10 reason=WRONG_KEY");
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Idle));
    assert_eq!(next(&rx), WifiEvent::Failed(WifiError::WrongPassword { ssid: "VRP".into() }));
    let sent = fake.sent();
    let from = sent.iter().position(|c| c == "ADD_NETWORK").unwrap();
    assert_eq!(sent[from..], ["ADD_NETWORK", "SET_NETWORK 4 ssid 565250", "SET_NETWORK 4 psk \"wrongpassword1\"", "SELECT_NETWORK 4", "REMOVE_NETWORK 4", "ENABLE_NETWORK all"]);
}

#[test]
fn a_join_is_saved_once_connected_then_waits_for_an_address() {
    let status = Arc::new(Mutex::new(SCANNING));
    let fake = Fake::new(answers(status.clone()));
    let (mut wifi, rx) = started(&fake);
    next(&rx);
    next(&rx);
    wifi.send(WifiCmd::Join { ssid: "VRP".into(), password: Some(Secret::new("rightpassword")) });
    next(&rx);
    *status.lock().unwrap() = "ssid=VRP\nwpa_state=COMPLETED\n";
    fake.say("<3>CTRL-EVENT-CONNECTED - Connection to 16:98:8f:bf:49:5e completed [id=4 id_str=]");
    assert!(matches!(next(&rx), WifiEvent::Networks(_)));
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Connecting { ssid: "VRP".into(), step: JoinStep::GettingAddress }));
    assert!(fake.sent().contains(&"SAVE_CONFIG".to_string()));
    *status.lock().unwrap() = "ssid=VRP\nwpa_state=COMPLETED\nip_address=10.0.0.9\n";
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Connected { ssid: "VRP".into(), address: "10.0.0.9".into() }));
}

#[test]
fn out_of_range_only_after_a_scan_since_the_join() {
    let fake = Fake::new(answers(Arc::new(Mutex::new(SCANNING))));
    let (mut wifi, rx) = started(&fake);
    next(&rx);
    next(&rx);
    wifi.send(WifiCmd::Join { ssid: "Elsewhere".into(), password: None });
    next(&rx);
    fake.say("<3>CTRL-EVENT-NETWORK-NOT-FOUND ");
    fake.say("<3>CTRL-EVENT-SCAN-STARTED ");
    fake.say("<3>CTRL-EVENT-NETWORK-NOT-FOUND ");
    assert_eq!(next(&rx), WifiEvent::Link(WifiLink::Idle));
    assert_eq!(next(&rx), WifiEvent::Failed(WifiError::OutOfRange { ssid: "Elsewhere".into() }));
    assert!(fake.sent().contains(&"SET_NETWORK 4 key_mgmt NONE".to_string()));
}
