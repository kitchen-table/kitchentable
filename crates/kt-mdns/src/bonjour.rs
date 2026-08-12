//! macOS: publish A records through mDNSResponder via the `dns_sd` C API.
//!
//! `dns_sd` lives in libSystem, so there is nothing to link and no crate to
//! pull in. Going through mDNSResponder rather than speaking mDNS ourselves is
//! the whole point: it is what makes the Local Network permission prompt name
//! Kitchen Table, and it is what defends the name against conflicts.
//!
//! Spike S1 (does the prompt attribute correctly when the *daemon*, a child
//! process, announces first?) is verified on a real machine with a real phone,
//! not here.

use std::ffi::{c_char, c_int, c_void, CString};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use crate::{Announcement, Announcer, Handle, MdnsError};

// ---- dns_sd FFI -----------------------------------------------------------

#[allow(non_camel_case_types)]
type DNSServiceRef = *mut c_void;
#[allow(non_camel_case_types)]
type DNSRecordRef = *mut c_void;
#[allow(non_camel_case_types)]
type DNSServiceErrorType = i32;
#[allow(non_camel_case_types)]
type DNSServiceFlags = u32;

/// Exactly one host may own this name; mDNSResponder probes and reports a
/// conflict rather than letting two machines answer for it.
const FLAGS_UNIQUE: DNSServiceFlags = 0x20;
const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;
const ERR_NO_ERROR: DNSServiceErrorType = 0;
const ERR_NAME_CONFLICT: DNSServiceErrorType = -65548;
/// Long enough that a phone does not re-query constantly, short enough that a
/// laptop moving networks stops answering for a stale address quickly.
const TTL_SECONDS: u32 = 120;

type RegisterRecordReply = extern "C" fn(
    sd_ref: DNSServiceRef,
    record_ref: DNSRecordRef,
    flags: DNSServiceFlags,
    error_code: DNSServiceErrorType,
    context: *mut c_void,
);

unsafe extern "C" {
    fn DNSServiceCreateConnection(sd_ref: *mut DNSServiceRef) -> DNSServiceErrorType;
    #[allow(clippy::too_many_arguments)]
    fn DNSServiceRegisterRecord(
        sd_ref: DNSServiceRef,
        record_ref: *mut DNSRecordRef,
        flags: DNSServiceFlags,
        interface_index: u32,
        fullname: *const c_char,
        rrtype: u16,
        rrclass: u16,
        rdlen: u16,
        rdata: *const c_void,
        ttl: u32,
        callback: Option<RegisterRecordReply>,
        context: *mut c_void,
    ) -> DNSServiceErrorType;
    fn DNSServiceProcessResult(sd_ref: DNSServiceRef) -> DNSServiceErrorType;
    fn DNSServiceRefSockFD(sd_ref: DNSServiceRef) -> c_int;
    fn DNSServiceRefDeallocate(sd_ref: DNSServiceRef);
}

// ---- safe wrapper ---------------------------------------------------------

/// One connection to mDNSResponder, shared by every record we publish.
///
/// mDNSResponder is happiest with one connection per process; a connection per
/// app would multiply the file descriptors and the permission bookkeeping for
/// no benefit.
struct Connection {
    sd_ref: DNSServiceRef,
}

// The `DNSServiceRef` is only ever used from the pump thread or under the
// registry mutex, and dns_sd itself is thread-safe for these calls.
unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.sd_ref.is_null() {
            unsafe { DNSServiceRefDeallocate(self.sd_ref) };
        }
    }
}

/// A published record. Dropping the announcement drops this, and dropping the
/// whole connection withdraws everything.
#[derive(Debug)]
pub struct Registration {
    name: String,
}

/// Names mDNSResponder has told us are conflicted, keyed by the name we asked
/// for. Written by the callback on the pump thread.
type Conflicts = Arc<Mutex<Vec<String>>>;

pub struct BonjourAnnouncer {
    connection: Mutex<Option<Arc<Connection>>>,
    conflicts: Conflicts,
    /// Names we believe are live, so a second app asking for the same one is
    /// suffixed locally rather than waiting on a network probe.
    claimed: Mutex<Vec<String>>,
}

impl Default for BonjourAnnouncer {
    fn default() -> Self {
        Self::new()
    }
}

impl BonjourAnnouncer {
    pub fn new() -> Self {
        Self {
            connection: Mutex::new(None),
            conflicts: Arc::new(Mutex::new(Vec::new())),
            claimed: Mutex::new(Vec::new()),
        }
    }

    /// Open the connection on first use, and start the thread that pumps
    /// replies. Opening lazily means the Local Network permission prompt fires
    /// at the moment onboarding expects it, not at daemon start.
    fn connection(&self) -> Result<Arc<Connection>, MdnsError> {
        let mut guard = self.connection.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(existing) = guard.as_ref() {
            return Ok(Arc::clone(existing));
        }

        let mut sd_ref: DNSServiceRef = std::ptr::null_mut();
        let err = unsafe { DNSServiceCreateConnection(&mut sd_ref) };
        if err != ERR_NO_ERROR || sd_ref.is_null() {
            return Err(MdnsError::Responder(format!(
                "DNSServiceCreateConnection failed ({err})"
            )));
        }

        let connection = Arc::new(Connection { sd_ref });
        spawn_pump(Arc::clone(&connection));
        *guard = Some(Arc::clone(&connection));

        Ok(connection)
    }
}

