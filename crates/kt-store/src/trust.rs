//! Devices, invites, and the access log.
//!
//! Split from the app storage because it answers a different question: not
//! "what is here" but "who may see it, and who has".

use kt_auth::{Device, DeviceId, DeviceStatus, Invite, InvitePolicy, InviteToken, NamedBy};
use rusqlite::{params, OptionalExtension};

use crate::{Store, StoreError};

/// One line in the access log.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessEvent {
    pub at: i64,
    pub app_slug: Option<String>,
    pub device_id: Option<String>,
    /// Who did it: `owner`, `viewer`, `agent`, `cli`, `system`.
    pub actor: String,
    /// What happened: `opened`, `paired`, `denied`, `revoked`, `deployed`.
    pub action: String,
    pub detail: Option<String>,
}

impl Store {
    // ---- devices ----------------------------------------------------------

    pub fn upsert_device(&self, device: &Device) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO devices (id, name, status, user_agent, fingerprint, first_seen, last_seen, named_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                user_agent = excluded.user_agent,
                last_seen = excluded.last_seen",
            params![
                device.id.as_str(),
                device.name,
                status_str(device.status),
                device.user_agent,
                device.fingerprint,
                device.first_seen,
                device.last_seen,
                named_by_str(device.named_by),
            ],
        )?;
        Ok(())
    }

    pub fn get_device(&self, id: &DeviceId) -> Result<Option<Device>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, status, user_agent, fingerprint, first_seen, last_seen, named_by
             FROM devices WHERE id = ?1",
        )?;
        stmt.query_row(params![id.as_str()], |row| Ok(row_to_device(row)))
            .optional()?
            .transpose()
    }

    pub fn list_devices(&self) -> Result<Vec<Device>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, status, user_agent, fingerprint, first_seen, last_seen, named_by
             FROM devices ORDER BY last_seen DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok(row_to_device(row)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn set_device_status(
        &self,
        id: &DeviceId,
        status: DeviceStatus,
    ) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE devices SET status = ?2 WHERE id = ?1",
            params![id.as_str(), status_str(status)],
        )?;
        Ok(n > 0)
    }

    /// The owner naming a device. Final: nothing overwrites this afterwards.
    pub fn rename_device(&self, id: &DeviceId, name: &str) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE devices SET name = ?2, named_by = 'owner' WHERE id = ?1",
            params![id.as_str(), name],
        )?;
        Ok(n > 0)
    }

    /// The network answering with the name a device publishes for itself.
    ///
    /// Only replaces a guess. The lookup is slow and lands whenever it lands,
    /// so without the `named_by = 'guess'` guard it could arrive after the
    /// owner has already typed something and quietly undo them.
    ///
    /// Returns whether the name actually changed.
    pub fn set_discovered_name(&self, id: &DeviceId, name: &str) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE devices SET name = ?2, named_by = 'network'
             WHERE id = ?1 AND named_by = 'guess'",
            params![id.as_str(), name],
        )?;
        Ok(n > 0)
    }

    // ---- invites ----------------------------------------------------------

    pub fn create_invite(&self, invite: &Invite) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO invites
               (token, app_slug, label, expires_at, pin_to_first, auto_approve,
                pinned_device, redemptions, created_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                invite.token.as_str(),
                invite.app_slug,
                invite.label,
                invite.policy.expires_at,
                invite.policy.pin_to_first_device,
                invite.policy.auto_approve_on_household,
                invite
                    .pinned_device
                    .as_ref()
                    .map(|d| d.as_str().to_string()),
                invite.redemptions,
                invite.created_at,
                invite.revoked_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_invite(&self, token: &InviteToken) -> Result<Option<Invite>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT token, app_slug, label, expires_at, pin_to_first, auto_approve,
                    pinned_device, redemptions, created_at, revoked_at
             FROM invites WHERE token = ?1",
        )?;
        stmt.query_row(params![token.as_str()], |row| Ok(row_to_invite(row)))
            .optional()?
            .transpose()
    }

    pub fn list_invites(&self, app_slug: &str) -> Result<Vec<Invite>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT token, app_slug, label, expires_at, pin_to_first, auto_approve,
                    pinned_device, redemptions, created_at, revoked_at
             FROM invites WHERE app_slug = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![app_slug], |row| Ok(row_to_invite(row)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Record a redemption: the pin, if this is the first, and the count.
    pub fn record_redemption(
        &self,
        token: &InviteToken,
        pinned: Option<&DeviceId>,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE invites
             SET redemptions = redemptions + 1,
                 pinned_device = COALESCE(pinned_device, ?2)
             WHERE token = ?1",
            params![token.as_str(), pinned.map(|d| d.as_str().to_string())],
        )?;
        Ok(())
    }

    pub fn revoke_invite(&self, token: &InviteToken, now: i64) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE invites SET revoked_at = ?2 WHERE token = ?1 AND revoked_at IS NULL",
            params![token.as_str(), now],
        )?;
        Ok(n > 0)
    }

    // ---- access log -------------------------------------------------------

    pub fn log_access(&self, event: &AccessEvent) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO access_log (at, app_slug, device_id, actor, action, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.at,
                event.app_slug,
                event.device_id,
                event.actor,
                event.action,
                event.detail,
            ],
        )?;
        Ok(())
    }

    /// Most recent first. `app_slug` of None means the whole workspace.
    pub fn recent_access(
        &self,
        app_slug: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AccessEvent>, StoreError> {
        let conn = self.lock();
        let (sql, filtered) = match app_slug {
            Some(_) => (
                "SELECT at, app_slug, device_id, actor, action, detail
                 FROM access_log WHERE app_slug = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
                true,
            ),
            None => (
                "SELECT at, app_slug, device_id, actor, action, detail
                 FROM access_log ORDER BY at DESC, id DESC LIMIT ?1",
                false,
            ),
        };

        let mut stmt = conn.prepare(sql)?;
        let to_event = |row: &rusqlite::Row<'_>| {
            Ok(AccessEvent {
                at: row.get(0)?,
                app_slug: row.get(1)?,
                device_id: row.get(2)?,
                actor: row.get(3)?,
                action: row.get(4)?,
                detail: row.get(5)?,
            })
        };

        let rows = if filtered {
            stmt.query_map(params![app_slug.unwrap_or_default(), limit], to_event)?
        } else {
            stmt.query_map(params![limit], to_event)?
        };

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn row_to_device(row: &rusqlite::Row<'_>) -> Result<Device, StoreError> {
    let raw: String = row.get(0)?;
    Ok(Device {
        id: DeviceId::parse(&raw).ok_or(StoreError::CorruptIdentifier {
            kind: "device id",
            value: raw,
        })?,
        name: row.get(1)?,
        status: status_from_str(&row.get::<_, String>(2)?),
        user_agent: row.get(3)?,
        fingerprint: row.get(4)?,
        first_seen: row.get(5)?,
        last_seen: row.get(6)?,
        named_by: named_by_from_str(&row.get::<_, String>(7).unwrap_or_default()),
    })
}

fn named_by_str(source: NamedBy) -> &'static str {
    match source {
        NamedBy::Guess => "guess",
        NamedBy::Network => "network",
        NamedBy::Owner => "owner",
    }
}

