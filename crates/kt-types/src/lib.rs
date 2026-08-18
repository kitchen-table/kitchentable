//! Shared types for the Kitchen Table daemon socket API.
//!
//! This crate is the single source of truth for the protocol. The daemon, the
//! CLI, the MCP server, and (via ts-rs) the React UI all speak these types, so
//! the shell and daemon cannot drift silently.
//!
//! # What is exported to TypeScript, and what is not
//!
//! Only *client-facing* types carry `#[ts(export)]`: the things a caller sends
//! and receives. The JSON-RPC envelope stays Rust-side, because the UI never
//! sees it - the shell proxies Tauri IPC onto the socket and unwraps the
//! envelope on the way (docs/architecture.md section 13). Likewise
//! [`AppManifest`] is the on-disk `app.json` schema, owned by the daemon; the
//! UI reads [`App`] instead.
//!
//! # Adding a method
//!
//! Every mutating method must be a single idempotent call that a stateless
//! caller can make with no prior context. That constraint is what lets the MCP
//! tool surface in D6 be a thin projection of this API rather than a rewrite of
//! it (docs/architecture.md section 12).

pub mod account;
pub mod app;
pub mod event;
pub mod notify;
pub mod paths;
pub mod protocol;
pub mod status;

pub use account::{AccountLink, AccountStatus};
pub use app::{
    normalise_public_label, App, AppManifest, AppRecord, PublicLabelError, RelayMode, StorageMode,
    Urls, Visibility, MAX_DNS_LABEL,
};
pub use event::{Event, Viewer};
pub use notify::NotifyPrefs;
pub use protocol::{ErrorCode, KtError, Request, Response, PROTOCOL_VERSION};
pub use status::{DegradedReason, RelayState, ServingState, SysStatus};
