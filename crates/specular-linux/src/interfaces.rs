//! The network interfaces' addresses, from getifaddrs.

use std::ffi::CStr;
use std::io;
use std::net::Ipv4Addr;

use specular_core::wireless::wifi::Interface;

/// The IPv4 addresses of the interfaces that are up and have a link (IFF_RUNNING: a USB link
/// with no cable in isn't), loopback left out. IPv6 ones are left out too: what a user types
/// to reach the device is its IPv4 address, and a link-local IPv6 one can't be typed without
/// its interface.
pub fn addresses() -> io::Result<Vec<Interface>> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 { return Err(io::Error::last_os_error()); }
    let (wanted, unwanted) = ((libc::IFF_UP | libc::IFF_RUNNING) as u32, libc::IFF_LOOPBACK as u32);
    let mut out = vec![];
    let mut at = list;
    // Safety: getifaddrs gave a linked list, valid until freeifaddrs; an AF_INET address is a
    // sockaddr_in.
    while let Some(i) = unsafe { at.as_ref() } {
        at = i.ifa_next;
        let Some(addr) = (unsafe { i.ifa_addr.as_ref() }) else { continue };
        if i.ifa_flags & (wanted | unwanted) != wanted || addr.sa_family != libc::AF_INET as libc::sa_family_t { continue; }
        let sin = unsafe { &*(i.ifa_addr as *const libc::sockaddr_in) };
        let name = unsafe { CStr::from_ptr(i.ifa_name) }.to_string_lossy().into_owned();
        out.push(Interface { name, address: Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string() });
    }
    unsafe { libc::freeifaddrs(list) };
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_without_loopback() {
        let all = super::addresses().unwrap();
        assert!(all.iter().all(|i| i.name != "lo" && i.address.parse::<std::net::Ipv4Addr>().is_ok()), "{all:?}");
    }
}
