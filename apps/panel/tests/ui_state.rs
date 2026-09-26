use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dji4g_application::{
    ActionKindTag, BackendEvent, CheckMask, CheckResult, CommandStateSnapshot, ControllerSnapshot,
    DiagnosticCheckState, DiagnosticSet, FailureCode, OperationPhase, OperationState,
    OperationUiSnapshot, ReducerState, RefreshCycleId, SettingsSnapshot, StableCode,
    UnexecutedReason, reduce_state,
};
use dji4g_domain::{
    ActionKind, AppSnapshot, Availability, BoundDnsStatus, BoundPublicStatus, DeviceEpoch,
    ErrorCode, Freshness, HotspotStatus, LimitedReason, NetworkSnapshot, ProtocolCoverage,
    UnavailableReason,
};
use dji4g_panel::localization::{
    Language, TextKey, availability_reason, availability_title, error_text, stable_code_text,
    template,
};
use dji4g_panel::ui::{
    StatusTone, availability_vm, diagnostic_state_vm, operation_outcome_text, overview_vm,
    repairs_vm,
};

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn app_snapshot(
    availability: Availability,
    freshness: Freshness,
    hotspot: HotspotStatus,
) -> AppSnapshot {
    AppSnapshot {
        revision: 7,
        observed_at: now(),
        freshness,
        availability,
        hotspot,
        device: None,
        cellular: None,
        network: None,
        active_operation: None,
        issues: Vec::new(),
    }
}

fn empty_diagnostics() -> dji4g_application::DiagnosticSet {
    dji4g_application::DiagnosticSet::new(dji4g_domain::DeviceEpoch(1))
}

fn network_with_rates(down: u64, up: u64) -> NetworkSnapshot {
    NetworkSnapshot {
        adapter_id: "{adapter}".into(),
        addresses: vec!["192.168.225.30".into()],
        gateways: vec!["192.168.225.1".into()],
        dns_servers: vec!["192.168.225.1".into()],
        adapter_state: dji4g_domain::AdapterState::UsableAddressAndRoute,
        bound_public: BoundPublicStatus::Succeeded,
        bound_dns: BoundDnsStatus::Succeeded,
        protocol_coverage: ProtocolCoverage::AllRequiredFamilies,
        system_default_route: dji4g_domain::DefaultRouteOwner::TargetAdapter,
        down_bytes_per_sec: Some(down),
        up_bytes_per_sec: Some(up),
    }
}

fn controller_snapshot(app: AppSnapshot) -> ControllerSnapshot {
    ControllerSnapshot {
        module_network_check: None,
        host_network: dji4g_application::HostNetworkSnapshot::default(),
        publication_revision: 7,
        app: Arc::new(app),
        diagnostics: DiagnosticSet::new(DeviceEpoch(1)),
        prepared_action: None,
        operation: None,
        settings: SettingsSnapshot::default(),
        command_state: CommandStateSnapshot::default(),
        action_readiness: Vec::new(),
        feedback: None,
        sim_epoch: 0,
        feature_status: None,
        adapter_metrics: None,
        timeline: Default::default(),
        sms_inbox: Default::default(),
        sms_messages: Vec::new(),
        sms_send: None,
        sms_delete: None,
        serial_work_busy: false,
        sms_refresh_pending: false,
        sms_read_phase: None,
        sms_read_progress: 0,
        sms_read_report: None,
        sms_inbox_failure: None,
        device_tools: Default::default(),
    }
}

fn text(value: impl AsRef<str>) -> String {
    value.as_ref().to_owned()
}

