//! Whether this install is linked to an account, and to which.
//!
//! An account exists for exactly one reason: publishing through the relay. It
//! is the subscription, the handle, and the way links survive a dead laptop,
//! in one object. Nothing on the local network ever needs one - viewers pair,
//! they do not sign in - so "unlinked" is the whole product for most people
//! and is reported as a plain state, never as an error.
//!
//! The daemon never holds account credentials. Linking binds the *install key*
//! to an account on the cloud's side; what comes back down is only the public
//! outcome - a handle and a domain - which is why the whole status fits in
//! this little type.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where this install stands with an account.
///
/// Tagged like [`crate::ServingState`] so the TypeScript side can narrow on
/// `state` and the window can render each case as its own surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum AccountLink {
    /// No account. Every install starts here, and the free tier stays here -
    /// it is not a step on the way to somewhere else.
    Unlinked,
    /// The upgrade page is open in a browser and the daemon is asking the
    /// cloud whether the checkout has landed. A real state of its own: the
    /// window has to say "finish in your browser" rather than either "not
    /// linked" (looks broken) or "linked" (a lie for the whole checkout).
    Waiting,
    /// Linked. The handle is the account's name made routable: every app on
    /// this machine publishes as `<label>-<handle>.<domain>`.
    Linked { handle: String, domain: String },
}

/// The full answer to `account.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AccountStatus {
    /// This install's public identity, unpadded base64url - the value linking
    /// binds to an account. `None` when the keystore would not give one back
    /// this run (see the daemon's keys module), in which case there is nothing
    /// to link and `account.begin_upgrade` says so instead of guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub install_key: Option<String>,
    pub link: AccountLink,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_state_is_tagged_so_typescript_can_narrow_it() {
        let json = serde_json::to_value(AccountLink::Unlinked).expect("serialises");
        assert_eq!(json["state"], "unlinked");

        let linked = AccountLink::Linked {
            handle: "devi".into(),
            domain: "kitchentable.cloud".into(),
        };
        let json = serde_json::to_value(&linked).expect("serialises");
        assert_eq!(json["state"], "linked");
        assert_eq!(json["handle"], "devi");
        assert_eq!(json["domain"], "kitchentable.cloud");
    }

    #[test]
    fn an_install_with_no_identity_reports_that_rather_than_an_empty_string() {
        // The UI branches on "is there a key at all", and an empty string
        // would make that check a string comparison instead of a presence one.
        let status = AccountStatus {
            install_key: None,
            link: AccountLink::Unlinked,
        };
        let json = serde_json::to_value(&status).expect("serialises");
        assert!(json.get("install_key").is_none(), "absent, not null");
    }
}
