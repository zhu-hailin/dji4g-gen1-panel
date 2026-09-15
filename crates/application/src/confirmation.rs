use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use dji4g_domain::{
    ActionKind, ActionPlan, ActionPlanDraft, ActionSafetyError, BeforeStateHash, DeviceEpoch,
    DeviceProfile, DisruptionLevel, DnsProfile, RiskLevel, StableDeviceIdentity, UsbNetworkProfile,
};

use dji4g_at_protocol::{Apn, PdpContextId, VerifiedUsbNetProfile};

/// Application-owned closed set of state-changing repair requests.
///
/// The UI and helper boundary use this type instead of exposing the domain's historical broad
/// action enum.  APNs and PDP identifiers are validated newtypes before they can become an
/// internal domain action.
#[derive(Clone, Eq, PartialEq)]
pub enum ControlledRepairRequest {
    RefreshDhcp,
    ApplyDnsProfile { profile: DnsProfile },
    RestartAdapter,
    ReenumerateDevice,
    SetUsbNetProfile { profile: VerifiedUsbNetProfile },
    SetApn { cid: PdpContextId, apn: Apn },
    RestartModule,
    ToggleHotspot { enabled: bool },
}

/// Stable conversion failures that never retain or print a sensitive APN/DNS payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlledRepairError {
    UnsupportedAction,
    InvalidPdpContextId,
    InvalidApn,
    InvalidDnsProfile,
}

impl fmt::Debug for ControlledRepairRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RefreshDhcp => formatter.write_str("RefreshDhcp"),
            Self::ApplyDnsProfile { profile } => formatter
                .debug_struct("ApplyDnsProfile")
                .field("profile", profile)
                .finish(),
            Self::RestartAdapter => formatter.write_str("RestartAdapter"),
            Self::ReenumerateDevice => formatter.write_str("ReenumerateDevice"),
            Self::SetUsbNetProfile { profile } => formatter
                .debug_struct("SetUsbNetProfile")
                .field("profile", profile)
                .finish(),
            Self::SetApn { cid, .. } => formatter
                .debug_struct("SetApn")
                .field("cid", cid)
                .field("apn", &"[REDACTED_APN]")
                .finish(),
            Self::RestartModule => formatter.write_str("RestartModule"),
            Self::ToggleHotspot { enabled } => formatter
                .debug_struct("ToggleHotspot")
                .field("enabled", enabled)
                .finish(),
        }
    }
}

impl ControlledRepairRequest {
    /// Build an APN request at the application boundary from an editor value.  The raw editor
    /// text is consumed immediately by the typed validators and is never placed in a snapshot.
    pub fn try_apn(cid: u8, apn: &str) -> Result<Self, ControlledRepairError> {
        let cid =
            PdpContextId::try_from(cid).map_err(|_| ControlledRepairError::InvalidPdpContextId)?;
        let apn = Apn::try_from(apn).map_err(|_| ControlledRepairError::InvalidApn)?;
        Ok(Self::SetApn { cid, apn })
    }

    #[must_use]
    pub fn into_action(self) -> ActionKind {
        match self {
            Self::RefreshDhcp => ActionKind::RenewDhcp,
            Self::ApplyDnsProfile { profile } => ActionKind::ApplyDnsProfile { profile },
            Self::RestartAdapter => ActionKind::RestartAdapter,
            Self::ReenumerateDevice => ActionKind::ReenumerateDevice,
            Self::SetUsbNetProfile { profile } => ActionKind::SetVerifiedUsbNetworkProfile {
                profile: match profile {
                    VerifiedUsbNetProfile::DjiNdis => UsbNetworkProfile::DjiNdis,
                    VerifiedUsbNetProfile::Ecm => UsbNetworkProfile::Ecm,
                },
            },
            Self::SetApn { cid, apn } => ActionKind::EditApn {
                cid: cid.get(),
                apn: apn.as_str().to_owned(),
            },
            Self::RestartModule => ActionKind::RestartModule,
            Self::ToggleHotspot { enabled } => ActionKind::ToggleHotspot { enabled },
        }
    }

