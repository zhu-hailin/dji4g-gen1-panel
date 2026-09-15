//! Deterministic, hardware-free snapshots used only by debug builds and UI review.

use std::sync::Arc;
use std::time::SystemTime;

use dji4g_application::{ControllerSnapshot, ReducerState};
use dji4g_domain::{
    Availability, BoundDnsStatus, BoundPublicStatus, Freshness, HotspotStatus, LimitedReason,
    UnavailableReason,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DemoScenario {
    Available,
    Limited,
    Unavailable,
    Absent,
    Detecting,
}

impl DemoScenario {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "available" => Some(Self::Available),
            "limited" => Some(Self::Limited),
            "unavailable" => Some(Self::Unavailable),
            "absent" => Some(Self::Absent),
            "detecting" => Some(Self::Detecting),
            _ => None,
        }
    }
}

#[must_use]
pub fn demo_snapshot(scenario: DemoScenario, now: SystemTime) -> ControllerSnapshot {
    let base = match scenario {
        DemoScenario::Detecting => ReducerState::new(now).snapshot(),
        DemoScenario::Absent => {
            let mut state = ReducerState::new(now).snapshot();
            let mut app = (*state.app).clone();
            app.availability = Availability::NotDetected;
            app.freshness = Freshness::Fresh;
            state.app = Arc::new(app);
            state
        }
        DemoScenario::Available | DemoScenario::Limited | DemoScenario::Unavailable => {
            ReducerState::test_ready(now).snapshot()
        }
    };
    if matches!(
        scenario,
        DemoScenario::Available | DemoScenario::Detecting | DemoScenario::Absent
    ) {
        return base;
    }

    let mut app = (*base.app).clone();
    match scenario {
        DemoScenario::Limited => {
            app.availability = Availability::Limited(LimitedReason::DnsFailure);
            app.hotspot = HotspotStatus::On { clients: Some(2) };
            if let Some(network) = app.network.as_mut() {
                network.bound_dns = BoundDnsStatus::Failed;
                network.bound_public = BoundPublicStatus::Succeeded;
            }
        }
        DemoScenario::Unavailable => {
            app.availability = Availability::Unavailable(UnavailableReason::NoBoundReachability);
            app.hotspot = HotspotStatus::Off;
        }
        DemoScenario::Available | DemoScenario::Detecting | DemoScenario::Absent => {}
    }
    ControllerSnapshot {
        app: Arc::new(app),
        ..base
    }
}
