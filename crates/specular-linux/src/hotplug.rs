//! Display hotplug: the kernel's uevents for a DRM card that say its connectors may have changed
//! (`HOTPLUG=1`: a TV plugged in or out), read from a netlink socket. They say only that something
//! changed; the connectors must be read again to know what (`kms::Kms::find_tv`).

use std::io;

/// Whether `event` (one kernel uevent: `ACTION@DEVPATH`, then `KEY=VALUE` fields, NUL-separated)
/// is a hotplug event of the DRM card `card` (e.g. "card0").
pub fn is_hotplug(event: &[u8], card: &str) -> bool {
    let fields: Vec<&[u8]> = event.split(|&b| b == 0).collect();
    let has = |field: &str| fields.contains(&field.as_bytes());
    has("SUBSYSTEM=drm") && has("HOTPLUG=1") && has(&format!("DEVNAME=dri/{card}"))
}

/// `event` for the log: its fields, space-separated.
fn describe(event: &[u8]) -> String {
    event.split(|&b| b == 0).filter(|f| !f.is_empty()).map(String::from_utf8_lossy).collect::<Vec<_>>().join(" ")
}

/// Calls `changed` at each hotplug event of `card` (logged as a `hotplug:` line), blocking until
/// the socket can't be read. Events lost to a full socket buffer count as one hotplug event, as
/// one of them may have been.
pub fn watch(card: &str, mut changed: impl FnMut()) -> io::Result<()> {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let _socket = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    // Group 1: the kernel's own events (udev's re-sent ones are group 2).
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as u16;
    address.nl_groups = 1;
    if unsafe { libc::bind(fd, &address as *const _ as *const libc::sockaddr, size_of::<libc::sockaddr_nl>() as u32) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; 8192];
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::ENOBUFS) => { eprintln!("hotplug: events were lost; reading the connectors again"); changed(); continue; }
                _ => return Err(e),
            }
        }
        let event = &buf[..n as usize];
        if is_hotplug(event, card) {
            eprintln!("hotplug: {}", describe(event));
            changed();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_cards_hotplug_events_count() {
        let event = |fields: &[&str]| fields.join("\0").into_bytes();
        let hdmi = event(&["change@/devices/platform/display-subsystem/drm/card0", "ACTION=change", "DEVPATH=/devices/platform/display-subsystem/drm/card0",
            "SUBSYSTEM=drm", "HOTPLUG=1", "CONNECTOR=74", "DEVNAME=dri/card0", "DEVTYPE=drm_minor", "SEQNUM=2345", "MAJOR=226", "MINOR=0", ""]);
        assert!(super::is_hotplug(&hdmi, "card0"));
        assert!(!super::is_hotplug(&hdmi, "card1"));
        // A render node, an event without HOTPLUG=1, and another subsystem's.
        assert!(!super::is_hotplug(&event(&["change@/x/renderD128", "SUBSYSTEM=drm", "HOTPLUG=1", "DEVNAME=dri/renderD128"]), "card0"));
        assert!(!super::is_hotplug(&event(&["change@/x/card0", "SUBSYSTEM=drm", "DEVNAME=dri/card0"]), "card0"));
        assert!(!super::is_hotplug(&event(&["change@/x/card0", "SUBSYSTEM=power_supply", "HOTPLUG=1", "DEVNAME=dri/card0"]), "card0"));
        // Not fooled by a field that only starts the same.
        assert!(!super::is_hotplug(&event(&["change@/x/card0", "SUBSYSTEM=drm", "HOTPLUG=10", "DEVNAME=dri/card0"]), "card0"));
        assert_eq!(super::describe(&hdmi[..60]), "change@/devices/platform/display-subsystem/drm/card0 ACTION=");
    }
}
