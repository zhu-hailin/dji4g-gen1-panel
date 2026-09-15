use std::time::{Duration, SystemTime};

use dji4g_domain::{
    AdapterBinding, AdapterState, AtControlAvailability, Availability, BoundDnsStatus,
    BoundEvidence, BoundPublicStatus, CellularBlock, ClassificationInput, ClassificationPhase,
    DefaultRouteOwner, DeviceEpoch, DevicePresence, DeviceProfile, Evidence, EvidenceSource,
    GlobalConnectivity, LimitedReason, ProtocolCoverage, StableDeviceIdentity, UnavailableReason,
    classify,
};

const EPOCH: DeviceEpoch = DeviceEpoch(7);

fn evidence<T>(now: SystemTime, source: EvidenceSource, value: T) -> Evidence<T> {
    Evidence {
        epoch: EPOCH,
        observed_at: now - Duration::from_secs(1),
        ttl: Duration::from_secs(30),
        source,
        value,
    }
}

fn target_identity() -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: "{f5af1065-56f4-42f4-a3bd-09caa15a31aa}".to_owned(),
        device_instance_id: "USB\\VID_2CA3&PID_4006\\REDACTED".to_owned(),
        vid: 0x2CA3,
        pid: 0x4006,
    }
}

fn adapter_binding() -> AdapterBinding {
    AdapterBinding {
        target: target_identity(),
        adapter_id: "{dji-rndis-adapter-guid}".to_owned(),
    }
}

fn bound<T>(value: T) -> BoundEvidence<T> {
    BoundEvidence {
        binding: adapter_binding(),
        value,
    }
}

fn supported(now: SystemTime) -> ClassificationInput {
    ClassificationInput {
        current_epoch: EPOCH,
        phase: ClassificationPhase::Stable,
        device_presence: Some(evidence(
            now,
            EvidenceSource::Pnp,
            DevicePresence::Supported(DeviceProfile::DJI_GEN1),
        )),
        target_identity: Some(evidence(now, EvidenceSource::Pnp, target_identity())),
        cellular_block: None,
        adapter_binding: Some(evidence(
            now,
            EvidenceSource::WindowsAdapter,
            adapter_binding(),
        )),
        adapter: Some(evidence(
            now,
            EvidenceSource::WindowsAdapter,
            AdapterState::UsableAddressAndRoute,
        )),
        bound_public: Some(evidence(
            now,
            EvidenceSource::BoundPublicProbe,
            bound(BoundPublicStatus::Succeeded),
        )),
        bound_dns: Some(evidence(
            now,
            EvidenceSource::BoundDnsProbe,
            bound(BoundDnsStatus::Succeeded),
        )),
        protocol_coverage: Some(evidence(
            now,
            EvidenceSource::BoundPublicProbe,
            bound(ProtocolCoverage::AllRequiredFamilies),
        )),
        at_control: Some(evidence(
            now,
            EvidenceSource::AtControl,
            AtControlAvailability::Available,
        )),
        system_default_route: Some(evidence(
            now,
            EvidenceSource::GlobalRoute,
            DefaultRouteOwner::TargetAdapter,
        )),
        global_connectivity: None,
    }
}

fn bound_public_and_dns_ok(now: SystemTime) -> ClassificationInput {
    supported(now)
}

fn public_ok_dns_failed(now: SystemTime) -> ClassificationInput {
    let mut input = supported(now);
    input.bound_dns = Some(evidence(
        now,
        EvidenceSource::BoundDnsProbe,
        bound(BoundDnsStatus::Failed),
    ));
    input
}

fn old_epoch_success_after_replug(now: SystemTime) -> ClassificationInput {
    let mut input = supported(now);
    input.current_epoch = DeviceEpoch(EPOCH.0 + 1);
    input
}

fn global_phone_only(now: SystemTime) -> ClassificationInput {
    let mut input = supported(now);
    input.bound_public = None;
    input.bound_dns = None;
    input.global_connectivity = Some(evidence(
        now,
        EvidenceSource::GlobalConnectivity,
        GlobalConnectivity::Online,
    ));
    input
}

