//! How long to wait before dialling the relay again.
//!
//! # Why this is not just a sleep
//!
//! Every daemon in the world is connected to the same edge, and the edge
//! restarts. If each one waits a fixed second and retries, they all come back in
//! the same second, and the thing they are all reconnecting to is the thing that
//! has just started and is least able to absorb them. The retry becomes the
//! outage. That failure has a name and it is worth naming in the code that
//! prevents it: a thundering herd.
//!
//! So delays grow, and - the half that actually does the work - they are
//! scattered. This uses **full jitter**: the delay is a uniform pick from
//! anywhere between zero and the current ceiling, rather than the ceiling with a
//! little noise added. Picking across the whole window is what turns a
//! synchronised crowd into an even trickle; adding ten per cent of noise to a
//! shared number leaves it very nearly shared.
//!
//! # Not every failure deserves a retry
//!
//! The schedule is only half of it. A daemon whose install was revoked an hour
//! ago must not be on any schedule at all - it should stop, and the window
//! should say why. That decision is not made here: the edge names a reason, and
//! [`kt_tunnel_proto::CloseReason::retry`] turns it into one of three answers.
//! This module carries out whichever it is given.

use std::time::Duration;

use kt_tunnel_proto::{Retry, RECONNECT_MAX_SECS, RECONNECT_MIN_SECS};

/// Where the next attempt may land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// How many attempts have failed since the last success. Saturates, so a
    /// daemon left dialling overnight does not wrap round to no delay at all.
    attempt: u32,
}

impl Backoff {
    pub const fn new() -> Self {
        Self { attempt: 0 }
    }

    /// A connection worked. The next failure starts from the bottom again.
    ///
    /// Called on a *established and useful* connection rather than on a
    /// successful TCP connect, because an edge that accepts and immediately
    /// closes would otherwise reset the schedule every time and turn the
    /// backoff into a tight loop.
    pub fn succeeded(&mut self) {
        self.attempt = 0;
    }

    /// How long to wait, or `None` to stop dialling entirely.
    ///
    /// The `Retry::Soon` case still waits, and still jitters. It is tempting to
    /// reconnect immediately after something as routine as a drain, but a drain
    /// is precisely the moment every daemon is disconnected at once, so it is
    /// the case that most needs scattering.
    pub fn next_delay(&mut self, retry: Retry) -> Option<Duration> {
        match retry {
            Retry::Never => None,
            Retry::Soon => {
                // Deliberately does not advance the attempt count: a drain is
                // not evidence that anything is wrong, and letting routine
                // restarts push the schedule upwards would leave a daemon
                // waiting a minute after a deploy it handled perfectly.
                Some(jitter(window(0)))
            }
            Retry::Backoff => {
                let delay = jitter(window(self.attempt));
                self.attempt = self.attempt.saturating_add(1);
                Some(delay)
            }
        }
    }

    #[cfg(test)]
    fn attempts(&self) -> u32 {
        self.attempt
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

/// The ceiling for a given attempt: doubling from the floor, capped.
///
/// Pure and separated from the random draw so that the shape of the schedule
/// can be asserted exactly, which is the half worth asserting.
fn window(attempt: u32) -> Duration {
    // 2^attempt as u64, without overflowing: past ~6 attempts the cap applies
    // anyway, so anything beyond that is the same answer by a longer route.
    let factor = 1u64.checked_shl(attempt.min(32)).unwrap_or(u64::MAX);
    let secs = u64::from(RECONNECT_MIN_SECS)
        .saturating_mul(factor)
        .min(u64::from(RECONNECT_MAX_SECS));
    Duration::from_secs(secs)
}

/// A uniform pick from `[0, ceiling]`, to millisecond resolution.
fn jitter(ceiling: Duration) -> Duration {
    let millis = ceiling.as_millis() as u64;
    if millis == 0 {
        return Duration::ZERO;
    }

    let mut bytes = [0u8; 8];
    // A failure here would mean the OS random source is gone, which is not a
    // reason to stop reconnecting - so it degrades to the full ceiling, which
    // is the unjittered behaviour and merely less polite.
    if getrandom::fill(&mut bytes).is_err() {
        return ceiling;
    }

    Duration::from_millis(u64::from_be_bytes(bytes) % (millis + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_doubles_from_the_floor() {
        assert_eq!(window(0), Duration::from_secs(1));
        assert_eq!(window(1), Duration::from_secs(2));
        assert_eq!(window(2), Duration::from_secs(4));
        assert_eq!(window(3), Duration::from_secs(8));
    }

    #[test]
    fn the_window_stops_at_the_ceiling_and_never_overflows() {
        assert_eq!(window(6), Duration::from_secs(60));
        assert_eq!(window(60), Duration::from_secs(60));
        // The case that would panic on a naive shift, reached by a daemon left
        // dialling a dead edge for a very long time.
        assert_eq!(window(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn a_permanent_refusal_yields_no_delay_at_all() {
        // Not a long wait - no wait, because there is nothing to wait for. The
        // daemon stops and the window tells the owner.
        let mut backoff = Backoff::new();
        assert_eq!(backoff.next_delay(Retry::Never), None);
    }

    #[test]
    fn repeated_failures_climb_and_then_hold() {
        let mut backoff = Backoff::new();
        for _ in 0..10 {
            let delay = backoff.next_delay(Retry::Backoff).expect("retryable");
            assert!(
                delay <= Duration::from_secs(u64::from(RECONNECT_MAX_SECS)),
                "{delay:?} is over the ceiling"
            );
        }
        assert_eq!(backoff.attempts(), 10);
    }

    #[test]
    fn a_drain_does_not_push_the_schedule_upwards() {
        // A deploy is routine and frequent. If it counted as a failure, a
        // daemon that had handled several perfectly would end up waiting a
        // minute after the next one.
        let mut backoff = Backoff::new();
        for _ in 0..5 {
            let delay = backoff.next_delay(Retry::Soon).expect("retryable");
            assert!(delay <= Duration::from_secs(1), "{delay:?} is not soon");
        }
        assert_eq!(backoff.attempts(), 0);
    }

    #[test]
    fn a_working_connection_resets_the_climb() {
        let mut backoff = Backoff::new();
        for _ in 0..5 {
            backoff.next_delay(Retry::Backoff);
        }
        assert_eq!(backoff.attempts(), 5);

        backoff.succeeded();
        assert_eq!(backoff.attempts(), 0);

        let delay = backoff.next_delay(Retry::Backoff).expect("retryable");
        assert!(delay <= Duration::from_secs(1), "back to the bottom");
    }

    #[test]
    fn jitter_spreads_across_the_whole_window_rather_than_near_its_top() {
        // The property the thundering herd depends on. Full jitter draws from
        // the entire window, so a hundred daemons released at once land spread
        // out; a scheme that added a little noise to a shared number would put
        // them all in the same neighbourhood and fail this.
        let ceiling = Duration::from_secs(60);
        let draws: Vec<Duration> = (0..200).map(|_| jitter(ceiling)).collect();

        assert!(
            draws.iter().all(|d| *d <= ceiling),
            "a draw escaped the window"
        );

        let quarter = ceiling / 4;
        let low = draws.iter().filter(|d| **d < quarter).count();
        let high = draws.iter().filter(|d| **d > ceiling - quarter).count();
        assert!(
            low > 10 && high > 10,
            "draws are bunched: {low} in the bottom quarter, {high} in the top"
        );
    }

    #[test]
    fn a_zero_window_is_not_a_division_by_zero() {
        assert_eq!(jitter(Duration::ZERO), Duration::ZERO);
    }
}
