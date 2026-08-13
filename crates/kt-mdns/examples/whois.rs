//! Ask the network what the device at an address calls itself.
//!
//! `cargo run -p kt-mdns --example whois -- 192.168.0.115`
//!
//! Exists because this is the one part of pairing that cannot be tested without
//! a real network and a real device: whether anything answers at all is a
//! property of the phone in your hand, not of this code.

fn main() {
    let addr: std::net::Ipv4Addr = std::env::args()
        .nth(1)
        .expect("usage: whois <ipv4>")
        .parse()
        .expect("not an IPv4 address");

    let started = std::time::Instant::now();
    let discoverer = kt_mdns::discoverer_for_this_platform();
    let name = discoverer.name_for(addr, kt_mdns::NAME_LOOKUP_BUDGET);

    println!(
        "{addr} -> {}  ({:?})",
        name.unwrap_or_else(|| "<nothing announced>".into()),
        started.elapsed()
    );
}