#[test]
fn only_pid_4006_matches_the_first_generation_profile() {
    assert!(DeviceProfile::DJI_GEN1.matches(0x2CA3, 0x4006));
    assert!(!DeviceProfile::DJI_GEN1.matches(0x2CA3, 0x4009));
    assert!(!DeviceProfile::DJI_GEN1.matches(0x1234, 0x4006));
}

#[test]
fn classifies_bound_public_and_dns_success_as_available() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert_eq!(
        classify(&bound_public_and_dns_ok(now), now).status,
        Availability::Available
    );
}

#[test]
fn classifies_dns_only_failure_as_limited() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert!(matches!(
        classify(&public_ok_dns_failed(now), now).status,
        Availability::Limited(LimitedReason::DnsFailure)
    ));
}

#[test]
fn rejects_positive_evidence_from_an_old_device_epoch() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert_eq!(
        classify(&old_epoch_success_after_replug(now), now).status,
        Availability::Detecting
    );
}

#[test]
fn global_phone_connectivity_cannot_prove_dongle_reachability() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert_eq!(
        classify(&global_phone_only(now), now).status,
        Availability::Unavailable(UnavailableReason::NoBoundReachability)
    );
}

#[test]
fn classification_table_covers_the_spec_rules() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut cases: Vec<(&str, ClassificationInput, Availability)> = Vec::new();

    let mut single_family = supported(now);
    single_family.protocol_coverage = Some(evidence(
        now,
        EvidenceSource::BoundPublicProbe,
        bound(ProtocolCoverage::SingleFamilyOnly),
    ));
    cases.push((
        "only one required protocol family works",
        single_family,
        Availability::Limited(LimitedReason::SingleProtocolFamily),
    ));

    let mut competing_route = supported(now);
    competing_route.system_default_route = Some(evidence(
        now,
        EvidenceSource::GlobalRoute,
        DefaultRouteOwner::VpnOrTun,
    ));
    cases.push((
        "VPN or TUN owns the global default route",
        competing_route,
        Availability::Limited(LimitedReason::CompetingDefaultRoute),
    ));

    let mut at_unavailable = supported(now);
    at_unavailable.at_control = Some(evidence(
        now,
        EvidenceSource::AtControl,
        AtControlAvailability::Unavailable,
    ));
    cases.push((
        "AT unavailable while bound RNDIS data succeeds",
        at_unavailable,
        Availability::Limited(LimitedReason::AtControlUnavailable),
    ));

    let mut incomplete = supported(now);
    incomplete.bound_dns = Some(evidence(
        now,
        EvidenceSource::BoundDnsProbe,
        bound(BoundDnsStatus::Incomplete),
    ));
    cases.push((
        "required probe evidence is incomplete",
        incomplete,
        Availability::Limited(LimitedReason::IncompleteEvidence),
    ));

    let mut public_incomplete = supported(now);
    public_incomplete.bound_public = Some(evidence(
        now,
        EvidenceSource::BoundPublicProbe,
        bound(BoundPublicStatus::Incomplete),
    ));
    cases.push((
        "bound public evidence is incomplete",
        public_incomplete,
        Availability::Limited(LimitedReason::IncompleteEvidence),
    ));

    let mut at_not_observed = supported(now);
    at_not_observed.at_control = None;
    cases.push((
        "AT evidence has not completed",
        at_not_observed,
        Availability::Limited(LimitedReason::IncompleteEvidence),
    ));

    let mut family_not_observed = supported(now);
    family_not_observed.protocol_coverage = None;
    cases.push((
        "protocol-family evidence has not completed",
        family_not_observed,
        Availability::Limited(LimitedReason::IncompleteEvidence),
    ));

    for block in [
        CellularBlock::SimRejected,
        CellularBlock::RegistrationRejected,
    ] {
        let mut rejected = supported(now);
        rejected.cellular_block = Some(evidence(now, EvidenceSource::AtControl, block));
        cases.push((
            "SIM or registration is definitively rejected",
            rejected,
            Availability::Unavailable(UnavailableReason::CellularRejected),
        ));
    }

    let mut no_route = supported(now);
    no_route.adapter = Some(evidence(
        now,
        EvidenceSource::WindowsAdapter,
        AdapterState::NoUsableAddressOrRoute,
    ));
    cases.push((
        "Windows has no usable address or route",
        no_route,
        Availability::Unavailable(UnavailableReason::NoUsableAddressOrRoute),
    ));

    let mut failed_twice = supported(now);
    failed_twice.bound_public = Some(evidence(
        now,
        EvidenceSource::BoundPublicProbe,
        bound(BoundPublicStatus::Failed {
            consecutive_cycles: 2,
        }),
    ));
    cases.push((
        "independent bound public probes fail twice",
        failed_twice,
        Availability::Unavailable(UnavailableReason::BoundPublicProbeFailed),
    ));

    let mut failed_once = supported(now);
    failed_once.bound_public = Some(evidence(
        now,
        EvidenceSource::BoundPublicProbe,
        bound(BoundPublicStatus::Failed {
            consecutive_cycles: 1,
        }),
    ));
    cases.push((
        "one public failure is not yet definitive",
        failed_once,
        Availability::Limited(LimitedReason::IncompleteEvidence),
    ));

    let mut absent = supported(now);
    absent.device_presence = Some(evidence(
        now,
        EvidenceSource::Pnp,
        DevicePresence::NotDetected,
    ));
    cases.push((
        "current enumeration proves the target absent",
        absent,
        Availability::NotDetected,
    ));

    let mut unsupported = supported(now);
    unsupported.device_presence = Some(evidence(
        now,
        EvidenceSource::Pnp,
        DevicePresence::Unsupported {
            vid: 0x2CA3,
            pid: 0x4009,
        },
    ));
    cases.push((
        "related out-of-scope device is present",
        unsupported,
        Availability::UnsupportedDevice,
    ));

    for phase in [
        ClassificationPhase::Startup,
        ClassificationPhase::RecentInsertion,
        ClassificationPhase::Reenumerating,
        ClassificationPhase::PostWriteVerification,
    ] {
        let mut detecting = supported(now);
        detecting.phase = phase;
        cases.push((
            "transitional phase is detecting",
            detecting,
            Availability::Detecting,
        ));
    }

    let mut expired = supported(now);
    expired.bound_public.as_mut().unwrap().observed_at = now - Duration::from_secs(31);
    cases.push((
        "expired positive evidence is discarded",
        expired,
        Availability::Detecting,
    ));

    for (name, input, expected) in cases {
        assert_eq!(classify(&input, now).status, expected, "{name}");
    }
}