#[test]
fn every_availability_has_chinese_title_reason_and_freshness() {
    let statuses = [
        Availability::Detecting,
        Availability::Available,
        Availability::Limited(LimitedReason::DnsFailure),
        Availability::Limited(LimitedReason::SingleProtocolFamily),
        Availability::Limited(LimitedReason::CompetingDefaultRoute),
        Availability::Limited(LimitedReason::AtControlUnavailable),
        Availability::Limited(LimitedReason::IncompleteEvidence),
        Availability::Unavailable(UnavailableReason::CellularRejected),
        Availability::Unavailable(UnavailableReason::NoUsableAddressOrRoute),
        Availability::Unavailable(UnavailableReason::BoundPublicProbeFailed),
        Availability::Unavailable(UnavailableReason::NoBoundReachability),
        Availability::NotDetected,
        Availability::UnsupportedDevice,
    ];

    for status in statuses {
        let vm = availability_vm(
            &app_snapshot(status, Freshness::Fresh, HotspotStatus::Off),
            &empty_diagnostics(),
            now(),
            Language::ZhCn,
        );
        assert!(!vm.title.text.trim().is_empty());
        assert!(!vm.reason.text.trim().is_empty());
        assert!(!vm.freshness.text.trim().is_empty());
        assert!(
            vm.title
                .text
                .chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        );
        assert!(
            vm.reason
                .text
                .chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        );
    }
}

#[test]
fn stale_available_never_remains_green_or_confirmed() {
    let vm = availability_vm(
        &app_snapshot(
            Availability::Available,
            Freshness::Stale,
            HotspotStatus::Off,
        ),
        &empty_diagnostics(),
        now() + Duration::from_secs(60),
        Language::ZhCn,
    );
    assert_ne!(vm.tone, StatusTone::Positive);
    assert!(!vm.is_confirmed_usable);
    assert!(vm.title.text.contains("过期") || vm.freshness.text.contains("过期"));
}

#[test]
fn hotspot_state_does_not_change_module_availability_presentation() {
    let statuses = [
        HotspotStatus::Unsupported(dji4g_domain::HotspotUnsupportedReason::NoWifiAdapter),
        HotspotStatus::Off,
        HotspotStatus::Starting,
        HotspotStatus::On { clients: Some(2) },
        HotspotStatus::On { clients: None },
        HotspotStatus::Stopping,
        HotspotStatus::Failed {
            code: ErrorCode::CapabilityUnavailable,
        },
    ];
    let mut baseline = None;
    for hotspot in statuses {
        let vm = availability_vm(
            &app_snapshot(
                Availability::Limited(LimitedReason::DnsFailure),
                Freshness::Fresh,
                hotspot,
            ),
            &empty_diagnostics(),
            now(),
            Language::ZhCn,
        );
        let current = (vm.tone, vm.title.text.clone(), vm.reason.text.clone());
        if let Some(expected) = &baseline {
            assert_eq!(&current, expected);
        } else {
            baseline = Some(current);
        }
    }
}

#[test]
fn dns_failure_explains_data_path_and_dns_separately() {
    let vm = availability_vm(
        &app_snapshot(
            Availability::Limited(LimitedReason::DnsFailure),
            Freshness::Fresh,
            HotspotStatus::On { clients: Some(2) },
        ),
        &empty_diagnostics(),
        now(),
        Language::ZhCn,
    );
    assert!(vm.reason.text.contains("数据通路"));
    assert!(vm.reason.text.contains("DNS"));
    assert!(!vm.reason.text.contains("4G 无网络"));
}

#[test]
fn no_bound_reachability_does_not_claim_global_network_is_the_module() {
    let vm = availability_vm(
        &app_snapshot(
            Availability::Unavailable(UnavailableReason::NoBoundReachability),
            Freshness::Fresh,
            HotspotStatus::Off,
        ),
        &empty_diagnostics(),
        now(),
        Language::ZhCn,
    );
    assert!(vm.reason.text.contains("其他网络"));
    assert!(vm.reason.text.contains("模块通路"));
}