    /// Convert an internal/domain action only when it is one of the controlled actions.
    pub fn try_from_action(action: ActionKind) -> Result<Self, ControlledRepairError> {
        match action {
            ActionKind::RenewDhcp => Ok(Self::RefreshDhcp),
            ActionKind::ApplyDnsProfile { profile } => valid_dns_profile(&profile)
                .then_some(Self::ApplyDnsProfile { profile })
                .ok_or(ControlledRepairError::InvalidDnsProfile),
            ActionKind::RestartAdapter => Ok(Self::RestartAdapter),
            ActionKind::ReenumerateDevice => Ok(Self::ReenumerateDevice),
            ActionKind::SetVerifiedUsbNetworkProfile { profile } => Ok(Self::SetUsbNetProfile {
                profile: match profile {
                    UsbNetworkProfile::DjiNdis => VerifiedUsbNetProfile::DjiNdis,
                    UsbNetworkProfile::Ecm => VerifiedUsbNetProfile::Ecm,
                },
            }),
            ActionKind::EditApn { cid, apn } => {
                let cid = PdpContextId::try_from(cid)
                    .map_err(|_| ControlledRepairError::InvalidPdpContextId)?;
                let apn = Apn::try_from(apn).map_err(|_| ControlledRepairError::InvalidApn)?;
                Ok(Self::SetApn { cid, apn })
            }
            ActionKind::RestartModule => Ok(Self::RestartModule),
            ActionKind::ToggleHotspot { enabled } => Ok(Self::ToggleHotspot { enabled }),
            ActionKind::Refresh => Err(ControlledRepairError::UnsupportedAction),
        }
    }
}

fn valid_dns_profile(profile: &DnsProfile) -> bool {
    match profile {
        DnsProfile::Automatic => true,
        DnsProfile::Static { servers } => {
            if servers.is_empty() || servers.len() > 3 {
                return false;
            }
            servers.iter().enumerate().all(|(index, address)| {
                !address.is_unspecified()
                    && !address.is_loopback()
                    && !address.is_multicast()
                    && !is_link_local(*address)
                    && !servers[..index].contains(address)
            })
        }
    }
}

fn is_link_local(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            let octets = address.octets();
            octets[0] == 169 && octets[1] == 254
        }
        std::net::IpAddr::V6(address) => (address.segments()[0] & 0xffc0) == 0xfe80,
    }
}

use crate::ReducerState;

pub type ActionRequest = ActionKind;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActionPlanId(u128);

impl ActionPlanId {
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }

    /// Deterministic constructor for external fixtures and tooling.  Production ids are minted
    /// only by [`next_plan_id`] and stay unique per process.
    #[must_use]
    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }
}