/// Anything unrecognised is treated as a guess, which is the weakest source -
/// so a corrupt value can only ever be improved on, never entrench itself.
fn named_by_from_str(raw: &str) -> NamedBy {
    match raw {
        "network" => NamedBy::Network,
        "owner" => NamedBy::Owner,
        _ => NamedBy::Guess,
    }
}

fn row_to_invite(row: &rusqlite::Row<'_>) -> Result<Invite, StoreError> {
    let raw: String = row.get(0)?;
    let pinned: Option<String> = row.get(6)?;

    Ok(Invite {
        token: InviteToken::parse(&raw).ok_or(StoreError::CorruptIdentifier {
            kind: "invite token",
            value: raw,
        })?,
        app_slug: row.get(1)?,
        label: row.get(2)?,
        policy: InvitePolicy {
            expires_at: row.get(3)?,
            pin_to_first_device: row.get(4)?,
            auto_approve_on_household: row.get(5)?,
        },
        pinned_device: pinned.as_deref().and_then(DeviceId::parse),
        redemptions: row.get(7)?,
        created_at: row.get(8)?,
        revoked_at: row.get(9)?,
    })
}

fn status_str(status: DeviceStatus) -> &'static str {
    match status {
        DeviceStatus::Pending => "pending",
        DeviceStatus::Approved => "approved",
        DeviceStatus::Revoked => "revoked",
        DeviceStatus::Owner => "owner",
    }
}