#[test]
fn bound_data_success_is_stronger_than_unavailable_at_control() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut input = supported(now);
    input.at_control = Some(evidence(
        now,
        EvidenceSource::AtControl,
        AtControlAvailability::Unavailable,
    ));

    assert_eq!(
        classify(&input, now).status,
        Availability::Limited(LimitedReason::AtControlUnavailable)
    );
}

#[test]
fn current_absence_overrides_leftover_positive_evidence_from_the_previous_epoch() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut input = supported(now);
    input.current_epoch = DeviceEpoch(EPOCH.0 + 1);
    input.device_presence = Some(Evidence {
        epoch: input.current_epoch,
        observed_at: now,
        ttl: Duration::from_secs(30),
        source: EvidenceSource::Pnp,
        value: DevicePresence::NotDetected,
    });

    assert_eq!(classify(&input, now).status, Availability::NotDetected);
}

#[test]
fn current_absence_and_unsupported_device_win_during_transitional_phases() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    for phase in [
        ClassificationPhase::RecentInsertion,
        ClassificationPhase::Reenumerating,
    ] {
        let mut absent = supported(now);
        absent.phase = phase;
        absent.device_presence = Some(evidence(
            now,
            EvidenceSource::Pnp,
            DevicePresence::NotDetected,
        ));
        assert_eq!(classify(&absent, now).status, Availability::NotDetected);

        let mut unsupported = supported(now);
        unsupported.phase = phase;
        unsupported.device_presence = Some(evidence(
            now,
            EvidenceSource::Pnp,
            DevicePresence::Unsupported {
                vid: 0x2CA3,
                pid: 0x4009,
            },
        ));
        assert_eq!(
            classify(&unsupported, now).status,
            Availability::UnsupportedDevice
        );
    }
}