static NEXT_PLAN_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_plan_id() -> ActionPlanId {
    let low = NEXT_PLAN_ID.fetch_add(1, Ordering::Relaxed);
    ActionPlanId((u128::from(std::process::id()) << 64) | u128::from(low))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKindTag {
    RenewDhcp,
    ApplyDnsProfile,
    RestartAdapter,
    ReenumerateDevice,
    RestartModule,
    EditApn { cid: u8 },
    SetVerifiedUsbNetworkProfile,
    ToggleHotspot { enabled: bool },
}

impl ActionKindTag {
    #[must_use]
    pub fn from_action(action: &ActionKind) -> Option<Self> {
        match action {
            ActionKind::Refresh => None,
            ActionKind::RenewDhcp => Some(Self::RenewDhcp),
            ActionKind::ApplyDnsProfile { .. } => Some(Self::ApplyDnsProfile),
            ActionKind::RestartAdapter => Some(Self::RestartAdapter),
            ActionKind::ReenumerateDevice => Some(Self::ReenumerateDevice),
            ActionKind::RestartModule => Some(Self::RestartModule),
            ActionKind::EditApn { cid, .. } => Some(Self::EditApn { cid: *cid }),
            ActionKind::SetVerifiedUsbNetworkProfile { .. } => {
                Some(Self::SetVerifiedUsbNetworkProfile)
            }
            ActionKind::ToggleHotspot { enabled } => {
                Some(Self::ToggleHotspot { enabled: *enabled })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmationInvalidationReason {
    Expired,
    SnapshotChanged,
    EpochChanged,
    TargetChanged,
    BeforeStateChanged,
    DeviceRemoved,
    Superseded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedActionState {
    AwaitingConfirmation,
    Invalidated {
        reason: ConfirmationInvalidationReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedActionSnapshot {
    pub id: ActionPlanId,
    pub action: ActionKindTag,
    pub target_profile: DeviceProfile,
    pub based_on_revision: u64,
    pub expires_at: SystemTime,
    pub disruption: DisruptionLevel,
    pub risk: RiskLevel,
    pub requires_elevation: bool,
    pub state: PreparedActionState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationPhase {
    Revalidating,
    AwaitingElevation,
    Executing,
    Verifying,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationState {
    Running {
        phase: OperationPhase,
    },
    Finished {
        outcome: dji4g_domain::OperationOutcome,
        finished_at: SystemTime,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationUiSnapshot {
    pub operation_id: u64,
    pub action: ActionKindTag,
    pub started_at: SystemTime,
    pub state: OperationState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmResult {
    Executed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmError {
    UnknownPlan,
    AlreadyConsumed,
    Busy,
    Invalidated(ConfirmationInvalidationReason),
    RevalidationFailed(ConfirmationInvalidationReason),
    ExecutionFailed,
}

impl From<ActionSafetyError> for ConfirmationInvalidationReason {
    fn from(error: ActionSafetyError) -> Self {
        match error {
            ActionSafetyError::Expired => Self::Expired,
            ActionSafetyError::StaleSnapshot => Self::SnapshotChanged,
            ActionSafetyError::StaleEpoch => Self::EpochChanged,
            ActionSafetyError::TargetIdentityChanged | ActionSafetyError::UnsupportedDevice => {
                Self::TargetChanged
            }
            ActionSafetyError::BeforeStateChanged => Self::BeforeStateChanged,
        }
    }
}

/// A capability minted by the controller after a complete revalidation.
///
/// The fields are intentionally private: UI code can only send the opaque plan id and
/// cannot construct a token for an arbitrary target or command.
#[derive(Clone)]
pub struct ValidatedActionToken {
    pub(crate) plan_id: ActionPlanId,
    pub(crate) action: ActionKind,
    pub(crate) epoch: DeviceEpoch,
    pub(crate) target: StableDeviceIdentity,
    pub(crate) before_state_hash: BeforeStateHash,
}

impl fmt::Debug for ValidatedActionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedActionToken")
            .field("plan_id", &self.plan_id)
            .field("action", &ActionKindTag::from_action(&self.action))
            .field("epoch", &self.epoch)
            .finish()
    }
}

impl ValidatedActionToken {
    /// Read-only proof fields exposed to the platform composition layer.  Callers cannot
    /// construct or mutate a token; these accessors only allow a typed helper request to carry
    /// the controller's already-validated capability forward.
    #[must_use]
    pub fn action(&self) -> &ActionKind {
        &self.action
    }

    #[must_use]
    pub const fn epoch(&self) -> DeviceEpoch {
        self.epoch
    }

    #[must_use]
    pub fn target(&self) -> &StableDeviceIdentity {
        &self.target
    }

    #[must_use]
    pub const fn before_state_hash(&self) -> BeforeStateHash {
        self.before_state_hash
    }

    #[must_use]
    pub const fn plan_id(&self) -> ActionPlanId {
        self.plan_id
    }

    /// Derive the execution target from this already-validated token.
    ///
    /// Like [`crate::AdapterContext::target_context`], this reuses the crate-internal validated
    /// constructor so the composition layer can re-find the exact device before a privileged
    /// dispatch without ever building a target for an arbitrary, unsupported device.
    pub fn target_context(&self) -> Result<crate::ports::TargetContext, crate::ports::PortError> {
        crate::ports::TargetContext::new(self.epoch, self.target.clone(), None, None)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StoredActionPlan {
    pub id: ActionPlanId,
    pub plan: ActionPlan,
    pub summary: PreparedActionSnapshot,
}

pub(crate) fn mint_token(id: ActionPlanId, plan: &ActionPlan) -> ValidatedActionToken {
    ValidatedActionToken {
        plan_id: id,
        action: plan.kind.clone(),
        epoch: plan.epoch,
        target: plan.target.clone(),
        before_state_hash: plan.before_state_hash,
    }
}

pub(crate) fn action_metadata(
    action: &ActionKind,
) -> Option<(ActionKindTag, DisruptionLevel, RiskLevel, bool)> {
    Some(match action {
        ActionKind::Refresh => return None,
        // `IpRenewAddress` requires an administrator token, and every Windows-native repair
        // executes through the elevated helper, so the metadata must say so instead of telling
        // the user this repair needs no elevation and then failing with a permission error.
        ActionKind::RenewDhcp => (
            ActionKindTag::RenewDhcp,
            DisruptionLevel::Brief,
            RiskLevel::Low,
            true,
        ),
        ActionKind::ApplyDnsProfile { .. } => (
            ActionKindTag::ApplyDnsProfile,
            DisruptionLevel::Brief,
            RiskLevel::Medium,
            true,
        ),
        ActionKind::RestartAdapter => (
            ActionKindTag::RestartAdapter,
            DisruptionLevel::ConnectionInterrupting,
            RiskLevel::Medium,
            true,
        ),
        ActionKind::ReenumerateDevice => (
            ActionKindTag::ReenumerateDevice,
            DisruptionLevel::DeviceReenumeration,
            RiskLevel::High,
            true,
        ),
        ActionKind::RestartModule => (
            ActionKindTag::RestartModule,
            DisruptionLevel::DeviceReenumeration,
            RiskLevel::High,
            true,
        ),
        ActionKind::EditApn { cid, .. } => (
            ActionKindTag::EditApn { cid: *cid },
            DisruptionLevel::ConnectionInterrupting,
            RiskLevel::High,
            true,
        ),
        ActionKind::SetVerifiedUsbNetworkProfile { .. } => (
            ActionKindTag::SetVerifiedUsbNetworkProfile,
            DisruptionLevel::DeviceReenumeration,
            RiskLevel::High,
            true,
        ),
        ActionKind::ToggleHotspot { enabled } => (
            ActionKindTag::ToggleHotspot { enabled: *enabled },
            DisruptionLevel::Brief,
            RiskLevel::Medium,
            false,
        ),
    })
}

/// The disruption level of a modeled action, exposed for UI grouping. `None` means the action
/// is not a confirmable operation (`Refresh`).
#[must_use]
pub fn action_disruption(action: &ActionKind) -> Option<DisruptionLevel> {
    action_metadata(action).map(|(_, disruption, _, _)| disruption)
}

/// The risk level of a modeled action, exposed so the confirmation message can be composed at
/// click time. `None` means the action is not a confirmable operation (`Refresh`).
#[must_use]
pub fn action_risk(action: &ActionKind) -> Option<RiskLevel> {
    action_metadata(action).map(|(_, _, risk, _)| risk)
}

/// Whether executing the action requires an elevated helper, exposed so the confirmation message
/// can be composed at click time. `Refresh` reports `false` (it is not a confirmable operation
/// and never executes).
#[must_use]
pub fn action_requires_elevation(action: &ActionKind) -> bool {
    action_metadata(action).is_some_and(|(_, _, _, requires_elevation)| requires_elevation)
}

pub(crate) fn build_plan(
    id: ActionPlanId,
    action: ActionKind,
    state: &ReducerState,
    target: StableDeviceIdentity,
    before_state_hash: BeforeStateHash,
    now: SystemTime,
) -> Result<StoredActionPlan, crate::PrepareError> {
    let Some((tag, disruption, risk, requires_elevation)) = action_metadata(&action) else {
        return Err(crate::PrepareError::UnsupportedAction);
    };
    let draft = ActionPlanDraft {
        kind: action,
        snapshot_revision: state.evidence_revision(),
        current_epoch: state.epoch(),
        evidence_epoch: state.epoch(),
        target,
        before_state_hash,
        expires_at: now + crate::PLAN_LIFETIME,
        disruption,
        risk,
        requires_elevation,
    };
    let plan = ActionPlan::try_new(draft).map_err(crate::PrepareError::Safety)?;
    Ok(StoredActionPlan {
        id,
        summary: PreparedActionSnapshot {
            id,
            action: tag,
            target_profile: DeviceProfile::DJI_GEN1,
            based_on_revision: plan.snapshot_revision,
            expires_at: plan.expires_at,
            disruption,
            risk,
            requires_elevation,
            state: PreparedActionState::AwaitingConfirmation,
        },
        plan,
    })
}

pub(crate) fn before_state_hash(
    state: &ReducerState,
    action: &ActionKind,
) -> Option<BeforeStateHash> {
    let target = state.target_identity()?;
    let material = BeforeStateMaterialV1::from_state(state, action, target);
    Some(BeforeStateHash(sha256(&material.encode())))
}

#[derive(Clone, Debug)]
struct BeforeStateMaterialV1 {
    action_tag: ActionKindTag,
    epoch: DeviceEpoch,
    target: StableDeviceIdentity,
    adapter_id: Option<String>,
    network: Option<dji4g_domain::NetworkSnapshot>,
    hotspot: dji4g_domain::HotspotStatus,
}

impl BeforeStateMaterialV1 {
    fn from_state(state: &ReducerState, action: &ActionKind, target: StableDeviceIdentity) -> Self {
        Self {
            action_tag: ActionKindTag::from_action(action)
                .expect("refresh rejected before hashing"),
            epoch: state.epoch(),
            target,
            adapter_id: state.adapter_id(),
            network: state.network_snapshot(),
            hotspot: state.hotspot_status(),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(512);
        output.extend_from_slice(b"dji4g-before-state-v1\0");
        put_u8(&mut output, action_tag_code(self.action_tag));
        if let ActionKindTag::EditApn { cid } = self.action_tag {
            put_u8(&mut output, cid);
        }
        if let ActionKindTag::ToggleHotspot { enabled } = self.action_tag {
            put_u8(&mut output, u8::from(enabled));
        }
        output.extend_from_slice(&self.epoch.0.to_le_bytes());
        put_str(&mut output, &self.target.container_id);
        put_str(&mut output, &self.target.device_instance_id);
        output.extend_from_slice(&self.target.vid.to_le_bytes());
        output.extend_from_slice(&self.target.pid.to_le_bytes());
        put_optional_str(&mut output, self.adapter_id.as_deref());
        if let Some(network) = &self.network {
            put_str(&mut output, &network.adapter_id);
            put_strs(&mut output, &network.addresses);
            put_strs(&mut output, &network.gateways);
            put_strs(&mut output, &network.dns_servers);
            put_u8(&mut output, bound_public_code(network.bound_public));
            put_u8(&mut output, bound_dns_code(network.bound_dns));
            put_u8(
                &mut output,
                protocol_coverage_code(network.protocol_coverage),
            );
            put_u8(&mut output, route_owner_code(network.system_default_route));
        } else {
            put_u8(&mut output, 0xff);
        }
        put_u8(&mut output, hotspot_code(self.hotspot));
        output
    }
}

fn action_tag_code(tag: ActionKindTag) -> u8 {
    match tag {
        ActionKindTag::RenewDhcp => 1,
        ActionKindTag::ApplyDnsProfile => 2,
        ActionKindTag::RestartAdapter => 3,
        ActionKindTag::ReenumerateDevice => 4,
        ActionKindTag::RestartModule => 5,
        ActionKindTag::EditApn { .. } => 6,
        ActionKindTag::SetVerifiedUsbNetworkProfile => 7,
        ActionKindTag::ToggleHotspot { .. } => 8,
    }
}

fn hotspot_code(status: dji4g_domain::HotspotStatus) -> u8 {
    match status {
        dji4g_domain::HotspotStatus::Unsupported(_) => 0,
        dji4g_domain::HotspotStatus::Off => 1,
        dji4g_domain::HotspotStatus::Starting => 2,
        dji4g_domain::HotspotStatus::On { .. } => 3,
        dji4g_domain::HotspotStatus::Stopping => 4,
        dji4g_domain::HotspotStatus::Failed { .. } => 5,
    }
}

fn bound_public_code(status: dji4g_domain::BoundPublicStatus) -> u8 {
    match status {
        dji4g_domain::BoundPublicStatus::Succeeded => 1,
        dji4g_domain::BoundPublicStatus::Failed { consecutive_cycles } => {
            2_u8.saturating_add(consecutive_cycles)
        }
        dji4g_domain::BoundPublicStatus::Incomplete => 0,
    }
}

fn bound_dns_code(status: dji4g_domain::BoundDnsStatus) -> u8 {
    match status {
        dji4g_domain::BoundDnsStatus::Succeeded => 1,
        dji4g_domain::BoundDnsStatus::Failed => 2,
        dji4g_domain::BoundDnsStatus::Incomplete => 0,
    }
}

fn protocol_coverage_code(status: dji4g_domain::ProtocolCoverage) -> u8 {
    match status {
        dji4g_domain::ProtocolCoverage::AllRequiredFamilies => 1,
        dji4g_domain::ProtocolCoverage::SingleFamilyOnly => 2,
    }
}

fn route_owner_code(status: dji4g_domain::DefaultRouteOwner) -> u8 {
    match status {
        dji4g_domain::DefaultRouteOwner::TargetAdapter => 1,
        dji4g_domain::DefaultRouteOwner::VpnOrTun => 2,
        dji4g_domain::DefaultRouteOwner::Other => 3,
    }
}

fn put_u8(output: &mut Vec<u8>, value: u8) {
    output.push(value);
}
fn put_str(output: &mut Vec<u8>, value: &str) {
    let length = u32::try_from(value.len()).unwrap_or(u32::MAX);
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}
fn put_optional_str(output: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            put_u8(output, 1);
            put_str(output, value);
        }
        None => put_u8(output, 0),
    }
}
fn put_strs(output: &mut Vec<u8>, values: &[String]) {
    let length = u32::try_from(values.len()).unwrap_or(u32::MAX);
    output.extend_from_slice(&length.to_le_bytes());
    for value in values {
        put_str(output, value);
    }
}

// Minimal dependency-free SHA-256 implementation. It is used only for canonical, local
// precondition material and avoids bringing a crypto crate into the application layer.
fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = input.to_vec();
    let bit_len = (data.len() as u64).saturating_mul(8);
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    let mut h = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (i, word) in words[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = words[i - 15].rotate_right(7)
                ^ words[i - 15].rotate_right(18)
                ^ (words[i - 15] >> 3);
            let s1 = words[i - 2].rotate_right(17)
                ^ words[i - 2].rotate_right(19)
                ^ (words[i - 2] >> 10);
            words[i] = words[i - 16]
                .wrapping_add(s0)
                .wrapping_add(words[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(words[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (value, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *value = value.wrapping_add(add);
        }
    }
    let mut output = [0_u8; 32];
    for (i, word) in h.into_iter().enumerate() {
        output[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}
