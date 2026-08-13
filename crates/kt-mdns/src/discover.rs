//! macOS: ask the network what a device calls itself.
//!
//! A browser will never tell us. There is no web API for a device's name, on
//! purpose - it is a fingerprinting vector - so the only thing an HTTP request
//! carries is a user agent, which gives a model class at best and lies outright
//! on an iPad in desktop mode.
//!
//! The network is more forthcoming. Devices announce services under the name
//! their owner gave them: an iPhone called "Priya's iPhone" in Settings
//! advertises `_companion-link._tcp` under exactly that string. So given the
//! address that just asked to pair, we browse, resolve each instance to a
//! hostname, resolve that to an address, and take the name whose address
//! matches.
//!
//! This is a suggestion, not an identification. It is matched on IP, which the
//! device asserts and a hostile one could lie about; nothing is authorised on
//! the strength of it and the owner sees and edits the name before anything is
//! approved. What it buys is that the prompt says "Priya's iPhone" instead of
//! "iPhone", which is the difference between a device list someone can revoke
//! from and one they cannot.
//!
//! Reachability, measured rather than assumed:
//! - Apple devices answer reliably while awake and unlocked.
//! - Windows answers via `_smb`, usually.
//! - Android usually announces nothing. Those stay on the user-agent guess.
//! - Reverse PTR (`x.y.z.w.in-addr.arpa`) is *not* answered by any device
//!   tested, and `getnameinfo` does not consult mDNSResponder at all, so
//!   neither of the obvious shortcuts works.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::Discoverer;

// ---- dns_sd FFI -----------------------------------------------------------

#[allow(non_camel_case_types)]
type DNSServiceRef = *mut c_void;
#[allow(non_camel_case_types)]
type DNSServiceErrorType = i32;
#[allow(non_camel_case_types)]
type DNSServiceFlags = u32;

const ERR_NO_ERROR: DNSServiceErrorType = 0;
/// `kDNSServiceProtocol_IPv4`. A phone's IPv6 address is usually privacy-random
/// and changes; the v4 one is what the HTTP request arrived from.
const PROTOCOL_IPV4: u32 = 0x01;
const AF_INET: u8 = 2;

type BrowseReply = extern "C" fn(
    sd_ref: DNSServiceRef,
    flags: DNSServiceFlags,
    interface_index: u32,
    error_code: DNSServiceErrorType,
    service_name: *const c_char,
    regtype: *const c_char,
    reply_domain: *const c_char,
    context: *mut c_void,
);

type ResolveReply = extern "C" fn(
    sd_ref: DNSServiceRef,
    flags: DNSServiceFlags,
    interface_index: u32,
    error_code: DNSServiceErrorType,
    fullname: *const c_char,
    host_target: *const c_char,
    port: u16,
    txt_len: u16,
    txt_record: *const u8,
    context: *mut c_void,
);

type AddrInfoReply = extern "C" fn(
    sd_ref: DNSServiceRef,
    flags: DNSServiceFlags,
    interface_index: u32,
    error_code: DNSServiceErrorType,
    hostname: *const c_char,
    address: *const SockaddrIn,
    ttl: u32,
    context: *mut c_void,
);

/// macOS `sockaddr_in`, which carries its own length in the first byte.
#[repr(C)]
struct SockaddrIn {
    len: u8,
    family: u8,
    port: u16,
    addr: [u8; 4],
    zero: [u8; 8],
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x0001;

unsafe extern "C" {
    fn DNSServiceBrowse(
        sd_ref: *mut DNSServiceRef,
        flags: DNSServiceFlags,
        interface_index: u32,
        regtype: *const c_char,
        domain: *const c_char,
        callback: Option<BrowseReply>,
        context: *mut c_void,
    ) -> DNSServiceErrorType;

    #[allow(clippy::too_many_arguments)]
    fn DNSServiceResolve(
        sd_ref: *mut DNSServiceRef,
        flags: DNSServiceFlags,
        interface_index: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        callback: Option<ResolveReply>,
        context: *mut c_void,
    ) -> DNSServiceErrorType;

    fn DNSServiceGetAddrInfo(
        sd_ref: *mut DNSServiceRef,
        flags: DNSServiceFlags,
        interface_index: u32,
        protocol: u32,
        hostname: *const c_char,
        callback: Option<AddrInfoReply>,
        context: *mut c_void,
    ) -> DNSServiceErrorType;

    fn DNSServiceProcessResult(sd_ref: DNSServiceRef) -> DNSServiceErrorType;
    fn DNSServiceRefSockFD(sd_ref: DNSServiceRef) -> c_int;
    fn DNSServiceRefDeallocate(sd_ref: DNSServiceRef);

    fn poll(fds: *mut PollFd, nfds: c_uint, timeout: c_int) -> c_int;
}

/// Deallocates its `DNSServiceRef` however the scope exits.
struct Query(DNSServiceRef);

impl Drop for Query {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DNSServiceRefDeallocate(self.0) };
        }
    }
}