#[test]
fn future_dated_evidence_is_not_fresh() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut input = supported(now);
    input.bound_public.as_mut().unwrap().observed_at = now + Duration::from_secs(1);

    assert_eq!(classify(&input, now).status, Availability::Detecting);
}

#[test]
fn stale_global_connectivity_does_not_demote_fresh_bound_success() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut input = supported(now);
    let mut stale_global = evidence(
        now,
        EvidenceSource::GlobalConnectivity,
        GlobalConnectivity::Offline,
    );
    stale_global.observed_at = now - Duration::from_secs(31);
    input.global_connectivity = Some(stale_global);

    assert_eq!(classify(&input, now).status, Availability::Available);
}

#[test]
fn phone_wifi_or_tun_binding_cannot_supply_dji_bound_success() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    for adapter_id in ["phone-wifi", "meta-tun"] {
        let mut input = supported(now);
        for binding in [
            &mut input.bound_public.as_mut().unwrap().value.binding,
            &mut input.bound_dns.as_mut().unwrap().value.binding,
            &mut input.protocol_coverage.as_mut().unwrap().value.binding,
        ] {
            binding.adapter_id = adapter_id.to_owned();
        }
        input.global_connectivity = Some(evidence(
            now,
            EvidenceSource::GlobalConnectivity,
            GlobalConnectivity::Online,
        ));

        assert_eq!(
            classify(&input, now).status,
            Availability::Unavailable(UnavailableReason::NoBoundReachability),
            "{adapter_id}"
        );
    }
}

#[test]
fn global_sources_cannot_masquerade_as_bound_probe_evidence() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    let mut public = supported(now);
    public.bound_public.as_mut().unwrap().source = EvidenceSource::GlobalConnectivity;
    assert_eq!(
        classify(&public, now).status,
        Availability::Limited(LimitedReason::IncompleteEvidence)
    );

    let mut dns = supported(now);
    dns.bound_dns.as_mut().unwrap().source = EvidenceSource::GlobalConnectivity;
    assert_eq!(
        classify(&dns, now).status,
        Availability::Limited(LimitedReason::IncompleteEvidence)
    );

    let mut family = supported(now);
    family.protocol_coverage.as_mut().unwrap().source = EvidenceSource::GlobalConnectivity;
    assert_eq!(
        classify(&family, now).status,
        Availability::Limited(LimitedReason::IncompleteEvidence)
    );
}

fn at_unavailable(now: SystemTime) -> Evidence<AtControlAvailability> {
    evidence(
        now,
        EvidenceSource::AtControl,
        AtControlAvailability::Unavailable,
    )
}