/// An unreadable status reads as Revoked.
///
/// The same instinct as visibility falling back to Private: a row we cannot
/// interpret must never grant more access than intended.
fn status_from_str(s: &str) -> DeviceStatus {
    match s {
        "approved" => DeviceStatus::Approved,
        "pending" => DeviceStatus::Pending,
        "owner" => DeviceStatus::Owner,
        _ => DeviceStatus::Revoked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn device() -> Device {
        Device::pending(DeviceId::generate(), "Mozilla/5.0 (iPhone)", NOW)
    }

    #[test]
    fn an_invite_cannot_pin_a_device_that_does_not_exist() {
        // The foreign key is the guard: a pin naming an unknown device would
        // be a dangling reference in the one table that decides access.
        let store = Store::in_memory().expect("opens");
        let invite = Invite::new("trip", "x", InvitePolicy::default(), NOW);
        store.create_invite(&invite).expect("creates");

        let ghost = DeviceId::generate();
        assert!(store
            .record_redemption(&invite.token, Some(&ghost))
            .is_err());
    }

    #[test]
    fn a_device_round_trips() {
        let store = Store::in_memory().expect("opens");
        let d = device();
        store.upsert_device(&d).expect("inserts");

        assert_eq!(store.get_device(&d.id).expect("reads"), Some(d));
    }

    #[test]
    fn approving_and_revoking_stick() {
        let store = Store::in_memory().expect("opens");
        let d = device();
        store.upsert_device(&d).expect("inserts");

        assert!(store
            .set_device_status(&d.id, DeviceStatus::Approved)
            .expect("approves"));
        assert_eq!(
            store
                .get_device(&d.id)
                .expect("reads")
                .expect("exists")
                .status,
            DeviceStatus::Approved
        );

        assert!(store
            .set_device_status(&d.id, DeviceStatus::Revoked)
            .expect("revokes"));
        assert_eq!(
            store
                .get_device(&d.id)
                .expect("reads")
                .expect("exists")
                .status,
            DeviceStatus::Revoked
        );
    }

    #[test]
    fn an_unreadable_status_reads_as_revoked() {
        // The same instinct as visibility falling back to Private: never
        // resolve an unknown value into more access.
        assert_eq!(status_from_str("superuser"), DeviceStatus::Revoked);
        assert_eq!(status_from_str(""), DeviceStatus::Revoked);
    }

    #[test]
    fn renaming_keeps_everything_else() {
        let store = Store::in_memory().expect("opens");
        let d = device();
        store.upsert_device(&d).expect("inserts");
        store.rename_device(&d.id, "Kitchen iPad").expect("renames");

        let back = store.get_device(&d.id).expect("reads").expect("exists");
        assert_eq!(back.name, "Kitchen iPad");
        assert_eq!(back.fingerprint, d.fingerprint);
    }

    #[test]
    fn an_invite_round_trips_with_its_policy() {
        let store = Store::in_memory().expect("opens");
        let invite = Invite::new(
            "trip-planner",
            "For the family",
            InvitePolicy {
                expires_at: Some(NOW + 604_800),
                pin_to_first_device: false,
                auto_approve_on_household: true,
            },
            NOW,
        );
        store.create_invite(&invite).expect("creates");

        assert_eq!(
            store.get_invite(&invite.token).expect("reads"),
            Some(invite)
        );
    }

    #[test]
    fn redemption_pins_only_the_first_device() {
        let store = Store::in_memory().expect("opens");
        let invite = Invite::new("trip", "For book club", InvitePolicy::default(), NOW);
        store.create_invite(&invite).expect("creates");

        // A device is always registered before it can pin an invite - the
        // foreign key enforces that ordering, and the redemption path follows
        // it.
        let first = device();
        let second = device();
        store.upsert_device(&first).expect("registers");
        store.upsert_device(&second).expect("registers");

        store
            .record_redemption(&invite.token, Some(&first.id))
            .expect("first");
        store
            .record_redemption(&invite.token, Some(&second.id))
            .expect("second");

        let back = store
            .get_invite(&invite.token)
            .expect("reads")
            .expect("exists");
        assert_eq!(
            back.pinned_device,
            Some(first.id),
            "COALESCE must not overwrite an existing pin"
        );
        assert_eq!(back.redemptions, 2);
    }

    #[test]
    fn revoking_is_idempotent_and_reports_the_first_time() {
        let store = Store::in_memory().expect("opens");
        let invite = Invite::new("trip", "x", InvitePolicy::default(), NOW);
        store.create_invite(&invite).expect("creates");

        assert!(store.revoke_invite(&invite.token, NOW).expect("revokes"));
        assert!(!store
            .revoke_invite(&invite.token, NOW + 1)
            .expect("already revoked"));

        let back = store
            .get_invite(&invite.token)
            .expect("reads")
            .expect("exists");
        assert_eq!(
            back.revoked_at,
            Some(NOW),
            "the first revocation time stands"
        );
    }

    #[test]
    fn invites_are_listed_per_app_newest_first() {
        let store = Store::in_memory().expect("opens");
        for (slug, label, at) in [
            ("a", "old", NOW),
            ("a", "new", NOW + 10),
            ("b", "other", NOW),
        ] {
            let mut invite = Invite::new(slug, label, InvitePolicy::default(), at);
            invite.created_at = at;
            store.create_invite(&invite).expect("creates");
        }

        let labels: Vec<_> = store
            .list_invites("a")
            .expect("lists")
            .into_iter()
            .map(|i| i.label)
            .collect();
        assert_eq!(labels, ["new", "old"]);
    }

    #[test]
    fn the_access_log_is_newest_first_and_filterable() {
        let store = Store::in_memory().expect("opens");
        for (i, (app, action)) in [("trip", "opened"), ("chores", "opened"), ("trip", "paired")]
            .iter()
            .enumerate()
        {
            store
                .log_access(&AccessEvent {
                    at: NOW + i as i64,
                    app_slug: Some((*app).to_string()),
                    device_id: None,
                    actor: "viewer".into(),
                    action: (*action).to_string(),
                    detail: None,
                })
                .expect("logs");
        }

        let all = store.recent_access(None, 10).expect("reads");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].action, "paired", "newest first");

        let trip = store.recent_access(Some("trip"), 10).expect("reads");
        assert_eq!(trip.len(), 2);
        assert!(trip.iter().all(|e| e.app_slug.as_deref() == Some("trip")));
    }

    #[test]
    fn the_log_limit_is_respected() {
        let store = Store::in_memory().expect("opens");
        for i in 0..50 {
            store
                .log_access(&AccessEvent {
                    at: NOW + i,
                    app_slug: None,
                    device_id: None,
                    actor: "system".into(),
                    action: "tick".into(),
                    detail: None,
                })
                .expect("logs");
        }
        assert_eq!(store.recent_access(None, 10).expect("reads").len(), 10);
    }
}

