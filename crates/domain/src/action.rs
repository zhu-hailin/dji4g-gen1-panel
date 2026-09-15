use std::{net::IpAddr, time::SystemTime};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{DeviceEpoch, ErrorCode, StableDeviceIdentity};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DnsProfile {
    Automatic,
    Static { servers: Vec<IpAddr> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UsbNetworkProfile {
    DjiNdis,
    Ecm,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActionKind {
    Refresh,
    RenewDhcp,
    ApplyDnsProfile { profile: DnsProfile },
    RestartAdapter,
    ReenumerateDevice,
    RestartModule,
    EditApn { cid: u8, apn: String },
    SetVerifiedUsbNetworkProfile { profile: UsbNetworkProfile },
    ToggleHotspot { enabled: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DisruptionLevel {
    None,
    Brief,
    ConnectionInterrupting,
    DeviceReenumeration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BeforeStateHash(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AfterStateHash(pub [u8; 32]);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionPlanDraft {
    pub kind: ActionKind,
    pub snapshot_revision: u64,
    pub current_epoch: DeviceEpoch,
    pub evidence_epoch: DeviceEpoch,
    pub target: StableDeviceIdentity,
    pub before_state_hash: BeforeStateHash,
    pub expires_at: SystemTime,
    pub disruption: DisruptionLevel,
    pub risk: RiskLevel,
    pub requires_elevation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionPlan {
    pub kind: ActionKind,
    pub snapshot_revision: u64,
    pub epoch: DeviceEpoch,
    pub target: StableDeviceIdentity,
    pub before_state_hash: BeforeStateHash,
    pub expires_at: SystemTime,
    pub disruption: DisruptionLevel,
    pub risk: RiskLevel,
    pub requires_elevation: bool,
}

impl ActionPlan {
    pub fn try_new(draft: ActionPlanDraft) -> Result<Self, ActionSafetyError> {
        if !draft.target.is_supported() {
            return Err(ActionSafetyError::UnsupportedDevice);
        }
        if draft.current_epoch != draft.evidence_epoch {
            return Err(ActionSafetyError::StaleEpoch);
        }

        Ok(Self {
            kind: draft.kind,
            snapshot_revision: draft.snapshot_revision,
            epoch: draft.current_epoch,
            target: draft.target,
            before_state_hash: draft.before_state_hash,
            expires_at: draft.expires_at,
            disruption: draft.disruption,
            risk: draft.risk,
            requires_elevation: draft.requires_elevation,
        })
    }

    pub fn validate_for_execution(
        &self,
        snapshot_revision: u64,
        epoch: DeviceEpoch,
        target: &StableDeviceIdentity,
        before_state_hash: BeforeStateHash,
        now: SystemTime,
    ) -> Result<(), ActionSafetyError> {
        if !target.is_supported() {
            return Err(ActionSafetyError::UnsupportedDevice);
        }
        if now > self.expires_at {
            return Err(ActionSafetyError::Expired);
        }
        if snapshot_revision != self.snapshot_revision {
            return Err(ActionSafetyError::StaleSnapshot);
        }
        if epoch != self.epoch {
            return Err(ActionSafetyError::StaleEpoch);
        }
        if target != &self.target {
            return Err(ActionSafetyError::TargetIdentityChanged);
        }
        if before_state_hash != self.before_state_hash {
            return Err(ActionSafetyError::BeforeStateChanged);
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq, Serialize, Deserialize)]
pub enum ActionSafetyError {
    #[error("the target is not the supported first-generation device")]
    UnsupportedDevice,
    #[error("the device epoch changed")]
    StaleEpoch,
    #[error("the snapshot revision changed")]
    StaleSnapshot,
    #[error("the stable target identity changed")]
    TargetIdentityChanged,
    #[error("the before-state hash changed")]
    BeforeStateChanged,
    #[error("the action plan expired")]
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RollbackOutcome {
    NotRequired,
    Applied,
    Failed { code: ErrorCode },
    NotAttempted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OperationOutcome {
    Applied {
        after_state_hash: AfterStateHash,
    },
    Failed {
        code: ErrorCode,
        rollback: RollbackOutcome,
    },
    OutcomeUnknown {
        code: ErrorCode,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationSnapshot {
    pub kind: ActionKind,
    pub started_at: SystemTime,
}
