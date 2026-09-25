use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{
    AdapterBinding, AdapterState, AtControlAvailability, BoundDnsStatus, BoundEvidence,
    BoundPublicStatus, CellularBlock, DefaultRouteOwner, DeviceEpoch, DevicePresence, Evidence,
    EvidenceSource, GlobalConnectivity, ProtocolCoverage, StableDeviceIdentity,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Availability {
    Detecting,
    Available,
    Limited(LimitedReason),
    Unavailable(UnavailableReason),
    NotDetected,
    UnsupportedDevice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LimitedReason {
    DnsFailure,
    SingleProtocolFamily,
    CompetingDefaultRoute,
    AtControlUnavailable,
    IncompleteEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UnavailableReason {
    CellularRejected,
    NoUsableAddressOrRoute,
    BoundPublicProbeFailed,
    NoBoundReachability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClassificationPhase {
    Startup,
    RecentInsertion,
    Reenumerating,
    PostWriteVerification,
    Stable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationInput {
    pub current_epoch: DeviceEpoch,
    pub phase: ClassificationPhase,
    pub device_presence: Option<Evidence<DevicePresence>>,
    pub target_identity: Option<Evidence<StableDeviceIdentity>>,
    pub cellular_block: Option<Evidence<CellularBlock>>,
    pub adapter_binding: Option<Evidence<AdapterBinding>>,
    pub adapter: Option<Evidence<AdapterState>>,
    pub bound_public: Option<Evidence<BoundEvidence<BoundPublicStatus>>>,
    pub bound_dns: Option<Evidence<BoundEvidence<BoundDnsStatus>>>,
    pub protocol_coverage: Option<Evidence<BoundEvidence<ProtocolCoverage>>>,
    pub at_control: Option<Evidence<AtControlAvailability>>,
    pub system_default_route: Option<Evidence<DefaultRouteOwner>>,
    pub global_connectivity: Option<Evidence<GlobalConnectivity>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityDecision {
    pub status: Availability,
}

#[must_use]
pub fn classify(input: &ClassificationInput, now: SystemTime) -> AvailabilityDecision {
    AvailabilityDecision {
        status: classify_status(input, now),
    }
}

fn classify_status(input: &ClassificationInput, now: SystemTime) -> Availability {
    let Some(device_presence) = input.device_presence.as_ref().filter(|evidence| {
        evidence.source == EvidenceSource::Pnp && evidence.is_fresh_for(input.current_epoch, now)
    }) else {
        return Availability::Detecting;
    };

    match &device_presence.value {
        DevicePresence::Supported(profile) if *profile == crate::DJI_GEN1 => {}
        DevicePresence::NotDetected => return Availability::NotDetected,
        DevicePresence::Unsupported { .. } => return Availability::UnsupportedDevice,
        DevicePresence::PermissionDenied => return Availability::Detecting,
        DevicePresence::Supported(_) => return Availability::UnsupportedDevice,
    }

    if input.phase != ClassificationPhase::Stable {
        return Availability::Detecting;
    }

    let Some(target) = input
        .target_identity
        .as_ref()
        .filter(|evidence| {
            evidence.source == EvidenceSource::Pnp
                && evidence.is_fresh_for(input.current_epoch, now)
                && evidence.value.is_supported()
        })
        .map(|evidence| &evidence.value)
    else {
        return Availability::Detecting;
    };

    if contains_stale_non_device_evidence(input, now) {
        return Availability::Detecting;
    }

    let Some(adapter_binding) = input
        .adapter_binding
        .as_ref()
        .filter(|evidence| {
            evidence.source == EvidenceSource::WindowsAdapter
                && evidence.is_fresh_for(input.current_epoch, now)
                && evidence.value.is_valid_for(target)
        })
        .map(|evidence| &evidence.value)
    else {
        return incomplete_evidence(input, now);
    };

    if input.cellular_block.is_some() {
        return Availability::Unavailable(UnavailableReason::CellularRejected);
    }

    match input
        .adapter
        .as_ref()
        .filter(|evidence| evidence.source == EvidenceSource::WindowsAdapter)
        .map(|evidence| evidence.value)
    {
        Some(AdapterState::UsableAddressAndRoute) => {}
        Some(AdapterState::NoUsableAddressOrRoute) => {
            return Availability::Unavailable(UnavailableReason::NoUsableAddressOrRoute);
        }
        None => return incomplete_evidence(input, now),
    }

    match valid_bound_value(
        input.bound_public.as_ref(),
        EvidenceSource::BoundPublicProbe,
        adapter_binding,
        input.current_epoch,
        now,
    ) {
        Some(BoundPublicStatus::Succeeded) => classify_bound_success(input, adapter_binding, now),
        Some(BoundPublicStatus::Failed { consecutive_cycles }) if *consecutive_cycles >= 2 => {
            Availability::Unavailable(UnavailableReason::BoundPublicProbeFailed)
        }
        Some(BoundPublicStatus::Failed { .. }) | Some(BoundPublicStatus::Incomplete) => {
            incomplete_evidence(input, now)
        }
        None if global_is_online(input, now) => {
            Availability::Unavailable(UnavailableReason::NoBoundReachability)
        }
        None => incomplete_evidence(input, now),
    }
}

fn classify_bound_success(
    input: &ClassificationInput,
    adapter_binding: &AdapterBinding,
    now: SystemTime,
) -> Availability {
    match valid_bound_value(
        input.bound_dns.as_ref(),
        EvidenceSource::BoundDnsProbe,
        adapter_binding,
        input.current_epoch,
        now,
    ) {
        Some(BoundDnsStatus::Failed) => {
            return Availability::Limited(LimitedReason::DnsFailure);
        }
        Some(BoundDnsStatus::Incomplete) | None => {
            return incomplete_evidence(input, now);
        }
        Some(BoundDnsStatus::Succeeded) => {}
    }

    match valid_bound_value(
        input.protocol_coverage.as_ref(),
        EvidenceSource::BoundPublicProbe,
        adapter_binding,
        input.current_epoch,
        now,
    ) {
        Some(ProtocolCoverage::AllRequiredFamilies) => {}
        Some(ProtocolCoverage::SingleFamilyOnly) => {
            return Availability::Limited(LimitedReason::SingleProtocolFamily);
        }
        None => return incomplete_evidence(input, now),
    }

    match input
        .at_control
        .as_ref()
        .filter(|evidence| evidence.source == EvidenceSource::AtControl)
        .map(|evidence| evidence.value)
    {
        Some(AtControlAvailability::Available) => {}
        Some(AtControlAvailability::Unavailable) => {
            return Availability::Limited(LimitedReason::AtControlUnavailable);
        }
        None => return incomplete_evidence(input, now),
    }

    Availability::Available
}

fn valid_bound_value<'a, T>(
    evidence: Option<&'a Evidence<BoundEvidence<T>>>,
    source: EvidenceSource,
    adapter_binding: &AdapterBinding,
    epoch: DeviceEpoch,
    now: SystemTime,
) -> Option<&'a T> {
    evidence
        .filter(|evidence| {
            evidence.source == source
                && evidence.is_fresh_for(epoch, now)
                && evidence.value.binding == *adapter_binding
        })
        .map(|evidence| &evidence.value.value)
}

fn global_is_online(input: &ClassificationInput, now: SystemTime) -> bool {
    input.global_connectivity.as_ref().is_some_and(|evidence| {
        evidence.source == EvidenceSource::GlobalConnectivity
            && evidence.is_fresh_for(input.current_epoch, now)
            && evidence.value == GlobalConnectivity::Online
    })
}

/// `true` only when AT control is *definitely* known unavailable for the current epoch.
///
/// A missing, stale, or wrongly-sourced observation yields `false`, so this can never invent an
/// explanation out of absent evidence; it only promotes an explanation the platform already proved
/// (for example `select_at_port` failing with `pnp:no_safe_at_port` because the module's serial
/// interfaces are in an error state).
fn at_control_known_unavailable(input: &ClassificationInput, now: SystemTime) -> bool {
    input.at_control.as_ref().is_some_and(|evidence| {
        evidence.source == EvidenceSource::AtControl
            && evidence.is_fresh_for(input.current_epoch, now)
            && evidence.value == AtControlAvailability::Unavailable
    })
}

/// The honest "the full data path could not be proven" verdict for a device that *is* recognized.
///
/// Every call site is reached only after `DevicePresence::Supported(DJI_GEN1)`, a fresh supported
/// target identity, and `ClassificationPhase::Stable`, so the device is known to be present.  When
/// the AT port is additionally known to be unavailable, that concrete fact is reported instead of
/// the generic incomplete-evidence wording, which would otherwise hide an actionable hardware/driver
/// fault behind 「现有证据不足」.
///
/// This is fail-closed by construction: it only ever substitutes one `Limited` reason for another,
/// so it can never produce `Available`, and every definite `Unavailable` verdict
/// (`CellularRejected`, `NoUsableAddressOrRoute`, `BoundPublicProbeFailed`, `NoBoundReachability`)
/// is decided before these branches are reached and is therefore never masked.
fn incomplete_evidence(input: &ClassificationInput, now: SystemTime) -> Availability {
    if at_control_known_unavailable(input, now) {
        Availability::Limited(LimitedReason::AtControlUnavailable)
    } else {
        Availability::Limited(LimitedReason::IncompleteEvidence)
    }
}

fn contains_stale_non_device_evidence(input: &ClassificationInput, now: SystemTime) -> bool {
    let epoch = input.current_epoch;

    input
        .target_identity
        .as_ref()
        .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .cellular_block
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .adapter_binding
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .adapter
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .bound_public
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .bound_dns
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .protocol_coverage
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .at_control
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
        || input
            .system_default_route
            .as_ref()
            .is_some_and(|evidence| !evidence.is_fresh_for(epoch, now))
}
