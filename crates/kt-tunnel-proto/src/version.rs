//! Which version of this protocol two peers are going to speak.
//!
//! The daemon and the edge are never the same age. One ships inside a signed
//! application bundle and updates when the person using it agrees; the other
//! redeploys on a Tuesday afternoon. So every connection settles the question
//! rather than assuming it, and the answer is per connection: an edge that
//! rolls out support for version 3 keeps speaking 2 to every daemon that has
//! not updated yet, until the day the range moves and it stops.
//!
//! Both peers advertise a *range* rather than a single number, because a peer
//! that could only say "I am version 2" would force a flag day for every
//! change. A range says what a peer can still speak as well as what it prefers.

use serde::{Deserialize, Serialize};

/// The newest version this build speaks, and the one it will pick given a
/// choice.
///
/// Bump on any change to the wire, semver-major on this crate for a breaking
/// one, and the cloud repo moves its pinned tag deliberately
/// (repo-architecture.md section 6). Raising this is not on its own a
/// compatibility break; raising [`MIN_PROTOCOL_VERSION`] is.
pub const PROTOCOL_VERSION: u16 = 1;

/// The oldest version this build will still accept.
///
/// Raising it drops every peer older than the new floor, which for the daemon
/// half means every install that has not updated. Treat it as a deprecation
/// with a date, not a cleanup.
pub const MIN_PROTOCOL_VERSION: u16 = 1;

// A floor above the ceiling would leave this build unable to speak to a copy of
// itself, which is a typo rather than a decision, so it stops the build.
const _: () = assert!(MIN_PROTOCOL_VERSION <= PROTOCOL_VERSION);

/// What a peer says it can speak, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRange {
    pub min: u16,
    pub max: u16,
}

impl VersionRange {
    /// What this build supports.
    pub const fn supported() -> Self {
        Self {
            min: MIN_PROTOCOL_VERSION,
            max: PROTOCOL_VERSION,
        }
    }

    pub const fn new(min: u16, max: u16) -> Self {
        Self { min, max }
    }

    /// The highest version both peers can speak.
    ///
    /// Highest rather than lowest, so that two current peers get current
    /// behaviour and the old path is taken only when something old is actually
    /// present.
    ///
    /// `self` is us and `peer` is them, and the errors are told apart on that
    /// basis. It matters because the two have different remedies and only one
    /// of them is the user's to apply: a daemon that is too old is fixed by
    /// updating the app, and an edge that is too old is fixed by us, with the
    /// daemon's only correct move being to wait.
    pub fn negotiate(self, peer: VersionRange) -> Result<u16, VersionError> {
        if self.min > self.max {
            return Err(VersionError::Malformed { range: self });
        }
        if peer.min > peer.max {
            return Err(VersionError::Malformed { range: peer });
        }

        let floor = self.min.max(peer.min);
        let ceiling = self.max.min(peer.max);

        if floor <= ceiling {
            return Ok(ceiling);
        }
        if peer.max < self.min {
            Err(VersionError::PeerTooOld { ours: self, peer })
        } else {
            Err(VersionError::PeerTooNew { ours: self, peer })
        }
    }

    /// Whether a single version falls inside this range.
    pub const fn accepts(self, version: u16) -> bool {
        version >= self.min && version <= self.max
    }
}

impl Default for VersionRange {
    fn default() -> Self {
        Self::supported()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    #[error(
        "the other end speaks tunnel protocol {}-{}, and the oldest we accept is {}",
        peer.min, peer.max, ours.min
    )]
    PeerTooOld {
        ours: VersionRange,
        peer: VersionRange,
    },
    #[error(
        "the other end needs tunnel protocol {} at least, and the newest we speak is {}",
        peer.min, ours.max
    )]
    PeerTooNew {
        ours: VersionRange,
        peer: VersionRange,
    },
    /// A range whose floor is above its ceiling. Nobody can speak it, so it is
    /// a broken peer rather than an old one, and worth saying so separately.
    #[error("a version range of {}-{} is not a range", range.min, range.max)]
    Malformed { range: VersionRange },
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn r(min: u16, max: u16) -> VersionRange {
        VersionRange::new(min, max)
    }

    #[test]
    fn identical_ranges_pick_the_top() {
        assert_eq!(r(1, 3).negotiate(r(1, 3)), Ok(3));
    }

    #[test]
    fn overlapping_ranges_pick_the_highest_in_common() {
        assert_eq!(r(1, 5).negotiate(r(3, 9)), Ok(5));
        assert_eq!(r(3, 9).negotiate(r(1, 5)), Ok(5));
    }

    #[test]
    fn a_single_shared_version_is_enough() {
        assert_eq!(r(1, 4).negotiate(r(4, 8)), Ok(4));
    }

    #[test]
    fn the_error_names_which_side_is_behind() {
        // Their ceiling is below our floor: they are the old one.
        assert_eq!(
            r(5, 9).negotiate(r(1, 2)),
            Err(VersionError::PeerTooOld {
                ours: r(5, 9),
                peer: r(1, 2)
            })
        );
        // Their floor is above our ceiling: we are.
        assert_eq!(
            r(1, 2).negotiate(r(5, 9)),
            Err(VersionError::PeerTooNew {
                ours: r(1, 2),
                peer: r(5, 9)
            })
        );
    }

    #[test]
    fn an_inverted_range_is_rejected_from_either_side() {
        assert!(matches!(
            r(9, 1).negotiate(r(1, 3)),
            Err(VersionError::Malformed { range }) if range == r(9, 1)
        ));
        assert!(matches!(
            r(1, 3).negotiate(r(9, 1)),
            Err(VersionError::Malformed { range }) if range == r(9, 1)
        ));
    }

    #[test]
    fn this_build_agrees_with_itself() {
        let ours = VersionRange::supported();
        assert_eq!(ours.negotiate(ours), Ok(PROTOCOL_VERSION));
        assert!(ours.accepts(PROTOCOL_VERSION));
        assert!(ours.accepts(MIN_PROTOCOL_VERSION));
    }

    #[test]
    fn accepts_is_inclusive_and_bounded() {
        assert!(r(2, 4).accepts(2));
        assert!(r(2, 4).accepts(4));
        assert!(!r(2, 4).accepts(1));
        assert!(!r(2, 4).accepts(5));
    }
}
