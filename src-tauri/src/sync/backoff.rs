//! Reconnect timing. docs/03 §5, docs/06 Phase 5.
//!
//! *Jittered backoff on reconnect: 1s → 2 → 4 → … → 300s. Never a tight retry loop.*
//!
//! The jitter is not decoration. Every account in the app, and on a bad day every copy of the
//! app behind one office NAT, fails at the same instant when the network drops — and then
//! retries at the same instant, and again at the same doubled instant. That is a connection
//! storm, and the 12-hour soak in the exit gate exists to catch exactly it. Spreading each
//! delay across a random band turns a synchronised thundering herd into a smear.
//!
//! Pure and deterministic given a random source, so the whole policy is testable without
//! waiting 300 seconds for anything.

use std::time::Duration;

/// First delay after a failure.
const BASE: Duration = Duration::from_secs(1);

/// Ceiling. docs/03 §5 fixes this at 300s — past that a user has long since noticed, and a
/// longer wait only makes the app look dead when the network comes back.
const CAP: Duration = Duration::from_secs(300);

/// How much of the delay is randomised, as a fraction.
///
/// Full jitter (0..=delay) is the usual recommendation, but it makes the *first* retry often
/// near-instant, which reads as a tight loop in a log and wastes a connection when the server
/// is still down. ±25% around the nominal delay keeps the shape of the curve while still
/// breaking synchronisation.
const JITTER: f64 = 0.25;

#[derive(Debug, Clone)]
pub struct Backoff {
    attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    pub const fn new() -> Self {
        Self { attempts: 0 }
    }

    /// The undithered delay for the current attempt count.
    ///
    /// Saturating rather than shifting: `1u64 << 64` is undefined, and an account that has
    /// been failing for a week will get there.
    fn nominal(&self) -> Duration {
        let doubled = BASE
            .as_secs()
            .checked_shl(self.attempts)
            .unwrap_or(u64::MAX);

        Duration::from_secs(doubled).min(CAP)
    }

    /// The next delay to wait, with jitter applied, and advances the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.jittered(rand::random::<f64>());
        self.attempts = self.attempts.saturating_add(1);
        delay
    }

    /// The jitter calculation, with the random draw passed in so it can be tested.
    ///
    /// `roll` is in `0.0..1.0`.
    fn jittered(&self, roll: f64) -> Duration {
        let nominal = self.nominal().as_secs_f64();
        let spread = nominal * JITTER;

        // roll 0.0 → nominal - spread, roll 1.0 → nominal + spread.
        let seconds = nominal - spread + (roll.clamp(0.0, 1.0) * spread * 2.0);

        // Never below a quarter-second, however the arithmetic lands: a delay of zero is a
        // tight retry loop by another name, which the spec rules out explicitly.
        Duration::from_secs_f64(seconds.max(0.25))
    }

    /// Called after a successful connection. The next failure starts from one second again.
    pub fn reset(&mut self) {
        self.attempts = 0;
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether this account has been failing long enough that the UI should stop saying
    /// "connecting" and start saying what is wrong. docs/06 Phase 5 §9 — *a retry-at time,
    /// not a spinner that never ends.*
    pub fn should_surface_error(&self) -> bool {
        self.attempts >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_doubles_from_one_second_and_stops_at_five_minutes() {
        let mut backoff = Backoff::new();
        let mut seen = Vec::new();

        for _ in 0..12 {
            seen.push(backoff.nominal().as_secs());
            backoff.attempts += 1;
        }

        assert_eq!(seen, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
    }

    #[test]
    fn a_very_long_outage_does_not_overflow_the_shift() {
        // `1u64 << 64` is undefined behaviour in C and a panic in debug Rust. An account that
        // has been failing for a week gets there, and it must simply sit at the cap.
        let backoff = Backoff { attempts: 64 };
        assert_eq!(backoff.nominal(), CAP);

        let backoff = Backoff { attempts: u32::MAX };
        assert_eq!(backoff.nominal(), CAP);
    }

    #[test]
    fn jitter_spreads_around_the_nominal_delay_without_changing_its_shape() {
        let backoff = Backoff { attempts: 3 }; // nominal 8s

        let lowest = backoff.jittered(0.0).as_secs_f64();
        let middle = backoff.jittered(0.5).as_secs_f64();
        let highest = backoff.jittered(1.0).as_secs_f64();

        assert!((lowest - 6.0).abs() < 0.001, "{lowest}");
        assert!((middle - 8.0).abs() < 0.001, "{middle}");
        assert!((highest - 10.0).abs() < 0.001, "{highest}");
    }

    #[test]
    fn no_delay_is_ever_short_enough_to_be_a_tight_loop() {
        // The one property that matters more than the curve: *never a tight retry loop*.
        // A zero-length delay against a server that is refusing connections is a denial of
        // service pointed at someone else's mail host.
        for attempts in 0..40 {
            let backoff = Backoff { attempts };

            for roll in [0.0, 0.001, 0.5, 0.999, 1.0] {
                let delay = backoff.jittered(roll);
                assert!(
                    delay >= Duration::from_millis(250),
                    "attempt {attempts} roll {roll} gave {delay:?}"
                );
            }
        }
    }

    #[test]
    fn a_random_draw_outside_the_unit_range_cannot_produce_a_negative_delay() {
        let backoff = Backoff { attempts: 5 };

        assert!(backoff.jittered(-5.0) > Duration::ZERO);
        assert!(backoff.jittered(f64::NAN) > Duration::ZERO);
        assert!(backoff.jittered(1e9) <= Duration::from_secs(64));
    }

    #[test]
    fn success_puts_the_next_failure_back_at_one_second() {
        let mut backoff = Backoff::new();

        for _ in 0..8 {
            let _ = backoff.next_delay();
        }
        assert!(backoff.nominal() > BASE);

        backoff.reset();
        assert_eq!(backoff.nominal(), BASE);
        assert_eq!(backoff.attempts(), 0);
    }

    #[test]
    fn a_single_blip_does_not_put_an_error_in_front_of_the_user() {
        // One failed reconnect is a lid closing or a train tunnel. Telling the user their
        // account is broken every time their laptop sleeps is how a mail client earns the
        // reputation of crying wolf.
        let mut backoff = Backoff::new();
        assert!(!backoff.should_surface_error());

        let _ = backoff.next_delay();
        assert!(!backoff.should_surface_error());

        let _ = backoff.next_delay();
        assert!(
            backoff.should_surface_error(),
            "two failures is a real problem"
        );
    }

    #[test]
    fn two_accounts_failing_together_do_not_retry_together() {
        // The connection storm the 12-hour soak exists to catch. Without jitter these two
        // sequences would be identical.
        let mut left = Backoff::new();
        let mut right = Backoff::new();

        let a: Vec<_> = (0..6).map(|_| left.next_delay()).collect();
        let b: Vec<_> = (0..6).map(|_| right.next_delay()).collect();

        assert_ne!(
            a, b,
            "jitter must desynchronise two accounts failing at once"
        );
    }
}