/// The real-machine case behind this fix: the module is present and its RNDIS adapter works, but
/// its AT serial interfaces are in an error state, so `select_at_port` fails with
/// `pnp:no_safe_at_port` and no cellular evidence can ever be collected.  The verdict must name the
/// AT fault instead of hiding it behind the generic incomplete-evidence wording, which reads to a
/// user as "still working on it" rather than "this specific capability is broken".
#[test]
fn present_device_with_unavailable_at_port_names_the_fault_not_incomplete_evidence() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    // Active probing turned off in settings: no bound evidence can ever be collected.
    let mut probing_disabled = supported(now);
    probing_disabled.bound_public = None;
    probing_disabled.bound_dns = None;
    probing_disabled.protocol_coverage = None;
    probing_disabled.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&probing_disabled, now).status,
        Availability::Limited(LimitedReason::AtControlUnavailable),
        "the AT fault must be named when bound probing is disabled"
    );

    // The adapter binding has not been resolved yet.
    let mut no_binding = supported(now);
    no_binding.adapter_binding = None;
    no_binding.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&no_binding, now).status,
        Availability::Limited(LimitedReason::AtControlUnavailable),
        "the AT fault must be named when the adapter binding is missing"
    );

    // The adapter state itself has not been observed yet.
    let mut no_adapter_state = supported(now);
    no_adapter_state.adapter = None;
    no_adapter_state.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&no_adapter_state, now).status,
        Availability::Limited(LimitedReason::AtControlUnavailable),
        "the AT fault must be named when the adapter state is missing"
    );

    // Bound DNS is still incomplete even though bound public reachability succeeded.
    let mut dns_incomplete = supported(now);
    dns_incomplete.bound_dns = Some(evidence(
        now,
        EvidenceSource::BoundDnsProbe,
        bound(BoundDnsStatus::Incomplete),
    ));
    dns_incomplete.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&dns_incomplete, now).status,
        Availability::Limited(LimitedReason::AtControlUnavailable),
        "the AT fault must be named when bound DNS is incomplete"
    );

    // Protocol coverage has not been established.
    let mut no_coverage = supported(now);
    no_coverage.protocol_coverage = None;
    no_coverage.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&no_coverage, now).status,
        Availability::Limited(LimitedReason::AtControlUnavailable),
        "the AT fault must be named when protocol coverage is missing"
    );
}

/// A broken AT port is never a path to a green state, however much other evidence is missing.
#[test]
fn unavailable_at_control_can_never_produce_available() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    let mut full = supported(now);
    full.at_control = Some(at_unavailable(now));
    assert_ne!(classify(&full, now).status, Availability::Available);

    let mut stripped = supported(now);
    stripped.adapter_binding = None;
    stripped.adapter = None;
    stripped.bound_public = None;
    stripped.bound_dns = None;
    stripped.protocol_coverage = None;
    stripped.system_default_route = None;
    stripped.at_control = Some(at_unavailable(now));
    assert_ne!(classify(&stripped, now).status, Availability::Available);
}

/// Naming the AT fault must never demote a stronger, more actionable negative verdict.
#[test]
fn definite_unavailable_verdicts_are_not_masked_by_an_unavailable_at_port() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    let mut no_route = supported(now);
    no_route.adapter = Some(evidence(
        now,
        EvidenceSource::WindowsAdapter,
        AdapterState::NoUsableAddressOrRoute,
    ));
    no_route.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&no_route, now).status,
        Availability::Unavailable(UnavailableReason::NoUsableAddressOrRoute)
    );

    let mut probe_failed = supported(now);
    probe_failed.bound_public = Some(evidence(
        now,
        EvidenceSource::BoundPublicProbe,
        bound(BoundPublicStatus::Failed {
            consecutive_cycles: 2,
        }),
    ));
    probe_failed.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&probe_failed, now).status,
        Availability::Unavailable(UnavailableReason::BoundPublicProbeFailed)
    );

    let mut phone_only = supported(now);
    phone_only.bound_public = None;
    phone_only.global_connectivity = Some(evidence(
        now,
        EvidenceSource::GlobalConnectivity,
        GlobalConnectivity::Online,
    ));
    phone_only.at_control = Some(at_unavailable(now));
    assert_eq!(
        classify(&phone_only, now).status,
        Availability::Unavailable(UnavailableReason::NoBoundReachability)
    );
}

/// Fail-closed: only a correctly-sourced, fresh AT observation may explain incomplete evidence.
#[test]
fn mis_sourced_at_evidence_cannot_explain_incomplete_evidence() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    let mut input = supported(now);
    input.bound_public = None;
    input.at_control = Some(evidence(
        now,
        EvidenceSource::GlobalConnectivity,
        AtControlAvailability::Unavailable,
    ));

    assert_eq!(
        classify(&input, now).status,
        Availability::Limited(LimitedReason::IncompleteEvidence)
    );
}
