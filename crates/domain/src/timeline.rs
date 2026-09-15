//! Observed network/device change timeline (research document §5.4, §7.3).
//!
//! Every entry records only what was observed: time, event kind, and a short, non-sensitive
//! detail string. Sampling can only see the changes it actually witnesses, so the model never
//! claims to have captured every radio handover or disconnect.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// One observed transition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineEvent {
    pub at: SystemTime,
    pub kind: TimelineEventKind,
    /// Short human-readable detail; closed-vocabulary text only (no subscriber data).
    pub detail: String,
}

/// Closed set of observed event kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineEventKind {
    /// A different SIM identity was observed (ICCID fingerprint changed).
    SimChanged,
    /// Cellular registration state changed (e.g. 已注册 → 搜索中).
    RegistrationChanged,
    /// The observed serving cell changed.
    CellChanged,
    /// The module stopped being detected (USB removal / unplug).
    DeviceRemoved,
    /// The module reappeared (re-enumeration).
    DeviceArrived,
    /// The bound adapter's link state changed.
    AdapterLinkChanged,
    /// The bound DNS probe changed verdict.
    DnsChanged,
}

/// Bounded timeline ring held by the application state.
///
/// Capacity is intentionally small: this is an operator aid, not a logging system, and the
/// entries must stay exportable in volume.
pub const TIMELINE_CAPACITY: usize = 64;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    events: Vec<TimelineEvent>,
}

impl Timeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push the newest event, dropping the oldest when the ring is full.
    pub fn push(&mut self, event: TimelineEvent) {
        if self.events.len() == TIMELINE_CAPACITY {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    /// Events oldest-first.
    #[must_use]
    pub fn events(&self) -> &[TimelineEvent] {
        &self.events
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_is_bounded_and_keeps_the_newest_events() {
        let mut timeline = Timeline::new();
        for index in 0..(TIMELINE_CAPACITY + 5) {
            timeline.push(TimelineEvent {
                at: SystemTime::UNIX_EPOCH,
                kind: TimelineEventKind::CellChanged,
                detail: format!("cell {index}"),
            });
        }
        assert_eq!(timeline.events().len(), TIMELINE_CAPACITY);
        let newest = format!("cell {}", TIMELINE_CAPACITY + 4);
        assert_eq!(
            timeline.events().last().map(|event| event.detail.as_str()),
            Some(newest.as_str())
        );
        assert_eq!(
            timeline.events().first().map(|event| event.detail.as_str()),
            Some("cell 5")
        );
    }
}
