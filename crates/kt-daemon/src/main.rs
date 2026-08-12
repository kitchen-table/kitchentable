//! Kitchen Table daemon: wiring, event bus, supervision entrypoint.
//!
//! The daemon is the product. The Tauri shell, the CLI, and agents are all
//! clients of its Unix domain socket API (docs/architecture.md section 4).

fn main() {
    println!("kt-daemon {} (scaffold)", env!("CARGO_PKG_VERSION"));
}