/// Service types whose instance name is the device's own name.
///
/// Ordered by how often the name is exactly what the owner typed. `_raop` and
/// `_workstation` decorate theirs, which [`tidy`] undoes.
const SERVICES: [&str; 5] = [
    "_companion-link._tcp",
    "_airplay._tcp",
    "_smb._tcp",
    "_device-info._tcp",
    "_workstation._tcp",
];

/// Pump replies for one query until `deadline`, or until `done` says stop.
///
/// `DNSServiceProcessResult` blocks, so it is only called once `poll` says the
/// socket has something on it - otherwise a device that never answers would
/// hang the lookup rather than time out.
fn pump(sd_ref: DNSServiceRef, deadline: Instant, done: &dyn Fn() -> bool) {
    let fd = unsafe { DNSServiceRefSockFD(sd_ref) };
    if fd < 0 {
        return;
    }

    while !done() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }

        let mut fds = PollFd {
            fd,
            events: POLLIN,
            revents: 0,
        };
        let ms = remaining.as_millis().min(c_int::MAX as u128) as c_int;
        let ready = unsafe { poll(&mut fds, 1, ms) };
        if ready <= 0 {
            return; // timed out, or the socket went away
        }
        if unsafe { DNSServiceProcessResult(sd_ref) } != ERR_NO_ERROR {
            return;
        }
    }
}

/// Instance names seen for one service type.
type Found = Arc<Mutex<Vec<String>>>;

