//! The balloon decision matrix: only a verdict transition, observed while the window is hidden,
//! outside the cooldown, is worth one shell toast.  Startup is never an event.

use std::time::{Duration, SystemTime};

use dji4g_domain::Availability;
use dji4g_panel::app::AvailabilityNotifier;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;
// The notifier takes `window_visible`: hidden means false.
const HIDDEN: bool = false;
const VISIBLE: bool = true;

#[test]
fn startup_and_first_sighting_never_notify() {
    let mut notifier = AvailabilityNotifier::default();
    assert_eq!(
        notifier.consider(Availability::Detecting, HIDDEN, NOW),
        None,
        "the first sighting is not an event"
    );
}

#[test]
fn a_hidden_transition_notifies_once_and_then_deduplicates() {
    let mut notifier = AvailabilityNotifier::default();
    let _ = notifier.consider(Availability::Available, HIDDEN, NOW);

    assert_eq!(
        notifier.consider(
            Availability::Limited(dji4g_domain::LimitedReason::AtControlUnavailable),
            HIDDEN,
            NOW + Duration::from_secs(10)
        ),
        Some(Availability::Limited(
            dji4g_domain::LimitedReason::AtControlUnavailable
        ))
    );
    // The same verdict again: a repeat, never a new balloon.
    assert_eq!(
        notifier.consider(
            Availability::Limited(dji4g_domain::LimitedReason::AtControlUnavailable),
            HIDDEN,
            NOW + Duration::from_secs(20)
        ),
        None
    );
}

#[test]
fn transitions_observed_while_visible_are_not_announced_later() {
    let mut notifier = AvailabilityNotifier::default();
    let _ = notifier.consider(Availability::Available, VISIBLE, NOW);

    // The change happens on the visible window: suppressed, and the sighting is recorded.
    assert_eq!(
        notifier.consider(
            Availability::Unavailable(dji4g_domain::UnavailableReason::BoundPublicProbeFailed),
            VISIBLE,
            NOW + Duration::from_secs(10)
        ),
        None
    );
    // Reopening onto the same verdict: still not an event.
    assert_eq!(
        notifier.consider(
            Availability::Unavailable(dji4g_domain::UnavailableReason::BoundPublicProbeFailed),
            HIDDEN,
            NOW + Duration::from_secs(20)
        ),
        None
    );
}

#[test]
fn the_cooldown_suppresses_a_flapping_link() {
    let mut notifier = AvailabilityNotifier::default();
    let _ = notifier.consider(Availability::Available, HIDDEN, NOW);

    let limited = Availability::Limited(dji4g_domain::LimitedReason::DnsFailure);
    assert_eq!(
        notifier.consider(limited, HIDDEN, NOW + Duration::from_secs(30)),
        Some(limited),
        "the first transition announces"
    );
    assert_eq!(
        notifier.consider(
            Availability::Available,
            HIDDEN,
            NOW + Duration::from_secs(60)
        ),
        None,
        "inside the cooldown"
    );
    assert_eq!(
        notifier.consider(limited, HIDDEN, NOW + Duration::from_secs(90)),
        None,
        "still inside the cooldown"
    );
    // A repeat of the current verdict stays deduplicated regardless of the cooldown: the last
    // thing announced (or seen) is already Limited, so a new toast would announce nothing.
    assert_eq!(
        notifier.consider(
            limited,
            HIDDEN,
            NOW + AVAILABILITY_BALLOON_COOLDOWN + Duration::from_secs(31)
        ),
        None
    );
    // A genuinely new verdict after the cooldown announces.
    let unavailable = Availability::Unavailable(dji4g_domain::UnavailableReason::CellularRejected);
    assert_eq!(
        notifier.consider(
            unavailable,
            HIDDEN,
            NOW + AVAILABILITY_BALLOON_COOLDOWN + Duration::from_secs(32)
        ),
        Some(unavailable),
        "the cooldown has expired and the verdict is new"
    );
}

use dji4g_panel::app::AVAILABILITY_BALLOON_COOLDOWN;