#[test]
fn all_diagnostic_lifecycles_remain_distinct() {
    let failure = FailureCode::new(
        ErrorCode::ProbeFailed,
        StableCode::try_from_static("probe:failed").expect("safe code"),
    );
    let states = [
        DiagnosticCheckState::Unexecuted {
            reason: UnexecutedReason::NotScheduled,
        },
        DiagnosticCheckState::Running {
            cycle: dji4g_application::RefreshCycleId(1),
        },
        DiagnosticCheckState::Passed,
        DiagnosticCheckState::Failed {
            code: failure.clone(),
        },
        DiagnosticCheckState::Unavailable { code: failure },
        DiagnosticCheckState::Expired,
    ];
    let labels: Vec<String> = states
        .into_iter()
        .map(|state| text(diagnostic_state_vm(&state, Language::ZhCn).label))
        .collect();
    assert_eq!(
        labels,
        ["未执行", "检测中", "已通过", "未通过", "不可用", "已过期"]
    );
}

#[test]
fn native_confirm_message_is_composed_from_the_click_metadata() {
    // The confirmation message is composed from the click itself (never from a later snapshot),
    // so the box can appear the moment the button is pressed. The content carries the same
    // operation/disruption/risk/elevation lines the snapshot-driven box used to carry.
    let (title, message) = dji4g_panel::native_dialog::confirm_message_for_action(
        &ActionKind::RestartModule,
        true,
        Some(dji4g_domain::DisruptionLevel::DeviceReenumeration),
        Some(dji4g_domain::RiskLevel::High),
        true,
        Language::ZhCn,
    )
    .expect("a confirmable action must produce a confirm message");
    assert_eq!(title, template(Language::ZhCn, TextKey::ConfirmationTitle));
    assert!(
        message.contains("开发构建"),
        "the dev warning must be part of the native dialog text"
    );
    assert!(message.contains("操作："));
    assert!(message.contains("中断："));
    assert!(message.contains("风险："));
    assert!(message.contains("提权："));
    assert!(message.contains(template(Language::ZhCn, TextKey::ConfirmationStateRecheck)));
    assert!(message.contains(template(
        Language::ZhCn,
        TextKey::ConfirmationNoAutomaticRetry
    )));

    // A non-dev build (or a non-elevated action) never appends the development warning.
    let (_title, message) = dji4g_panel::native_dialog::confirm_message_for_action(
        &ActionKind::RestartModule,
        true,
        Some(dji4g_domain::DisruptionLevel::DeviceReenumeration),
        Some(dji4g_domain::RiskLevel::High),
        false,
        Language::ZhCn,
    )
    .expect("a confirmable action must produce a confirm message");
    assert!(!message.contains("开发构建"));

    // Refresh is not a confirmable operation and therefore never composes a box.
    assert!(
        dji4g_panel::native_dialog::confirm_message_for_action(
            &ActionKind::Refresh,
            false,
            None,
            None,
            false,
            Language::ZhCn,
        )
        .is_none()
    );
}

#[test]
fn native_result_request_follows_a_finished_operation() {
    let mut snapshot = controller_snapshot(app_snapshot(
        Availability::Available,
        Freshness::Fresh,
        HotspotStatus::Off,
    ));
    // A running operation never opens a result box.
    snapshot.operation = Some(OperationUiSnapshot {
        operation_id: 1,
        action: ActionKindTag::ToggleHotspot { enabled: true },
        started_at: now(),
        state: OperationState::Running {
            phase: OperationPhase::Executing,
        },
    });
    assert!(
        dji4g_panel::native_dialog::result_request(&snapshot, 1, Language::ZhCn).is_none(),
        "a running operation must not open a result box"
    );

    snapshot.operation = Some(OperationUiSnapshot {
        operation_id: 1,
        action: ActionKindTag::ToggleHotspot { enabled: true },
        started_at: now(),
        state: OperationState::Finished {
            outcome: dji4g_domain::OperationOutcome::Failed {
                code: ErrorCode::Internal,
                rollback: dji4g_domain::RollbackOutcome::NotAttempted,
            },
            finished_at: now(),
        },
    });
    let dji4g_panel::native_dialog::DialogRequest::Result {
        operation_id,
        message,
        ..
    } = dji4g_panel::native_dialog::result_request(&snapshot, 1, Language::ZhCn)
        .expect("a finished operation must present a result box");
    assert_eq!(operation_id, 1);
    assert!(
        message.contains("操作失败"),
        "the result box must carry the honest outcome text"
    );
}
#[test]
fn outcome_unknown_is_a_warning_without_retry_command() {
    let outcome = dji4g_domain::OperationOutcome::OutcomeUnknown {
        code: ErrorCode::Timeout,
    };
    let rendered = operation_outcome_text(&outcome, Language::ZhCn);
    assert!(rendered.text.contains("无法确认"));
    assert!(rendered.text.contains("不会自动重试"));
    assert!(!rendered.text.contains("一键重试"));
}