extern "C" fn on_browse(
    _sd_ref: DNSServiceRef,
    _flags: DNSServiceFlags,
    _interface_index: u32,
    error_code: DNSServiceErrorType,
    service_name: *const c_char,
    _regtype: *const c_char,
    _reply_domain: *const c_char,
    context: *mut c_void,
) {
    if error_code != ERR_NO_ERROR || service_name.is_null() || context.is_null() {
        return;
    }
    let Ok(name) = (unsafe { CStr::from_ptr(service_name) }).to_str() else {
        return;
    };
    let found = unsafe { &*(context as *const Found) };
    if let Ok(mut names) = found.lock() {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
}

extern "C" fn on_resolve(
    _sd_ref: DNSServiceRef,
    _flags: DNSServiceFlags,
    _interface_index: u32,
    error_code: DNSServiceErrorType,
    _fullname: *const c_char,
    host_target: *const c_char,
    _port: u16,
    _txt_len: u16,
    _txt_record: *const u8,
    context: *mut c_void,
) {
    if error_code != ERR_NO_ERROR || host_target.is_null() || context.is_null() {
        return;
    }
    let Ok(host) = (unsafe { CStr::from_ptr(host_target) }).to_str() else {
        return;
    };
    let slot = unsafe { &*(context as *const Arc<Mutex<Option<String>>>) };
    if let Ok(mut out) = slot.lock() {
        out.get_or_insert_with(|| host.to_string());
    }
}

extern "C" fn on_address(
    _sd_ref: DNSServiceRef,
    _flags: DNSServiceFlags,
    _interface_index: u32,
    error_code: DNSServiceErrorType,
    _hostname: *const c_char,
    address: *const SockaddrIn,
    _ttl: u32,
    context: *mut c_void,
) {
    if error_code != ERR_NO_ERROR || address.is_null() || context.is_null() {
        return;
    }
    let sockaddr = unsafe { &*address };
    if sockaddr.family != AF_INET {
        return;
    }
    let found = Ipv4Addr::from(sockaddr.addr);
    let slot = unsafe { &*(context as *const Arc<Mutex<Vec<Ipv4Addr>>>) };
    if let Ok(mut addrs) = slot.lock() {
        addrs.push(found);
    }
}

fn browse(service: &str, deadline: Instant) -> Vec<String> {
    let Ok(regtype) = CString::new(service) else {
        return Vec::new();
    };
    let found: Found = Arc::new(Mutex::new(Vec::new()));

    let mut sd_ref: DNSServiceRef = std::ptr::null_mut();
    let err = unsafe {
        DNSServiceBrowse(
            &mut sd_ref,
            0,
            0,
            regtype.as_ptr(),
            std::ptr::null(),
            Some(on_browse),
            &found as *const Found as *mut c_void,
        )
    };
    if err != ERR_NO_ERROR {
        return Vec::new();
    }
    let query = Query(sd_ref);

    // Browsing never "finishes" - it is a subscription - so this always runs to
    // the deadline. That is why the deadline is short and the whole lookup
    // happens off the request path.
    pump(query.0, deadline, &|| false);

    found.lock().map(|n| n.clone()).unwrap_or_default()
}

fn host_for(instance: &str, service: &str, deadline: Instant) -> Option<String> {
    let (name, regtype) = (CString::new(instance).ok()?, CString::new(service).ok()?);
    let slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let mut sd_ref: DNSServiceRef = std::ptr::null_mut();
    let err = unsafe {
        DNSServiceResolve(
            &mut sd_ref,
            0,
            0,
            name.as_ptr(),
            regtype.as_ptr(),
            c"local.".as_ptr(),
            Some(on_resolve),
            &slot as *const Arc<Mutex<Option<String>>> as *mut c_void,
        )
    };
    if err != ERR_NO_ERROR {
        return None;
    }
    let query = Query(sd_ref);
    pump(query.0, deadline, &|| {
        slot.lock().map(|s| s.is_some()).unwrap_or(true)
    });

    let host = slot.lock().ok()?.clone();
    host
}

fn addresses_of(host: &str, deadline: Instant) -> Vec<Ipv4Addr> {
    let Ok(hostname) = CString::new(host) else {
        return Vec::new();
    };
    let slot: Arc<Mutex<Vec<Ipv4Addr>>> = Arc::new(Mutex::new(Vec::new()));

    let mut sd_ref: DNSServiceRef = std::ptr::null_mut();
    let err = unsafe {
        DNSServiceGetAddrInfo(
            &mut sd_ref,
            0,
            0,
            PROTOCOL_IPV4,
            hostname.as_ptr(),
            Some(on_address),
            &slot as *const Arc<Mutex<Vec<Ipv4Addr>>> as *mut c_void,
        )
    };
    if err != ERR_NO_ERROR {
        return Vec::new();
    }
    let query = Query(sd_ref);
    pump(query.0, deadline, &|| {
        slot.lock().map(|a| !a.is_empty()).unwrap_or(true)
    });

    slot.lock().map(|a| a.clone()).unwrap_or_default()
}

/// Strip the decoration some service types add to the device's name.
///
/// `_raop` prefixes the MAC address (`AABBCCDDEEFF@Living Room`) and
/// `_workstation` appends it (`desktop [aa:bb:cc:dd:ee:ff]`). Neither is
/// something to put in front of a person.
/// Six colon-separated hex pairs, and nothing else.
///
/// Checked properly rather than "has brackets in it": a device someone named
/// "Studio [the good one]" should keep the name they chose.
fn is_mac(candidate: &str) -> bool {
    let mut parts = 0;
    for part in candidate.split(':') {
        if part.len() != 2 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        parts += 1;
    }
    parts == 6
}

pub(crate) fn tidy(name: &str) -> String {
    let without_prefix = match name.split_once('@') {
        Some((mac, rest)) if mac.len() == 12 && mac.chars().all(|c| c.is_ascii_hexdigit()) => rest,
        _ => name,
    };
    let without_suffix = match without_prefix.rsplit_once(" [") {
        Some((rest, tail)) if is_mac(tail.trim_end_matches(']')) => rest,
        _ => without_prefix,
    };
    without_suffix.trim().to_string()
}

pub struct BonjourDiscoverer;

impl Discoverer for BonjourDiscoverer {
    fn name_for(&self, addr: Ipv4Addr, within: Duration) -> Option<String> {
        let deadline = Instant::now() + within;

        for service in SERVICES {
            if Instant::now() >= deadline {
                break;
            }
            // A slice of the budget per service type, so one silent service
            // cannot eat the whole allowance and starve the rest.
            let slice = Instant::now()
                + (deadline.saturating_duration_since(Instant::now()))
                    .min(within / SERVICES.len() as u32);

            for instance in browse(service, slice) {
                if Instant::now() >= deadline {
                    break;
                }
                let Some(host) = host_for(&instance, service, deadline) else {
                    continue;
                };
                if addresses_of(&host, deadline).contains(&addr) {
                    let name = tidy(&instance);
                    if !name.is_empty() {
                        tracing::debug!(%addr, name, service, "device named itself");
                        return Some(name);
                    }
                }
            }
        }

        tracing::debug!(%addr, "no device on the network claims this address");
        None
    }

    fn is_live(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raop_and_workstation_decorations_are_stripped() {
        assert_eq!(tidy("AABBCCDDEEFF@Living Room"), "Living Room");
        assert_eq!(tidy("desktop [aa:bb:cc:dd:ee:ff]"), "desktop");
        assert_eq!(tidy("Kitchen iPad"), "Kitchen iPad");
    }

    #[test]
    fn a_name_that_merely_contains_an_at_sign_is_left_alone() {
        // Only a 12-hex-digit prefix is a MAC address; an email-ish name is not.
        assert_eq!(tidy("me@home iMac"), "me@home iMac");
        assert_eq!(tidy("Studio [not a mac]"), "Studio [not a mac]");
    }

    #[test]
    fn looking_up_an_address_nothing_claims_gives_up_within_its_budget() {
        // TEST-NET-1: guaranteed not to be on this network.
        let started = Instant::now();
        let name = BonjourDiscoverer.name_for(Ipv4Addr::new(192, 0, 2, 1), Duration::from_secs(2));

        assert_eq!(name, None);
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "took {:?}; a lookup must not hang the caller",
            started.elapsed()
        );
    }
}
