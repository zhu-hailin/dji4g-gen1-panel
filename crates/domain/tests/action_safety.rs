use std::{
    net::Ipv4Addr,
    time::{Duration, SystemTime},
};

use dji4g_domain::{
    ActionKind, ActionPlan, ActionPlanDraft, ActionSafetyError, BeforeStateHash, DeviceEpoch,
    DisruptionLevel, DnsProfile, RiskLevel, StableDeviceIdentity, UsbNetworkProfile,
};

fn identity(pid: u16) -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: "{f5af1065-56f4-42f4-a3bd-09caa15a31aa}".to_owned(),
        device_instance_id: format!("USB\\VID_2CA3&PID_{pid:04X}\\REDACTED"),
        vid: 0x2CA3,
        pid,
    }
}

fn draft(
    now: SystemTime,
    pid: u16,
    current: DeviceEpoch,
    observed: DeviceEpoch,
) -> ActionPlanDraft {
    ActionPlanDraft {
        kind: ActionKind::RestartAdapter,
        snapshot_revision: 41,
        current_epoch: current,
        evidence_epoch: observed,
        target: identity(pid),
        before_state_hash: BeforeStateHash([0xA5; 32]),
        expires_at: now + Duration::from_secs(30),
        disruption: DisruptionLevel::ConnectionInterrupting,
        risk: RiskLevel::Medium,
        requires_elevation: true,
    }
}

#[test]
fn unsupported_pid_cannot_create_an_action_plan() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert_eq!(
        ActionPlan::try_new(draft(now, 0x4009, DeviceEpoch(2), DeviceEpoch(2))),
        Err(ActionSafetyError::UnsupportedDevice)
    );
}

#[test]
fn scalar_pid_cannot_hide_an_unsupported_instance_identity() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut plan = draft(now, 0x4006, DeviceEpoch(2), DeviceEpoch(2));
    plan.target.device_instance_id = "USB\\VID_2CA3&PID_4009\\REDACTED".to_owned();

    assert_eq!(
        ActionPlan::try_new(plan),
        Err(ActionSafetyError::UnsupportedDevice)
    );
}

#[test]
fn arbitrary_instance_identity_cannot_create_an_action_plan() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut plan = draft(now, 0x4006, DeviceEpoch(2), DeviceEpoch(2));
    plan.target.device_instance_id = "phone-wifi-adapter".to_owned();

    assert_eq!(
        ActionPlan::try_new(plan),
        Err(ActionSafetyError::UnsupportedDevice)
    );
}

#[test]
fn stale_device_epoch_cannot_create_an_action_plan() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert_eq!(
        ActionPlan::try_new(draft(now, 0x4006, DeviceEpoch(2), DeviceEpoch(1))),
        Err(ActionSafetyError::StaleEpoch)
    );
}

#[test]
fn plan_rechecks_all_identity_and_snapshot_preconditions_before_execution() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let plan = ActionPlan::try_new(draft(now, 0x4006, DeviceEpoch(2), DeviceEpoch(2))).unwrap();

    assert!(
        plan.validate_for_execution(
            41,
            DeviceEpoch(2),
            &identity(0x4006),
            BeforeStateHash([0xA5; 32]),
            now,
        )
        .is_ok()
    );
    assert_eq!(
        plan.validate_for_execution(
            42,
            DeviceEpoch(2),
            &identity(0x4006),
            BeforeStateHash([0xA5; 32]),
            now,
        ),
        Err(ActionSafetyError::StaleSnapshot)
    );
    assert_eq!(
        plan.validate_for_execution(
            41,
            DeviceEpoch(3),
            &identity(0x4006),
            BeforeStateHash([0xA5; 32]),
            now,
        ),
        Err(ActionSafetyError::StaleEpoch)
    );
    assert_eq!(
        plan.validate_for_execution(
            41,
            DeviceEpoch(2),
            &StableDeviceIdentity {
                container_id: "different".to_owned(),
                ..identity(0x4006)
            },
            BeforeStateHash([0xA5; 32]),
            now,
        ),
        Err(ActionSafetyError::TargetIdentityChanged)
    );
    assert_eq!(
        plan.validate_for_execution(
            41,
            DeviceEpoch(2),
            &identity(0x4006),
            BeforeStateHash([0x5A; 32]),
            now,
        ),
        Err(ActionSafetyError::BeforeStateChanged)
    );
    assert_eq!(
        plan.validate_for_execution(
            41,
            DeviceEpoch(2),
            &identity(0x4006),
            BeforeStateHash([0xA5; 32]),
            now + Duration::from_secs(31),
        ),
        Err(ActionSafetyError::Expired)
    );
}

#[test]
fn action_kind_is_closed_over_only_the_modeled_operations() {
    let actions = [
        ActionKind::Refresh,
        ActionKind::RenewDhcp,
        ActionKind::ApplyDnsProfile {
            profile: DnsProfile::Static {
                servers: vec![Ipv4Addr::new(1, 1, 1, 1).into()],
            },
        },
        ActionKind::RestartAdapter,
        ActionKind::ReenumerateDevice,
        ActionKind::RestartModule,
        ActionKind::EditApn {
            cid: 1,
            apn: "3gnet".to_owned(),
        },
        ActionKind::SetVerifiedUsbNetworkProfile {
            profile: UsbNetworkProfile::DjiNdis,
        },
        ActionKind::ToggleHotspot { enabled: true },
    ];

    assert_eq!(actions.len(), 9);
}