#[test]
fn chinese_catalog_is_nonempty_and_known_codes_are_localized() {
    for key in TextKey::ALL {
        let value = template(Language::ZhCn, *key);
        assert!(!value.trim().is_empty(), "empty catalog entry: {key:?}");
        assert!(!value.contains("TODO"));
        assert!(!value.contains("TBD"));
        assert!(!value.contains("untranslated"));
    }
    assert_eq!(
        availability_title(Availability::Available),
        TextKey::AvailabilityAvailableTitle
    );
    assert_eq!(
        availability_reason(Availability::Available),
        TextKey::AvailabilityAvailableReason
    );
    assert_eq!(error_text(ErrorCode::DnsFailed), TextKey::ErrorDnsFailed);
    assert_eq!(
        stable_code_text("pnp:no_safe_at_port"),
        Some(TextKey::PlatformNoSafeAtPort)
    );
    assert_eq!(stable_code_text("future_component:new_code"), None);
}

#[test]
fn network_values_are_available_to_diagnostics_without_raw_backend_strings() {
    let network = NetworkSnapshot {
        adapter_id: "{adapter}".into(),
        addresses: vec!["192.168.225.30".into()],
        gateways: vec!["192.168.225.1".into()],
        dns_servers: vec!["192.168.225.1".into()],
        adapter_state: dji4g_domain::AdapterState::UsableAddressAndRoute,
        bound_public: BoundPublicStatus::Succeeded,
        bound_dns: BoundDnsStatus::Succeeded,
        protocol_coverage: ProtocolCoverage::AllRequiredFamilies,
        system_default_route: dji4g_domain::DefaultRouteOwner::TargetAdapter,
        down_bytes_per_sec: None,
        up_bytes_per_sec: None,
    };
    let snapshot = app_snapshot(
        Availability::Available,
        Freshness::Fresh,
        HotspotStatus::Off,
    );
    let mut snapshot = snapshot;
    snapshot.network = Some(network);
    let vm = dji4g_panel::ui::diagnostics_vm(&controller_snapshot(snapshot), Language::ZhCn);
    let rendered = vm
        .rows
        .iter()
        .flat_map(|row| {
            std::iter::once(row.label.text.clone())
                .chain(row.detail.iter().map(|detail| detail.text.clone()))
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(rendered.contains("192.168.225.30"));
    assert!(!rendered.contains("{adapter}"));
}

#[test]
fn action_kind_is_closed_at_the_ui_boundary() {
    let action = ActionKind::RestartModule;
    let rendered = dji4g_panel::localization::action_text(&action, Language::ZhCn);
    assert!(rendered.text.contains("重启"));
    assert!(!rendered.text.contains("Debug"));
}

/// Pins the user-visible difference between the reported defect and the fixed behaviour.
///
/// A release build used to never scan at all, so the header stayed in its initial state and read
/// 「模块网络：正在检测」 + 「正在收集并校验当前设备的连接证据。」 + 「尚无有效的更新时间」 even though
/// the module was plugged in and providing internet.  After one scan the same machine must read
/// 「受限」 with a reason that names the AT port, and must still never read 「可用」.
#[test]
fn at_port_fault_reads_as_a_recognised_device_not_as_perpetual_detection() {
    // The misleading state: nothing has ever been observed.
    let unscanned = availability_vm(
        &app_snapshot(
            Availability::Detecting,
            Freshness::Unknown,
            HotspotStatus::Off,
        ),
        &empty_diagnostics(),
        now(),
        Language::ZhCn,
    );
    assert_eq!(
        unscanned.title.text,
        template(Language::ZhCn, TextKey::AvailabilityDetectingTitle)
    );
    assert_eq!(
        unscanned.freshness.text,
        template(Language::ZhCn, TextKey::FreshnessUnknown)
    );
    assert!(!unscanned.is_confirmed_usable);

    // The honest state after one scan of that same machine.
    let scanned = availability_vm(
        &app_snapshot(
            Availability::Limited(LimitedReason::AtControlUnavailable),
            Freshness::Fresh,
            HotspotStatus::Off,
        ),
        &empty_diagnostics(),
        now(),
        Language::ZhCn,
    );
    assert_eq!(scanned.title.text, "受限");
    assert_eq!(
        scanned.reason.text,
        "设备已识别，但 AT 端口不可用（串口异常），无法读取蜂窝状态。"
    );
    assert_eq!(scanned.tone, StatusTone::Caution);
    assert_ne!(
        scanned.title.text, unscanned.title.text,
        "the panel must leave 「正在检测」 once evidence exists"
    );
    assert_ne!(
        scanned.title.text,
        template(Language::ZhCn, TextKey::AvailabilityNotDetectedTitle),
        "a present device must never be reported as 未检测到"
    );
    // Never green: a broken AT port cannot be promoted to 可用.
    assert!(!scanned.is_confirmed_usable);
    assert_ne!(
        scanned.title.text,
        template(Language::ZhCn, TextKey::AvailabilityAvailableTitle)
    );
}

fn stable_failure(code: &'static str) -> FailureCode {
    FailureCode::new(
        ErrorCode::CapabilityUnavailable,
        StableCode::try_from_static(code).unwrap(),
    )
}

#[test]
fn header_reason_uses_the_failed_at_check_instead_of_the_generic_sentence() {
    let now = now();
    let mut state = ReducerState::test_ready(now);
    state = reduce_state(
        &state,
        BackendEvent::RefreshStarted {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
            scheduled: CheckMask::only(dji4g_application::DiagnosticCheckId::AtControl),
        },
        now,
    );
    state = reduce_state(
        &state,
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
            result: CheckResult::Failed {
                code: stable_failure("pnp:ambiguous_at_port"),
                observed_at: now,
            },
        },
        now,
    );
    state = reduce_state(
        &state,
        BackendEvent::RefreshFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
        },
        now,
    );
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.app.availability,
        Availability::Limited(LimitedReason::AtControlUnavailable)
    );

    let vm = availability_vm(&snapshot.app, &snapshot.diagnostics, now, Language::ZhCn);
    assert!(
        vm.reason.text.contains("找到多个同等候选"),
        "the header must carry the precise cause, got: {}",
        vm.reason.text
    );
}