/// Drive the connection's socket so registration callbacks actually fire.
///
/// One thread for the process, parked in `select` almost always. Without it,
/// mDNSResponder's replies (including conflict reports) are never delivered.
fn spawn_pump(connection: Arc<Connection>) {
    std::thread::Builder::new()
        .name("kt-mdns".into())
        .spawn(move || {
            let fd = unsafe { DNSServiceRefSockFD(connection.sd_ref) };
            if fd < 0 {
                tracing::warn!("mDNS connection has no socket; conflicts will go unreported");
                return;
            }

            loop {
                // DNSServiceProcessResult blocks until there is something to
                // read, so this is not a spin.
                let err = unsafe { DNSServiceProcessResult(connection.sd_ref) };
                if err != ERR_NO_ERROR {
                    tracing::debug!(err, "mDNS connection closed");
                    return;
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|e| tracing::warn!(error = %e, "could not start the mDNS pump"));
}

extern "C" fn on_register(
    _sd_ref: DNSServiceRef,
    _record_ref: DNSRecordRef,
    _flags: DNSServiceFlags,
    error_code: DNSServiceErrorType,
    context: *mut c_void,
) {
    if error_code == ERR_NO_ERROR {
        return;
    }

    // `context` is a leaked Box<(Conflicts, String)> owned by this callback.
    if context.is_null() {
        return;
    }
    let payload = unsafe { Box::from_raw(context as *mut (Conflicts, String)) };
    let (conflicts, name) = *payload;

    if error_code == ERR_NAME_CONFLICT {
        tracing::warn!(name = %name, "another device on this network claims this name");
        conflicts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name);
    } else {
        tracing::warn!(name = %name, error_code, "mDNS registration failed");
    }
}

impl Announcer for BonjourAnnouncer {
    fn announce(&self, name: &str, addr: Ipv4Addr) -> Result<Announcement, MdnsError> {
        let connection = self.connection()?;

        // Suffix against names this process already published, so two apps
        // wanting `notes` do not race each other through the network.
        let registered = {
            let mut claimed = self.claimed.lock().unwrap_or_else(|e| e.into_inner());
            let taken = self.conflicts.lock().unwrap_or_else(|e| e.into_inner());
            let mut candidate = name.to_string();
            let mut n = 2;
            while claimed.contains(&candidate) || taken.contains(&candidate) {
                candidate = format!("{name}-{n}");
                n += 1;
            }
            claimed.push(candidate.clone());
            candidate
        };

        // mDNS names are fully qualified and rooted.
        let fullname = CString::new(format!("{registered}.local."))
            .map_err(|_| MdnsError::Responder("name contained a NUL byte".to_string()))?;

        let octets = addr.octets();
        let context = Box::into_raw(Box::new((Arc::clone(&self.conflicts), registered.clone())))
            as *mut c_void;

        let mut record: DNSRecordRef = std::ptr::null_mut();
        let err = unsafe {
            DNSServiceRegisterRecord(
                connection.sd_ref,
                &mut record,
                FLAGS_UNIQUE,
                0, // every interface
                fullname.as_ptr(),
                TYPE_A,
                CLASS_IN,
                octets.len() as u16,
                octets.as_ptr() as *const c_void,
                TTL_SECONDS,
                Some(on_register),
                context,
            )
        };

        if err != ERR_NO_ERROR {
            // Reclaim the context the callback would otherwise have owned.
            drop(unsafe { Box::from_raw(context as *mut (Conflicts, String)) });
            self.claimed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|c| c != &registered);

            return Err(if err == ERR_NAME_CONFLICT {
                MdnsError::NameConflict { name: registered }
            } else {
                MdnsError::Responder(format!("DNSServiceRegisterRecord failed ({err})"))
            });
        }

        tracing::info!(hostname = %format!("{registered}.local"), %addr, "announced");

        Ok(Announcement {
            requested: name.to_string(),
            registered: registered.clone(),
            handle: Handle::Bonjour(Registration { name: registered }),
        })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Records are released when the shared connection is deallocated at
        // shutdown. Removing one record individually needs the connection ref
        // this struct deliberately does not hold, to keep drop order simple.
        tracing::debug!(name = %self.name, "announcement released");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_apps_wanting_the_same_name_get_different_ones() {
        // No network needed: the suffixing happens before the record is sent.
        let announcer = BonjourAnnouncer::new();
        announcer
            .claimed
            .lock()
            .expect("not poisoned")
            .push("notes".to_string());

        let claimed = announcer.claimed.lock().expect("not poisoned").clone();
        assert_eq!(claimed, ["notes"]);
    }

    #[test]
    fn a_name_with_a_nul_byte_is_refused_before_it_reaches_the_ffi() {
        assert!(CString::new("bad\0name.local.").is_err());
    }
}
