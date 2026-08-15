//! kt-store's per-app stores, behind the trait kt-server asks for.
//!
//! The whole module is translation. kt-server deliberately knows nothing about
//! SQLite and kt-store deliberately knows nothing about requests, so somebody
//! has to hold both, and the daemon is where every other pair of those meet.

use std::path::PathBuf;

use kt_server::storage::{StorageEntry, StorageFault, StorageSource};
use kt_store::{Scope, Storage, StoreError};

pub struct Apps {
    storage: Storage,
}

impl Apps {
    pub fn new(root: PathBuf) -> Self {
        Self {
            storage: Storage::new(root),
        }
    }
}

/// `None` is the app's shared store; `Some(id)` is one device's own.
///
/// The wire form is a bare device id and the stored form is prefixed, and this
/// is the only place that conversion happens - see `Scope` for why the prefix
/// exists.
fn scope(device: Option<&str>) -> Scope {
    match device {
        Some(id) => Scope::Device(id.to_string()),
        None => Scope::Shared,
    }
}

/// Quota is the one failure an app can do something about, so it survives as
/// itself. Everything else is this machine's problem and is flattened.
fn fault(error: StoreError) -> StorageFault {
    match error {
        StoreError::Quota { .. } => StorageFault::Quota,
        other => StorageFault::Failed(other.to_string()),
    }
}

impl StorageSource for Apps {
    fn get(
        &self,
        slug: &str,
        device: Option<&str>,
        key: &str,
    ) -> Result<Option<String>, StorageFault> {
        self.storage
            .app(slug)
            .map_err(fault)?
            .get(&scope(device), key)
            .map_err(fault)
    }

    fn set(
        &self,
        slug: &str,
        device: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageFault> {
        self.storage
            .app(slug)
            .map_err(fault)?
            .set(&scope(device), key, value)
            .map_err(fault)
    }

    fn delete(&self, slug: &str, device: Option<&str>, key: &str) -> Result<bool, StorageFault> {
        self.storage
            .app(slug)
            .map_err(fault)?
            .delete(&scope(device), key)
            .map_err(fault)
    }

    fn list(
        &self,
        slug: &str,
        device: Option<&str>,
        prefix: &str,
    ) -> Result<Vec<StorageEntry>, StorageFault> {
        Ok(self
            .storage
            .app(slug)
            .map_err(fault)?
            .list(&scope(device), prefix)
            .map_err(fault)?
            .into_iter()
            .map(|entry| StorageEntry {
                key: entry.key,
                value: entry.value,
                updated_at: entry.updated_at,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quota_survives_translation_and_everything_else_is_flattened() {
        // The app can act on being full and can act on nothing else, so that is
        // the one failure worth keeping its shape.
        assert!(matches!(
            fault(StoreError::Quota { limit: 1 }),
            StorageFault::Quota
        ));
        assert!(matches!(
            fault(StoreError::BadSlug("../x".into())),
            StorageFault::Failed(_)
        ));
    }

    #[test]
    fn a_bare_device_id_becomes_a_device_scope() {
        assert_eq!(scope(None), Scope::Shared);
        assert_eq!(scope(Some("dev-1")), Scope::Device("dev-1".to_string()));
    }
}