#[test]
fn header_reason_falls_back_when_the_check_has_no_precise_code() {
    let now = now();
    let state = ReducerState::test_ready(now);
    let snapshot = state.snapshot();
    let vm = availability_vm(&snapshot.app, &snapshot.diagnostics, now, Language::ZhCn);
    assert_ne!(snapshot.app.freshness, Freshness::Stale);
    assert!(!vm.reason.text.contains("找到多个同等候选"));
}

#[test]
fn running_operation_disables_repair_actions_until_finished() {
    let mut snapshot = ReducerState::test_ready(now()).snapshot();
    assert!(
        snapshot.app.device.is_some(),
        "the ready fixture has a device"
    );

    snapshot.serial_work_busy = true;
    snapshot.operation = Some(OperationUiSnapshot {
        operation_id: 1,
        action: ActionKindTag::ToggleHotspot { enabled: true },
        started_at: now(),
        state: OperationState::Running {
            phase: OperationPhase::Revalidating,
        },
    });
    let vm = repairs_vm(&snapshot, now(), Language::ZhCn);
    assert!(
        vm.actions.iter().all(|action| !action.enabled),
        "no repair button may look clickable while an operation runs"
    );

    snapshot.operation = Some(OperationUiSnapshot {
        operation_id: 1,
        action: ActionKindTag::ToggleHotspot { enabled: true },
        started_at: now(),
        state: OperationState::Finished {
            outcome: dji4g_domain::OperationOutcome::Applied {
                after_state_hash: dji4g_domain::AfterStateHash([0_u8; 32]),
            },
            finished_at: now(),
        },
    });
    // A finished operation restores exactly the baseline enablement (some actions carry extra
    // per-action gates, so the baseline comparison is the honest invariant).
    snapshot.serial_work_busy = false;
    let mut baseline_snapshot = ReducerState::test_ready(now()).snapshot();
    baseline_snapshot.operation = None;
    let baseline = repairs_vm(&baseline_snapshot, now(), Language::ZhCn);
    let vm = repairs_vm(&snapshot, now(), Language::ZhCn);
    let enabled_flags = |vm: &dji4g_panel::ui::RepairsVm| {
        vm.actions
            .iter()
            .map(|action| action.enabled)
            .collect::<Vec<_>>()
    };
    assert_eq!(enabled_flags(&vm), enabled_flags(&baseline));
}