#[cfg(test)]
mod naming {
    use super::*;
    use crate::Store;

    fn store_with_device() -> (Store, DeviceId) {
        let store = Store::in_memory().expect("opens");
        let device = Device::pending(
            DeviceId::parse("AAAAAAAAAAAAAAAAAAAAAA").expect("valid id"),
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Safari/605.1",
            1_000,
        );
        store.upsert_device(&device).expect("stores");
        (store, device.id)
    }

    #[test]
    fn a_new_device_is_named_by_guesswork() {
        let (store, id) = store_with_device();
        let device = store.get_device(&id).expect("reads").expect("exists");

        assert_eq!(device.name, "iPhone");
        assert_eq!(device.named_by, NamedBy::Guess);
    }

    #[test]
    fn the_network_improves_on_a_guess() {
        let (store, id) = store_with_device();

        assert!(store
            .set_discovered_name(&id, "Kitchen iPad")
            .expect("sets"));

        let device = store.get_device(&id).expect("reads").expect("exists");
        assert_eq!(device.name, "Kitchen iPad");
        assert_eq!(device.named_by, NamedBy::Network);
    }

    #[test]
    fn the_network_never_overwrites_the_owner() {
        // The lookup takes seconds and lands whenever it lands. Without this,
        // an answer arriving after the owner typed a name would undo them.
        let (store, id) = store_with_device();
        store.rename_device(&id, "Upstairs iPad").expect("renames");

        assert!(!store
            .set_discovered_name(&id, "Kitchen iPad")
            .expect("does not set"));

        let device = store.get_device(&id).expect("reads").expect("exists");
        assert_eq!(device.name, "Upstairs iPad");
        assert_eq!(device.named_by, NamedBy::Owner);
    }

    #[test]
    fn the_owner_may_overwrite_the_network() {
        let (store, id) = store_with_device();
        store
            .set_discovered_name(&id, "Kitchen iPad")
            .expect("sets");
        store
            .rename_device(&id, "The one in the hall")
            .expect("renames");

        let device = store.get_device(&id).expect("reads").expect("exists");
        assert_eq!(device.name, "The one in the hall");
        assert_eq!(device.named_by, NamedBy::Owner);
    }

    #[test]
    fn seeing_a_device_again_does_not_reset_its_name() {
        // A viewer refreshing the wait page upserts the device on every hit.
        let (store, id) = store_with_device();
        store.rename_device(&id, "Upstairs iPad").expect("renames");

        let mut seen_again = store.get_device(&id).expect("reads").expect("exists");
        seen_again.name = "iPhone".into();
        seen_again.named_by = NamedBy::Guess;
        seen_again.last_seen = 2_000;
        store.upsert_device(&seen_again).expect("upserts");

        let device = store.get_device(&id).expect("reads").expect("exists");
        assert_eq!(device.name, "Upstairs iPad", "the owner's name survived");
        assert_eq!(device.named_by, NamedBy::Owner);
        assert_eq!(device.last_seen, 2_000, "but it was still seen");
    }
}
