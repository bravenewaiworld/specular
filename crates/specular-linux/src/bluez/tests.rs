use super::*;

fn raw(props: &[(&str, Value)]) -> HashMap<String, OwnedValue> {
    props.iter().map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap())).collect()
}

fn ifaces(list: &[(&str, &[(&str, Value)])]) -> HashMap<String, HashMap<String, OwnedValue>> {
    list.iter().map(|(i, p)| (i.to_string(), raw(p))).collect()
}

const HCI: &str = "/org/bluez/hci0";
const HEADSET: &str = "/org/bluez/hci0/dev_00_11_22_33_44_55";

fn headset() -> HashMap<String, HashMap<String, OwnedValue>> {
    let adapter = Value::ObjectPath(HCI.try_into().unwrap());
    ifaces(&[
        (DEVICE, &[("Address", "00:11:22:33:44:55".into()), ("Name", "WH-1000XM4".into()), ("Icon", "audio-headset".into()),
                   ("Paired", true.into()), ("Connected", true.into()), ("Adapter", adapter), ("RSSI", Value::I16(-60))]),
        ("org.freedesktop.DBus.Introspectable", &[]),
    ])
}

fn objects() -> Objects {
    let mut o = Objects::default();
    o.added(HCI, &ifaces(&[(ADAPTER, &[("Powered", true.into()), ("Discovering", false.into()), ("Name", "brick".into())])]));
    o.added(HEADSET, &headset());
    // A device found by searching, with no name yet: not reported.
    let adapter = Value::ObjectPath(HCI.try_into().unwrap());
    o.added("/org/bluez/hci0/dev_AA", &ifaces(&[(DEVICE, &[("Address", "AA:AA:AA:AA:AA:AA".into()), ("Adapter", adapter)])]));
    o
}

#[test]
fn keeps_only_what_it_reads() {
    let o = objects();
    assert_eq!(o.0[HEADSET].len(), 1, "only Device1");
    assert!(!o.0[HEADSET][DEVICE].contains_key("RSSI"));
    assert_eq!(o.adapter(), Some(HCI));
}

#[test]
fn reports_named_devices_with_sound_once_a_transport_is_there() {
    let mut o = objects();
    let device = BtDevice { address: BtAddress("00:11:22:33:44:55".into()), name: "WH-1000XM4".into(), kind: BtKind::Headphones, paired: true, connected: true, audio: false };
    assert_eq!(o.devices(HCI), vec![device.clone()]);
    let transport = format!("{HEADSET}/fd0");
    let on = Value::ObjectPath(HEADSET.try_into().unwrap());
    o.added(&transport, &ifaces(&[(TRANSPORT, &[("Device", on), ("UUID", A2DP_SOURCE.into()), ("State", "idle".into())])]));
    assert_eq!(o.devices(HCI), vec![BtDevice { audio: true, ..device.clone() }]);
    // Disconnecting: not taking sound, even before the transport goes.
    o.changed(HEADSET, DEVICE, kept_props(DEVICE, &raw(&[("Connected", false.into())])).unwrap(), &[]);
    assert_eq!(o.devices(HCI), vec![BtDevice { connected: false, ..device.clone() }]);
    o.removed(&transport, &[TRANSPORT.into()]);
    assert!(!o.0.contains_key(&transport));
}

#[test]
fn properties_change_and_go() {
    let mut o = objects();
    o.changed(HCI, ADAPTER, kept_props(ADAPTER, &raw(&[("Discovering", true.into())])).unwrap(), &[]);
    assert_eq!(o.state(HCI), State { powered: true, searching: true, devices: o.devices(HCI) });
    o.changed(HEADSET, DEVICE, Props::new(), &["Name".into()]);
    assert_eq!(o.devices(HCI), vec![]);
    assert_eq!(o.device(&BtAddress("00:11:22:33:44:55".into())), Some((HEADSET.into(), "00:11:22:33:44:55".into())));
    // Unknown objects and interfaces are ignored.
    o.changed("/nowhere", DEVICE, Props::new(), &[]);
    assert!(!o.0.contains_key("/nowhere"));
}

#[test]
fn reports_only_what_changed() {
    let a = State { powered: true, searching: false, devices: vec![] };
    assert_eq!(changes(None, &a), vec![BtEvent::Powered(true), BtEvent::Searching(false), BtEvent::Devices(vec![])]);
    assert_eq!(changes(Some(&a), &a), vec![]);
    assert_eq!(changes(Some(&a), &State { searching: true, ..a.clone() }), vec![BtEvent::Searching(true)]);
}

#[test]
fn kinds_from_icons() {
    assert_eq!(kind("audio-headphones"), BtKind::Headphones);
    assert_eq!(kind("audio-card"), BtKind::Speaker);
    assert_eq!(kind("input-gaming"), BtKind::Controller);
    assert_eq!(kind(""), BtKind::Other);
}

#[test]
fn errors() {
    let e = bt_error("pair", "JBL", "org.bluez.Error.AuthenticationFailed: Authentication Failed".into());
    assert_eq!(e, BtError::PairingFailed { name: "JBL".into(), detail: "org.bluez.Error.AuthenticationFailed: Authentication Failed".into() });
    assert!(matches!(bt_error("connect", "JBL", "org.bluez.Error.Failed: br-connection-page-timeout".into()), BtError::ConnectFailed { .. }));
    assert!(matches!(bt_error("power", "", "org.freedesktop.DBus.Error.ServiceUnknown: The name is not activatable".into()), BtError::NotAvailable { .. }));
    assert_eq!(bt_error("forget", "", "x".into()), BtError::Service { step: "forget".into(), detail: "x".into() });
    assert_eq!(tolerate(Err::<(), _>("org.bluez.Error.InProgress: Operation already in progress".into()), &["org.bluez.Error.InProgress"]), Ok(()));
    assert!(tolerate(Err::<(), _>("org.bluez.Error.InProgressX: no".into()), &["org.bluez.Error.InProgress"]).is_err());
}