#[test]
fn rate_formatting_is_a_closed_1024_ladder() {
    use dji4g_panel::ui::format_rate;
    assert_eq!(format_rate(0), "0 B/s");
    assert_eq!(format_rate(999), "999 B/s");
    assert_eq!(format_rate(1023), "1023 B/s");
    assert_eq!(format_rate(1024), "1 KB/s");
    assert_eq!(format_rate(12_600), "12.3 KB/s");
    assert_eq!(format_rate(1_572_864), "1.5 MB/s");
    assert_eq!(format_rate(2 * 1024 * 1024), "2 MB/s");
    assert_eq!(format_rate(3 * 1024 * 1024 * 1024), "3 GB/s");
}

#[test]
fn overview_rows_carry_the_measured_rates_or_an_honest_gap() {
    let network = network_with_rates(12_600, 1_024);
    let mut snapshot = controller_snapshot(app_snapshot(
        Availability::Available,
        Freshness::Fresh,
        HotspotStatus::Off,
    ));
    snapshot.app = Arc::new({
        let mut app = (*snapshot.app).clone();
        app.network = Some(network);
        app
    });

    let vm = overview_vm(&snapshot, Language::ZhCn);
    assert_eq!(vm.down_rate, Some(12_600));
    assert_eq!(vm.up_rate, Some(1_024));

    let mut without = snapshot;
    without.app = Arc::new({
        let mut app = (*without.app).clone();
        app.network = None;
        app
    });
    let vm = overview_vm(&without, Language::ZhCn);
    assert_eq!(vm.down_rate, None);
    assert_eq!(vm.up_rate, None);
}

#[test]
fn the_rate_history_is_a_bounded_ring() {
    let mut history = dji4g_panel::ui::RateHistory::new();
    for index in 0..70_u64 {
        history.push((
            now() + Duration::from_secs(index),
            Some(index * 10),
            Some(index),
        ));
    }
    assert_eq!(history.len(), 60);
    let (at, down, up) = history.last().expect("ring retains samples");
    assert_eq!(*at, now() + Duration::from_secs(69));
    assert_eq!(*down, Some(690));
    assert_eq!(*up, Some(69));
}

#[test]
fn carrier_names_gain_their_chinese_name_from_a_closed_set() {
    use dji4g_panel::ui::carrier_display_name;
    assert_eq!(carrier_display_name("CHN-UNICOM"), "CHN-UNICOM（中国联通）");
    assert_eq!(carrier_display_name("Chn-Mobile"), "Chn-Mobile（中国移动）");
    assert_eq!(carrier_display_name("CMCC"), "CMCC（中国移动）");
    assert_eq!(
        carrier_display_name("CHN-TELECOM"),
        "CHN-TELECOM（中国电信）"
    );
    // Unknown and empty names pass through untouched — nothing is guessed.
    assert_eq!(carrier_display_name("Foo Carrier"), "Foo Carrier");
    assert_eq!(carrier_display_name(""), "");
}
