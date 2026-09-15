//! Closed, confirmation-first repair execution for the first-generation DJI module.
//!
//! The production backend in this module is deliberately narrow.  A backend receives only a
//! typed [`RepairAction`] together with an inventory-owned observation; it never receives a COM
//! number, FriendlyName, interface path, GUID, shell command, or raw AT string.  The fake backend
//! is used by deterministic tests and has the same boundary as the native implementation.

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use dji4g_at_protocol::{Apn, PdpContextId, VerifiedUsbNetProfile};
use dji4g_domain::{
    AfterStateHash, BeforeStateHash, DJI_GEN1, DeviceEpoch, DeviceProfile, DnsProfile, ErrorCode,
    OperationOutcome, RollbackOutcome,
};

const IDENTITY_MATERIAL_VERSION: &[u8] = b"dji4g-repair-identity-v1\0";
const BEFORE_MATERIAL_VERSION: &[u8] = b"dji4g-repair-before-v1\0";
const AUTHORITATIVE_IDENTITY_MATERIAL_VERSION: &[u8] = b"dji4g-identity-v1\0";

/// Compute the shared stable identity proof used by both the helper protocol and native repair.
///
/// The material is intentionally limited to the inventory-proven root instance and optional
/// container identity.  No interface path, COM number, display name, or raw hardware value is
/// accepted at this boundary.
#[cfg(windows)]
pub fn authoritative_identity_hash(device: &crate::DjiDevice) -> [u8; 32] {
    let mut material = Vec::with_capacity(96);
    material.extend_from_slice(AUTHORITATIVE_IDENTITY_MATERIAL_VERSION);
    material.extend_from_slice(device.root_instance_id().as_bytes());
    material.push(0);
    if let Some(container) = device.container_id() {
        material.extend_from_slice(container.as_bytes());
    }
    sha256(&material)
}

/// The only controlled repair actions admitted by Task 11.
///
/// `RefreshDhcp` is the panel's user-facing "refresh" operation.  A read-only inventory refresh
/// remains a controller concern and is deliberately not represented here.  All other variants
/// carry the smallest typed value required by the reviewed native boundary.
#[derive(Clone, Eq, PartialEq)]
pub enum RepairAction {
    RefreshDhcp,
    ApplyDnsProfile { profile: DnsProfile },
    RestartAdapter,
    ReenumerateDevice,
    SetUsbNetProfile { profile: VerifiedUsbNetProfile },
    SetApn { cid: PdpContextId, apn: Apn },
    RestartModule,
    ToggleHotspot { enabled: bool },
}

impl fmt::Debug for RepairAction {
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

impl RepairAction {
    #[must_use]
    pub const fn requires_at(&self) -> bool {
        matches!(
            self,
            Self::SetUsbNetProfile { .. } | Self::SetApn { .. } | Self::RestartModule
        )
    }

    #[must_use]
    pub const fn requires_adapter(&self) -> bool {
        matches!(
            self,
            Self::RefreshDhcp
                | Self::ApplyDnsProfile { .. }
                | Self::RestartAdapter
                | Self::ToggleHotspot { .. }
        )
    }
}

/// A proof of the exact VID/PID root selected by the inventory scanner.
#[derive(Clone, Eq, PartialEq)]
pub struct TargetProof {
    profile: DeviceProfile,
    identity_hash: [u8; 32],
}

impl TargetProof {
    /// Construct a fixture proof for the only supported device profile.
    #[must_use]
    pub const fn dji_gen1(identity_hash: [u8; 32]) -> Self {
        Self {
            profile: DJI_GEN1,
            identity_hash,
        }
    }

    /// Construct a proof for failure-injection tests.  Production inventory code should only
    /// produce [`Self::dji_gen1`] after exact VID/PID and ancestry checks.
    #[must_use]
    pub const fn for_profile(profile: DeviceProfile, identity_hash: [u8; 32]) -> Self {
        Self {
            profile,
            identity_hash,
        }
    }

    #[must_use]
    pub const fn profile(&self) -> DeviceProfile {
        self.profile
    }

    #[must_use]
    pub const fn identity_hash(&self) -> [u8; 32] {
        self.identity_hash
    }

    #[must_use]
    pub const fn is_supported(&self) -> bool {
        self.profile.matches(DJI_GEN1.vid, DJI_GEN1.pid)
    }
}

impl fmt::Debug for TargetProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TargetProof")
            .field("profile", &self.profile)
            .field("identity_hash", &"[REDACTED_HASH]")
            .finish()
    }
}

/// A proof produced by the authoritative network-adapter resolver.
#[derive(Clone, Eq, PartialEq)]
pub struct AdapterProof {
    identity_hash: [u8; 32],
}

impl AdapterProof {
    #[must_use]
    pub const fn fixture(identity_hash: [u8; 32]) -> Self {
        Self { identity_hash }
    }

    #[must_use]
    pub const fn identity_hash(&self) -> [u8; 32] {
        self.identity_hash
    }
}

impl fmt::Debug for AdapterProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdapterProof")
            .field("identity_hash", &"[REDACTED_HASH]")
            .finish()
    }
}

/// A capability proof for the exact WinRT source profile selected by Task 9.
///
/// The two digests are deliberately opaque.  Native code creates this value only after matching
/// a fresh `ConnectionProfile.NetworkAdapter.NetworkAdapterId` to the resolver-owned adapter
/// GUID and checking the runtime tethering capability.  The repair executor never accepts a
/// profile name, ordinal, or global/default connection profile.
#[derive(Clone, Eq, PartialEq)]
pub struct HotspotProof {
    source_profile_hash: [u8; 32],
    capability_hash: [u8; 32],
}

impl HotspotProof {
    #[must_use]
    pub const fn fixture(source_profile_hash: [u8; 32], capability_hash: [u8; 32]) -> Self {
        Self {
            source_profile_hash,
            capability_hash,
        }
    }

    #[must_use]
    pub const fn source_profile_hash(&self) -> [u8; 32] {
        self.source_profile_hash
    }

    #[must_use]
    pub const fn capability_hash(&self) -> [u8; 32] {
        self.capability_hash
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.source_profile_hash != [0; 32] && self.capability_hash != [0; 32]
    }
}

impl fmt::Debug for HotspotProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HotspotProof")
            .field("source_profile_hash", &"[REDACTED_HASH]")
            .field("capability_hash", &"[REDACTED_HASH]")
            .finish()
    }
}

/// A typed PDP context used by the fake backend and by native readback adapters.
#[derive(Clone, Eq, PartialEq)]
pub struct RepairPdpContext {
    cid: PdpContextId,
    pdp_type: Option<PdpTypeForRepair>,
    apn: Option<Apn>,
    active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdpTypeForRepair {
    Ip,
    Ipv6,
    Ipv4v6,
}

impl RepairPdpContext {
    #[must_use]
    pub const fn cid(&self) -> PdpContextId {
        self.cid
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    #[must_use]
    pub fn apn(&self) -> Option<&Apn> {
        self.apn.as_ref()
    }
}

impl fmt::Debug for RepairPdpContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepairPdpContext")
            .field("cid", &self.cid)
            .field("pdp_type", &self.pdp_type)
            .field("apn", &"[REDACTED_APN]")
            .field("active", &self.active)
            .finish()
    }
}

/// Fresh, action-independent facts collected during revalidation.
#[derive(Clone, Eq, PartialEq)]
pub struct RepairObservation {
    pub revision: u64,
    pub epoch: DeviceEpoch,
    pub target: TargetProof,
    /// Number of proven roots in the fresh inventory snapshot.  Exactly one is required.
    pub target_count: usize,
    pub adapter: AdapterProof,
    pub adapter_up: bool,
    pub reenumerated: bool,
    at_binding_hash: Option<[u8; 32]>,
    usb_profile: Option<VerifiedUsbNetProfile>,
    contexts: Vec<RepairPdpContext>,
    dns_profile: Option<DnsProfile>,
    hotspot: Option<HotspotProof>,
    hotspot_enabled: bool,
}

impl RepairObservation {
    #[must_use]
    pub fn fixture(
        epoch: DeviceEpoch,
        revision: u64,
        target: TargetProof,
        adapter: AdapterProof,
    ) -> Self {
        Self {
            revision,
            epoch,
            target,
            target_count: 1,
            adapter,
            adapter_up: true,
            reenumerated: true,
            at_binding_hash: Some([0x55; 32]),
            usb_profile: Some(VerifiedUsbNetProfile::DjiNdis),
            contexts: Vec::new(),
            dns_profile: Some(DnsProfile::Automatic),
            hotspot: Some(HotspotProof::fixture([0x33; 32], [0x44; 32])),
            hotspot_enabled: false,
        }
    }

    #[must_use]
    pub fn usb_profile(&self) -> Option<VerifiedUsbNetProfile> {
        self.usb_profile
    }

    #[must_use]
    pub const fn at_binding_hash(&self) -> Option<[u8; 32]> {
        self.at_binding_hash
    }

    #[must_use]
    pub fn contexts(&self) -> &[RepairPdpContext] {
        &self.contexts
    }

    #[must_use]
    pub fn dns_profile(&self) -> Option<&DnsProfile> {
        self.dns_profile.as_ref()
    }

    #[must_use]
    pub const fn hotspot(&self) -> Option<&HotspotProof> {
        self.hotspot.as_ref()
    }

    #[must_use]
    pub const fn hotspot_enabled(&self) -> bool {
        self.hotspot_enabled
    }
}

impl fmt::Debug for RepairObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepairObservation")
            .field("revision", &self.revision)
            .field("epoch", &self.epoch)
            .field("target", &self.target)
            .field("adapter", &self.adapter)
            .field("adapter_up", &self.adapter_up)
            .field("reenumerated", &self.reenumerated)
            .field(
                "at_binding_hash",
                &self.at_binding_hash.map(|_| "[REDACTED_HASH]"),
            )
            .field("usb_profile", &self.usb_profile)
            .field("contexts", &self.contexts)
            .field("dns_profile", &self.dns_profile)
            .field("hotspot", &self.hotspot)
            .field("hotspot_enabled", &self.hotspot_enabled)
            .finish()
    }
}

/// Set fake PDP observations without exposing APN values through diagnostics.
impl RepairObservation {
    pub fn set_contexts(&mut self, contexts: &[(u8, &str, &str, bool)]) {
        self.contexts = contexts
            .iter()
            .map(|(cid, pdp_type, apn, active)| RepairPdpContext {
                cid: PdpContextId::try_from(*cid)
                    .unwrap_or_else(|_| PdpContextId::try_from(1).expect("valid fallback CID")),
                pdp_type: match *pdp_type {
                    "IP" => Some(PdpTypeForRepair::Ip),
                    "IPV6" => Some(PdpTypeForRepair::Ipv6),
                    "IPV4V6" => Some(PdpTypeForRepair::Ipv4v6),
                    _ => None,
                },
                apn: Apn::try_from(*apn).ok(),
                active: *active,
            })
            .collect();
    }

    pub fn set_dns_profile(&mut self, profile: Option<DnsProfile>) {
        self.dns_profile = profile;
    }

    pub fn set_hotspot(&mut self, proof: Option<HotspotProof>, enabled: bool) {
        self.hotspot = proof;
        self.hotspot_enabled = enabled;
    }
}

/// Scriptable outcomes for a single mutation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchResult {
    Applied,
    /// Alias used by callers that describe the native call as completed.
    Completed,
    /// Alias used by callers that describe the native call as successful.
    Success,
    /// A native operation completed with a definite primary failure and a recorded best-effort
    /// recovery result (used by adapter disable/enable and DNS restoration paths).
    Failed {
        code: ErrorCode,
        rollback: RollbackOutcome,
    },
    Rejected(ErrorCode),
    Unknown(ErrorCode),
}

impl DispatchResult {
    #[must_use]
    const fn is_applied(self) -> bool {
        matches!(self, Self::Applied | Self::Completed | Self::Success)
    }
}

/// The narrow platform capability consumed by the executor.
pub trait RepairBackend: Send + Sync {
    fn observe(&self) -> Result<RepairObservation, RepairError>;

    /// Dispatch exactly one typed action.  A backend must never retry after returning
    /// [`DispatchResult::Unknown`].
    fn dispatch(&self, action: &RepairAction) -> DispatchResult;

    /// Dispatch against the fresh proof that was used to prepare the one-shot plan.  Native
    /// backends override this to perform one more exact inventory check immediately before a
    /// state change; fakes retain the simpler action-only seam.
    fn dispatch_verified(
        &self,
        action: &RepairAction,
        _before: &RepairObservation,
    ) -> DispatchResult {
        self.dispatch(action)
    }

    /// Read fresh facts after a mutation.  This is intentionally separate from `observe` so a
    /// helper/native implementation cannot accidentally reuse the before snapshot.
    fn observe_after(&self, _action: &RepairAction) -> Result<RepairObservation, RepairError> {
        self.observe()
    }
}

/// Stable preparation/execution failures.  No variant stores raw native text or APN material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairError {
    Unsupported,
    UnsupportedPlatform,
    TargetNotFound,
    TargetAmbiguous,
    PermissionDenied,
    TargetIdentityChanged,
    EpochChanged,
    EvidenceExpired,
    BeforeStateChanged,
    PdpContextMissing,
    PdpContextAmbiguous,
    PdpContextIncomplete,
    PdpContextActive,
    InvalidApn,
    InvalidUsbProfile,
    DnsStateUnavailable,
    InvalidDnsProfile,
    HotspotUnavailable,
    HotspotStateUnknown,
    ReenumerationFailed,
    NoChange,
    VerificationFailed,
    DeviceRemoved,
    Timeout,
    PeerDisconnected,
    OperationCancelled,
    AlreadyConsumed,
    Internal,
}

impl RepairError {
    #[must_use]
    pub const fn code(self) -> ErrorCode {
        match self {
            Self::Unsupported
            | Self::UnsupportedPlatform
            | Self::InvalidUsbProfile
            | Self::InvalidDnsProfile
            | Self::HotspotUnavailable => ErrorCode::Unsupported,
            Self::TargetNotFound | Self::DeviceRemoved => ErrorCode::DeviceRemoved,
            Self::TargetAmbiguous => ErrorCode::CapabilityUnavailable,
            Self::PermissionDenied => ErrorCode::PermissionDenied,
            Self::TargetIdentityChanged | Self::EpochChanged => ErrorCode::DeviceIdentityChanged,
            Self::EvidenceExpired | Self::BeforeStateChanged | Self::AlreadyConsumed => {
                ErrorCode::EvidenceExpired
            }
            Self::PdpContextMissing
            | Self::PdpContextAmbiguous
            | Self::PdpContextIncomplete
            | Self::PdpContextActive
            | Self::InvalidApn
            | Self::DnsStateUnavailable
            | Self::HotspotStateUnknown
            | Self::ReenumerationFailed
            | Self::NoChange
            | Self::VerificationFailed => ErrorCode::VerificationFailed,
            Self::Timeout => ErrorCode::Timeout,
            Self::PeerDisconnected => ErrorCode::DeviceRemoved,
            Self::OperationCancelled => ErrorCode::OperationCancelled,
            Self::Internal => ErrorCode::Internal,
        }
    }
}

impl fmt::Display for RepairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unsupported => "repair:unsupported",
            Self::UnsupportedPlatform => "repair:unsupported_platform",
            Self::TargetNotFound => "repair:target_not_found",
            Self::TargetAmbiguous => "repair:target_ambiguous",
            Self::PermissionDenied => "repair:permission_denied",
            Self::TargetIdentityChanged => "repair:target_identity_changed",
            Self::EpochChanged => "repair:epoch_changed",
            Self::EvidenceExpired => "repair:evidence_expired",
            Self::BeforeStateChanged => "repair:before_state_changed",
            Self::PdpContextMissing => "repair:pdp_context_missing",
            Self::PdpContextAmbiguous => "repair:pdp_context_ambiguous",
            Self::PdpContextIncomplete => "repair:pdp_context_incomplete",
            Self::PdpContextActive => "repair:pdp_context_active",
            Self::InvalidApn => "repair:invalid_apn",
            Self::InvalidUsbProfile => "repair:invalid_usb_profile",
            Self::DnsStateUnavailable => "repair:dns_state_unavailable",
            Self::InvalidDnsProfile => "repair:invalid_dns_profile",
            Self::HotspotUnavailable => "repair:hotspot_unavailable",
            Self::HotspotStateUnknown => "repair:hotspot_state_unknown",
            Self::ReenumerationFailed => "repair:reenumeration_failed",
            Self::NoChange => "repair:no_change",
            Self::VerificationFailed => "repair:verification_failed",
            Self::DeviceRemoved => "repair:device_removed",
            Self::Timeout => "repair:timeout",
            Self::PeerDisconnected => "repair:peer_disconnected",
            Self::OperationCancelled => "repair:operation_cancelled",
            Self::AlreadyConsumed => "repair:already_consumed",
            Self::Internal => "repair:internal",
        })
    }
}

impl std::error::Error for RepairError {}

/// An opaque, one-shot plan made from a fresh backend observation.
pub struct RepairPlan {
    action: RepairAction,
    revision: u64,
    epoch: DeviceEpoch,
    target: TargetProof,
    before_state_hash: BeforeStateHash,
    consumed: AtomicBool,
}

impl fmt::Debug for RepairPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepairPlan")
            .field("action", &self.action)
            .field("revision", &self.revision)
            .field("epoch", &self.epoch)
            .field("target", &self.target)
            .field("before_state_hash", &"[REDACTED_HASH]")
            .finish()
    }
}

impl RepairPlan {
    #[must_use]
    pub const fn epoch(&self) -> DeviceEpoch {
        self.epoch
    }

    #[must_use]
    pub const fn target_identity_hash(&self) -> [u8; 32] {
        self.target.identity_hash()
    }

    #[must_use]
    pub const fn before_state_hash(&self) -> BeforeStateHash {
        self.before_state_hash
    }

    #[must_use]
    pub const fn action(&self) -> &RepairAction {
        &self.action
    }
}

/// Execution result, retaining hashes for machine-readable evidence only.
#[derive(Clone, Eq, PartialEq)]
pub struct RepairResult {
    outcome: OperationOutcome,
    before_state_hash: BeforeStateHash,
    after_state_hash: Option<AfterStateHash>,
}

impl fmt::Debug for RepairResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepairResult")
            .field("outcome", &redacted_outcome(&self.outcome))
            .field("before_state_hash", &"[REDACTED_HASH]")
            .field(
                "after_state_hash",
                &self.after_state_hash.map(|_| "[REDACTED_HASH]"),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedactedOutcome {
    Applied,
    Failed {
        code: ErrorCode,
        rollback: RollbackOutcome,
    },
    OutcomeUnknown {
        code: ErrorCode,
    },
}

fn redacted_outcome(outcome: &OperationOutcome) -> RedactedOutcome {
    match outcome {
        OperationOutcome::Applied { .. } => RedactedOutcome::Applied,
        OperationOutcome::Failed { code, rollback } => RedactedOutcome::Failed {
            code: *code,
            rollback: *rollback,
        },
        OperationOutcome::OutcomeUnknown { code } => {
            RedactedOutcome::OutcomeUnknown { code: *code }
        }
    }
}

impl RepairResult {
    #[must_use]
    pub fn outcome(&self) -> &OperationOutcome {
        &self.outcome
    }

    #[must_use]
    pub const fn before_state_hash(&self) -> BeforeStateHash {
        self.before_state_hash
    }

    #[must_use]
    pub const fn after_state_hash(&self) -> Option<AfterStateHash> {
        self.after_state_hash
    }
}

/// Revalidates and executes a plan against exactly one backend mutation boundary.
pub struct WindowsRepairExecutor<B> {
    backend: B,
}

impl<B: RepairBackend> WindowsRepairExecutor<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn replace_backend(&mut self, backend: B) {
        self.backend = backend;
    }

    pub fn prepare(&self, action: RepairAction) -> Result<RepairPlan, RepairError> {
        let observation = self.backend.observe()?;
        validate_observation(&observation)?;
        validate_action(&observation, &action)?;
        let state_hash = before_hash(&observation, &action);
        Ok(RepairPlan {
            action: action.clone(),
            revision: observation.revision,
            epoch: observation.epoch,
            target: observation.target.clone(),
            before_state_hash: state_hash,
            consumed: AtomicBool::new(false),
        })
    }

    /// Alias for consumers that call the preparation step revalidation.
    pub fn revalidate(&self, action: RepairAction) -> Result<RepairPlan, RepairError> {
        self.prepare(action)
    }

    pub fn execute(&self, plan: &RepairPlan) -> RepairResult {
        if plan.consumed.swap(true, Ordering::AcqRel) {
            return failed_result(plan.before_state_hash, RepairError::AlreadyConsumed.code());
        }

        let current = match self.backend.observe() {
            Ok(value) => value,
            Err(error) => return failed_or_unknown(plan.before_state_hash, error, false),
        };
        if let Err(error) = validate_observation(&current)
            .and_then(|()| validate_plan_observation(plan, &current))
            .and_then(|()| validate_action(&current, &plan.action))
        {
            return failed_result(plan.before_state_hash, error.code());
        }

        let dispatch = self.backend.dispatch_verified(&plan.action, &current);
        match dispatch {
            DispatchResult::Rejected(code) => {
                return failed_result(plan.before_state_hash, code);
            }
            DispatchResult::Failed { code, rollback } => {
                match self.backend.observe_after(&plan.action) {
                    Ok(value) => {
                        let _ = verify_after(&value, &plan.action);
                    }
                    Err(_) => return unknown_result(plan.before_state_hash, code),
                }
                return failed_result_with_rollback(plan.before_state_hash, code, rollback);
            }
            DispatchResult::Applied
            | DispatchResult::Completed
            | DispatchResult::Success
            | DispatchResult::Unknown(_) => {}
        }
        let dispatch_applied = dispatch.is_applied();

        let after = match self.backend.observe_after(&plan.action) {
            Ok(value) => value,
            Err(error) => {
                let code = match dispatch {
                    DispatchResult::Unknown(code) => code,
                    _ => error.code(),
                };
                return unknown_result(plan.before_state_hash, code);
            }
        };
        if validate_observation(&after).is_err()
            || after.epoch != plan.epoch
            || after.target != plan.target
        {
            return unknown_result(plan.before_state_hash, ErrorCode::DeviceIdentityChanged);
        }
        let after_hash = after_hash(&after, &plan.action);
        let verified = verify_after(&after, &plan.action);
        if verified {
            return applied_result(plan.before_state_hash, after_hash);
        }

        if !dispatch_applied {
            let code = match dispatch {
                DispatchResult::Unknown(code) => code,
                _ => ErrorCode::VerificationFailed,
            };
            unknown_result(plan.before_state_hash, code)
        } else {
            failed_result(plan.before_state_hash, ErrorCode::VerificationFailed)
        }
    }

    /// Alias used by application/helper adapters after an opaque token has been minted.
    pub fn execute_once(&self, plan: &RepairPlan) -> RepairResult {
        self.execute(plan)
    }
}

fn validate_observation(observation: &RepairObservation) -> Result<(), RepairError> {
    if observation.target_count == 0 {
        return Err(RepairError::TargetNotFound);
    }
    if observation.target_count > 1 {
        return Err(RepairError::TargetAmbiguous);
    }
    if !observation.target.is_supported() {
        return Err(RepairError::Unsupported);
    }
    if observation.epoch == DeviceEpoch(0) {
        return Err(RepairError::EpochChanged);
    }
    Ok(())
}

fn validate_action(
    observation: &RepairObservation,
    action: &RepairAction,
) -> Result<(), RepairError> {
    if action.requires_adapter() && observation.adapter.identity_hash() == [0; 32] {
        return Err(RepairError::TargetNotFound);
    }
    if action.requires_at() && observation.at_binding_hash.is_none() {
        return Err(RepairError::Unsupported);
    }
    match action {
        RepairAction::ApplyDnsProfile { profile } => {
            if !valid_dns_profile(profile) {
                return Err(RepairError::InvalidDnsProfile);
            }
            let current = observation
                .dns_profile
                .as_ref()
                .ok_or(RepairError::DnsStateUnavailable)?;
            if current == profile {
                return Err(RepairError::NoChange);
            }
        }
        RepairAction::SetUsbNetProfile { profile } => {
            let current = observation
                .usb_profile
                .ok_or(RepairError::InvalidUsbProfile)?;
            if current == *profile {
                return Err(RepairError::NoChange);
            }
        }
        RepairAction::SetApn { cid, apn } => {
            if apn.as_str().is_empty() {
                return Err(RepairError::InvalidApn);
            }
            let matches: Vec<_> = observation
                .contexts
                .iter()
                .filter(|context| context.cid == *cid)
                .collect();
            let context = match matches.as_slice() {
                [] => return Err(RepairError::PdpContextMissing),
                [context] => *context,
                _ => return Err(RepairError::PdpContextAmbiguous),
            };
            if context.pdp_type != Some(PdpTypeForRepair::Ip) || context.apn.is_none() {
                return Err(RepairError::PdpContextIncomplete);
            }
            if context.active {
                return Err(RepairError::PdpContextActive);
            }
        }
        RepairAction::ToggleHotspot { enabled } => {
            let proof = observation
                .hotspot
                .as_ref()
                .ok_or(RepairError::HotspotUnavailable)?;
            if !proof.is_complete() {
                return Err(RepairError::HotspotUnavailable);
            }
            if observation.hotspot_enabled == *enabled {
                return Err(RepairError::NoChange);
            }
        }
        RepairAction::RefreshDhcp
        | RepairAction::RestartAdapter
        | RepairAction::ReenumerateDevice
        | RepairAction::RestartModule => {}
    }
    Ok(())
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

fn validate_plan_observation(
    plan: &RepairPlan,
    observation: &RepairObservation,
) -> Result<(), RepairError> {
    // The before-state comparison covers only the fields the action itself depends on, never
    // unrelated live state (PDP context churn, DNS values, hotspot client counts, a bumped
    // revision) that can legitimately move while the elevation prompt is open. The device
    // identity and epoch checks stay absolute; everything the operation will touch is hashed.
    if observation.epoch != plan.epoch {
        return Err(RepairError::EpochChanged);
    }
    if observation.target != plan.target {
        return Err(RepairError::TargetIdentityChanged);
    }
    if before_hash(observation, &plan.action) != plan.before_state_hash {
        return Err(RepairError::BeforeStateChanged);
    }
    Ok(())
}

fn verify_after(observation: &RepairObservation, action: &RepairAction) -> bool {
    if !observation.target.is_supported() || observation.target.profile() != DJI_GEN1 {
        return false;
    }
    match action {
        RepairAction::RefreshDhcp => observation.adapter.identity_hash() != [0; 32],
        RepairAction::ApplyDnsProfile { profile } => {
            observation.dns_profile.as_ref() == Some(profile)
        }
        RepairAction::RestartAdapter => {
            observation.adapter.identity_hash() != [0; 32] && observation.adapter_up
        }
        RepairAction::ReenumerateDevice => observation.reenumerated,
        RepairAction::SetUsbNetProfile { profile } => observation.usb_profile == Some(*profile),
        RepairAction::SetApn { cid, apn } => observation.contexts.iter().any(|context| {
            context.cid == *cid
                && context.pdp_type.is_some()
                && !context.active
                && context.apn.as_ref().is_some_and(|value| value == apn)
        }),
        RepairAction::RestartModule => observation.reenumerated,
        RepairAction::ToggleHotspot { enabled } => {
            observation
                .hotspot
                .as_ref()
                .is_some_and(HotspotProof::is_complete)
                && observation.hotspot_enabled == *enabled
        }
    }
}

fn before_hash(observation: &RepairObservation, action: &RepairAction) -> BeforeStateHash {
    let mut material = Vec::with_capacity(160);
    material.extend_from_slice(BEFORE_MATERIAL_VERSION);
    material.push(action_tag(action));
    material.extend_from_slice(&observation.epoch.0.to_le_bytes());
    material.extend_from_slice(&observation.target.identity_hash());
    // Action-relevant before-state only: an unrelated observation change must never turn a
    // wanted repair into BeforeStateChanged while the elevation prompt is open.
    match action {
        RepairAction::RefreshDhcp | RepairAction::RestartAdapter => {
            material.extend_from_slice(&observation.adapter.identity_hash());
            material.push(u8::from(observation.adapter_up));
        }
        RepairAction::ApplyDnsProfile { .. } => {
            encode_dns_profile(&mut material, observation.dns_profile.as_ref());
        }
        RepairAction::SetUsbNetProfile { .. } => {
            material.push(match observation.usb_profile {
                Some(VerifiedUsbNetProfile::DjiNdis) => 0,
                Some(VerifiedUsbNetProfile::Ecm) => 1,
                None => 0xff,
            });
        }
        RepairAction::SetApn { cid, .. } => {
            if let Some(context) = observation
                .contexts
                .iter()
                .find(|context| context.cid == *cid)
            {
                material.push(context.cid.get());
                material.push(match context.pdp_type {
                    Some(PdpTypeForRepair::Ip) => 0,
                    Some(PdpTypeForRepair::Ipv6) => 1,
                    Some(PdpTypeForRepair::Ipv4v6) => 2,
                    None => 0xff,
                });
                material.push(u8::from(context.active));
                if let Some(apn) = &context.apn {
                    material.extend_from_slice(&(apn.as_str().len() as u32).to_le_bytes());
                    material.extend_from_slice(apn.as_str().as_bytes());
                } else {
                    material.extend_from_slice(&0_u32.to_le_bytes());
                }
            } else {
                material.push(0xff);
            }
        }
        RepairAction::ToggleHotspot { .. } => {
            match &observation.hotspot {
                Some(proof) if proof.is_complete() => {
                    material.push(1);
                    material.extend_from_slice(&proof.source_profile_hash());
                    material.extend_from_slice(&proof.capability_hash());
                }
                _ => material.push(0),
            }
            material.push(u8::from(observation.hotspot_enabled));
        }
        RepairAction::RestartModule | RepairAction::ReenumerateDevice => {}
    }
    encode_action_fields(&mut material, action, observation);
    BeforeStateHash(sha256(&material))
}

fn after_hash(observation: &RepairObservation, action: &RepairAction) -> AfterStateHash {
    let mut material = Vec::with_capacity(128);
    material.extend_from_slice(IDENTITY_MATERIAL_VERSION);
    material.push(action_tag(action));
    material.extend_from_slice(&observation.epoch.0.to_le_bytes());
    material.extend_from_slice(&observation.target.identity_hash());
    material.extend_from_slice(&observation.adapter.identity_hash());
    encode_optional_hash(&mut material, observation.at_binding_hash);
    material.push(u8::from(observation.adapter_up));
    material.push(u8::from(observation.reenumerated));
    if let Some(profile) = observation.usb_profile {
        material.push(match profile {
            VerifiedUsbNetProfile::DjiNdis => 0,
            VerifiedUsbNetProfile::Ecm => 1,
        });
    }
    encode_dns_profile(&mut material, observation.dns_profile.as_ref());
    match &observation.hotspot {
        Some(proof) if proof.is_complete() => {
            material.push(1);
            material.extend_from_slice(&proof.source_profile_hash());
            material.extend_from_slice(&proof.capability_hash());
        }
        _ => material.push(0),
    }
    material.push(u8::from(observation.hotspot_enabled));
    encode_action_fields(&mut material, action, observation);
    AfterStateHash(sha256(&material))
}

fn encode_action_fields(
    material: &mut Vec<u8>,
    action: &RepairAction,
    observation: &RepairObservation,
) {
    match action {
        RepairAction::ApplyDnsProfile { profile } => encode_dns_profile(material, Some(profile)),
        RepairAction::SetUsbNetProfile { profile } => material.push(match profile {
            VerifiedUsbNetProfile::DjiNdis => 0,
            VerifiedUsbNetProfile::Ecm => 1,
        }),
        RepairAction::SetApn { cid, apn } => {
            material.push(cid.get());
            // APN bytes are part of the machine-only hash, never of Debug/audit output.
            material.push(u8::from(!apn.as_str().is_empty()));
            material.extend_from_slice(&(apn.as_str().len() as u32).to_le_bytes());
            material.extend_from_slice(apn.as_str().as_bytes());
            if let Some(context) = observation
                .contexts
                .iter()
                .find(|context| context.cid == *cid)
            {
                material.push(match context.pdp_type {
                    Some(PdpTypeForRepair::Ip) => 0,
                    Some(PdpTypeForRepair::Ipv6) => 1,
                    Some(PdpTypeForRepair::Ipv4v6) => 2,
                    None => 0xff,
                });
                material.push(u8::from(context.active));
                if let Some(current) = &context.apn {
                    material.extend_from_slice(&(current.as_str().len() as u32).to_le_bytes());
                    material.extend_from_slice(current.as_str().as_bytes());
                } else {
                    material.extend_from_slice(&0_u32.to_le_bytes());
                }
            } else {
                material.push(0xff);
                material.push(0xff);
                material.extend_from_slice(&0_u32.to_le_bytes());
            }
        }
        RepairAction::ToggleHotspot { enabled } => material.push(u8::from(*enabled)),
        RepairAction::RefreshDhcp
        | RepairAction::RestartAdapter
        | RepairAction::ReenumerateDevice
        | RepairAction::RestartModule => {}
    }
}

fn encode_dns_profile(material: &mut Vec<u8>, profile: Option<&DnsProfile>) {
    match profile {
        None => material.push(0xff),
        Some(DnsProfile::Automatic) => material.push(0),
        Some(DnsProfile::Static { servers }) => {
            material.push(1);
            material.push(u8::try_from(servers.len()).unwrap_or(u8::MAX));
            for address in servers {
                match address {
                    std::net::IpAddr::V4(value) => {
                        material.push(4);
                        material.extend_from_slice(&value.octets());
                    }
                    std::net::IpAddr::V6(value) => {
                        material.push(6);
                        for segment in value.segments() {
                            material.extend_from_slice(&segment.to_be_bytes());
                        }
                    }
                }
            }
        }
    }
}

fn encode_optional_hash(material: &mut Vec<u8>, value: Option<[u8; 32]>) {
    match value {
        Some(value) => {
            material.push(1);
            material.extend_from_slice(&value);
        }
        None => material.push(0),
    }
}

fn action_tag(action: &RepairAction) -> u8 {
    match action {
        RepairAction::RefreshDhcp => 1,
        RepairAction::ApplyDnsProfile { .. } => 2,
        RepairAction::RestartAdapter => 3,
        RepairAction::ReenumerateDevice => 4,
        RepairAction::SetUsbNetProfile { .. } => 5,
        RepairAction::SetApn { .. } => 6,
        RepairAction::RestartModule => 7,
        RepairAction::ToggleHotspot { .. } => 8,
    }
}

fn applied_result(before: BeforeStateHash, after: AfterStateHash) -> RepairResult {
    RepairResult {
        outcome: OperationOutcome::Applied {
            after_state_hash: after,
        },
        before_state_hash: before,
        after_state_hash: Some(after),
    }
}

fn failed_result(before: BeforeStateHash, code: ErrorCode) -> RepairResult {
    failed_result_with_rollback(before, code, RollbackOutcome::NotAttempted)
}

fn failed_result_with_rollback(
    before: BeforeStateHash,
    code: ErrorCode,
    rollback: RollbackOutcome,
) -> RepairResult {
    RepairResult {
        outcome: OperationOutcome::Failed { code, rollback },
        before_state_hash: before,
        after_state_hash: None,
    }
}

fn unknown_result(before: BeforeStateHash, code: ErrorCode) -> RepairResult {
    RepairResult {
        outcome: OperationOutcome::OutcomeUnknown { code },
        before_state_hash: before,
        after_state_hash: None,
    }
}

fn failed_or_unknown(
    before: BeforeStateHash,
    error: RepairError,
    dispatched: bool,
) -> RepairResult {
    if dispatched {
        unknown_result(before, error.code())
    } else {
        failed_result(before, error.code())
    }
}

/// A deterministic failure-injection backend with a strict typed mutation boundary.
#[derive(Clone)]
pub struct FakeRepairBackend {
    state: Arc<Mutex<FakeRepairState>>,
}

struct FakeRepairState {
    observation: RepairObservation,
    after_observation: Option<RepairObservation>,
    dispatch_result: DispatchResult,
    mutation_count: usize,
    last_typed_write: Option<&'static str>,
}

impl fmt::Debug for FakeRepairBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FakeRepairBackend([REDACTED_STATE])")
    }
}

impl FakeRepairBackend {
    #[must_use]
    pub fn ready(observation: RepairObservation) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeRepairState {
                observation,
                after_observation: None,
                dispatch_result: DispatchResult::Applied,
                mutation_count: 0,
                last_typed_write: None,
            })),
        }
    }

    pub fn set_dispatch_result(&mut self, result: DispatchResult) {
        self.state
            .lock()
            .expect("fake repair state lock")
            .dispatch_result = result;
    }

    pub fn set_after_usb_profile(&mut self, profile: Option<VerifiedUsbNetProfile>) {
        let mut state = self.state.lock().expect("fake repair state lock");
        let mut after = state
            .after_observation
            .clone()
            .unwrap_or_else(|| state.observation.clone());
        after.usb_profile = profile;
        state.after_observation = Some(after);
    }

    pub fn set_dns_profile(&mut self, profile: Option<DnsProfile>) {
        self.state
            .lock()
            .expect("fake repair state lock")
            .observation
            .dns_profile = profile;
    }

    pub fn set_after_dns_profile(&mut self, profile: Option<DnsProfile>) {
        let mut state = self.state.lock().expect("fake repair state lock");
        let mut after = state
            .after_observation
            .clone()
            .unwrap_or_else(|| state.observation.clone());
        after.dns_profile = profile;
        state.after_observation = Some(after);
    }

    pub fn set_hotspot(&mut self, proof: Option<HotspotProof>, enabled: bool) {
        let mut state = self.state.lock().expect("fake repair state lock");
        state.observation.hotspot = proof;
        state.observation.hotspot_enabled = enabled;
    }

    pub fn set_after_hotspot(&mut self, proof: Option<HotspotProof>, enabled: bool) {
        let mut state = self.state.lock().expect("fake repair state lock");
        let mut after = state
            .after_observation
            .clone()
            .unwrap_or_else(|| state.observation.clone());
        after.hotspot = proof;
        after.hotspot_enabled = enabled;
        state.after_observation = Some(after);
    }

    pub fn set_usb_profile(&mut self, profile: Option<VerifiedUsbNetProfile>) {
        self.state
            .lock()
            .expect("fake repair state lock")
            .observation
            .usb_profile = profile;
    }

    pub fn set_after_observation(&mut self, observation: RepairObservation) {
        self.state
            .lock()
            .expect("fake repair state lock")
            .after_observation = Some(observation);
    }

    pub fn set_contexts(&mut self, contexts: &[(u8, &str, &str, bool)]) {
        self.state
            .lock()
            .expect("fake repair state lock")
            .observation
            .set_contexts(contexts);
    }

    pub fn mutate_observation(&mut self, mutate: impl FnOnce(&mut RepairObservation)) {
        mutate(
            &mut self
                .state
                .lock()
                .expect("fake repair state lock")
                .observation,
        );
    }

    #[must_use]
    pub fn mutation_count(&self) -> usize {
        self.state
            .lock()
            .expect("fake repair state lock")
            .mutation_count
    }

    #[must_use]
    pub fn last_typed_write(&self) -> Option<&'static str> {
        self.state
            .lock()
            .expect("fake repair state lock")
            .last_typed_write
    }
}

impl RepairBackend for FakeRepairBackend {
    fn observe(&self) -> Result<RepairObservation, RepairError> {
        Ok(self
            .state
            .lock()
            .expect("fake repair state lock")
            .observation
            .clone())
    }

    fn dispatch(&self, action: &RepairAction) -> DispatchResult {
        let mut state = self.state.lock().expect("fake repair state lock");
        state.last_typed_write = Some(match action {
            RepairAction::RefreshDhcp => "RefreshDhcp",
            RepairAction::ApplyDnsProfile { .. } => "ApplyDnsProfile",
            RepairAction::RestartAdapter => "RestartAdapter",
            RepairAction::ReenumerateDevice => "ReenumerateDevice",
            RepairAction::SetUsbNetProfile { .. } => "SetUsbNetProfile",
            RepairAction::SetApn { .. } => "SetApn",
            RepairAction::RestartModule => "RestartModule",
            RepairAction::ToggleHotspot { .. } => "ToggleHotspot",
        });
        let dispatch = state.dispatch_result;
        match dispatch {
            DispatchResult::Failed { .. } => {
                // A partial adapter/DNS operation may already have reached the native boundary;
                // count it as one mutation even though the final result is Failed.
                state.mutation_count = state.mutation_count.saturating_add(1);
            }
            DispatchResult::Rejected(ErrorCode::OperationCancelled) => {}
            DispatchResult::Rejected(_) => {}
            DispatchResult::Applied | DispatchResult::Completed | DispatchResult::Success => {
                state.mutation_count = state.mutation_count.saturating_add(1);
                apply_fake_success(&mut state.observation, action);
            }
            DispatchResult::Unknown(_) => {
                state.mutation_count = state.mutation_count.saturating_add(1);
                if matches!(
                    action,
                    RepairAction::RestartModule | RepairAction::ReenumerateDevice
                ) {
                    // The fixture has no proof that a module restart completed when its transport
                    // timed out; a later refresh may provide that proof explicitly.
                    state.observation.reenumerated = false;
                }
            }
        }
        dispatch
    }

    fn observe_after(&self, _action: &RepairAction) -> Result<RepairObservation, RepairError> {
        let state = self.state.lock().expect("fake repair state lock");
        Ok(state
            .after_observation
            .clone()
            .unwrap_or_else(|| state.observation.clone()))
    }
}

fn apply_fake_success(observation: &mut RepairObservation, action: &RepairAction) {
    match action {
        RepairAction::ApplyDnsProfile { profile } => {
            observation.dns_profile = Some(profile.clone())
        }
        RepairAction::SetUsbNetProfile { profile } => observation.usb_profile = Some(*profile),
        RepairAction::SetApn { cid, apn } => {
            if let Some(context) = observation
                .contexts
                .iter_mut()
                .find(|context| context.cid == *cid)
            {
                context.apn = Some(apn.clone());
            }
        }
        RepairAction::RefreshDhcp => observation.adapter_up = true,
        RepairAction::RestartAdapter => observation.adapter_up = true,
        RepairAction::ReenumerateDevice | RepairAction::RestartModule => {
            observation.reenumerated = true
        }
        RepairAction::ToggleHotspot { enabled } => observation.hotspot_enabled = *enabled,
    }
}

/// Production backend for the reviewed Windows/AT boundaries.  The backend starts every
/// observation with a fresh PnP scan and only dispatches through inventory-owned devnodes,
/// resolver-owned adapter identities, the Task 9 WinRT source profile, or the selected typed AT
/// port.  Any platform capability that cannot provide that proof fails closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowsNativeRepairBackend {
    expected_epoch: DeviceEpoch,
}

impl Default for WindowsNativeRepairBackend {
    fn default() -> Self {
        Self {
            expected_epoch: DeviceEpoch(1),
        }
    }
}

impl WindowsNativeRepairBackend {
    /// Bind native observations to the application epoch that minted the confirmation token.
    /// The default exists for small platform probes; production callers should pass their
    /// current reducer epoch explicitly.
    #[must_use]
    pub const fn with_epoch(epoch: DeviceEpoch) -> Self {
        Self {
            expected_epoch: epoch,
        }
    }

    #[must_use]
    pub const fn expected_epoch(self) -> DeviceEpoch {
        self.expected_epoch
    }
}

impl RepairBackend for WindowsNativeRepairBackend {
    fn observe(&self) -> Result<RepairObservation, RepairError> {
        #[cfg(windows)]
        {
            native::observe(self.expected_epoch)
        }
        #[cfg(not(windows))]
        {
            Err(RepairError::UnsupportedPlatform)
        }
    }

    fn dispatch(&self, action: &RepairAction) -> DispatchResult {
        #[cfg(windows)]
        {
            native::dispatch(action, None, self.expected_epoch)
        }
        #[cfg(not(windows))]
        {
            let _ = action;
            DispatchResult::Rejected(ErrorCode::Unsupported)
        }
    }

    fn dispatch_verified(
        &self,
        action: &RepairAction,
        before: &RepairObservation,
    ) -> DispatchResult {
        #[cfg(windows)]
        {
            native::dispatch(action, Some(before), self.expected_epoch)
        }
        #[cfg(not(windows))]
        {
            let _ = (action, before);
            DispatchResult::Rejected(ErrorCode::Unsupported)
        }
    }
}

pub type NativeRepairBackend = WindowsNativeRepairBackend;

#[cfg(windows)]
mod native {
    use super::*;
    use crate::{
        AdapterIdentity, AdapterObservation, DjiDevice, HotspotCapabilityState,
        HotspotOperationOutcome, NetDevnodeRestart, WindowsAdapterResolver, WindowsDeviceInventory,
        WindowsHotspotControl,
    };
    use dji4g_at_protocol::{
        AtCommand, AtFinalCode, AtResponse, PdpContextState, PdpType,
        parse_pdp_contexts_with_activity,
    };
    use dji4g_domain::{DeviceEpoch, HotspotStatus};
    use std::{net::IpAddr, ptr, slice, sync::mpsc::RecvTimeoutError, time::Duration};
    use windows_sys::{
        Win32::NetworkManagement::IpHelper::{
            DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6,
            DNS_SETTING_NAMESERVER, DNS_SETTING_PROFILE_NAMESERVER, FreeInterfaceDnsSettings,
            GetInterfaceDnsSettings, GetInterfaceInfo, IP_ADAPTER_INDEX_MAP, IP_INTERFACE_INFO,
            IpRenewAddress, SetInterfaceDnsSettings,
        },
        core::GUID,
    };

    const AT_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_DNS_TEXT: usize = 4096;
    const ERROR_SUCCESS: u32 = 0;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
    const ERROR_NO_SUCH_DEVICE: u32 = 433;
    const CR_NO_SUCH_DEVNODE: u32 = 0x0000_000d;

    #[derive(Clone)]
    struct NativeScan {
        device: DjiDevice,
        observation: RepairObservation,
    }

    pub(super) fn observe(expected_epoch: DeviceEpoch) -> Result<RepairObservation, RepairError> {
        fresh_scan(expected_epoch).map(|scan| scan.observation)
    }

    pub(super) fn dispatch(
        action: &RepairAction,
        expected: Option<&RepairObservation>,
        expected_epoch: DeviceEpoch,
    ) -> DispatchResult {
        let scan = match fresh_scan(expected_epoch) {
            Ok(scan) => scan,
            Err(error) => return uncertain_or_rejected(error),
        };
        if let Some(expected) = expected {
            if scan.observation.target != expected.target {
                return DispatchResult::Rejected(ErrorCode::DeviceIdentityChanged);
            }
            if scan.observation.epoch != expected.epoch {
                return DispatchResult::Rejected(ErrorCode::DeviceIdentityChanged);
            }
            if before_hash(&scan.observation, action) != before_hash(expected, action) {
                return DispatchResult::Rejected(ErrorCode::EvidenceExpired);
            }
        }
        dispatch_for_scan(action, &scan)
    }

    fn fresh_scan(expected_epoch: DeviceEpoch) -> Result<NativeScan, RepairError> {
        let inventory = WindowsDeviceInventory
            .scan_now()
            .map_err(map_platform_error)?;
        let devices = inventory.devices();
        let device = match devices {
            [] => return Err(RepairError::TargetNotFound),
            [device] => device.clone(),
            _ => return Err(RepairError::TargetAmbiguous),
        };
        let target_hash = target_identity_hash(&device);
        let epoch = if expected_epoch == DeviceEpoch(0) {
            derive_epoch(target_hash)
        } else {
            expected_epoch
        };
        let adapter = WindowsAdapterResolver
            .resolve(&device, epoch)
            .map_err(map_platform_error)?;
        let (at_binding_hash, usb_profile, contexts) = at_observation(&device, epoch);
        let dns_profile = read_dns_profile(&adapter.identity).ok();
        let (hotspot, hotspot_enabled) = hotspot_observation(&adapter.identity);
        let target = TargetProof::dji_gen1(target_hash);
        let adapter_proof = AdapterProof::fixture(adapter_identity_hash(&adapter));
        let mut observation = RepairObservation {
            revision: 0,
            epoch,
            target,
            target_count: devices.len(),
            adapter: adapter_proof,
            adapter_up: adapter.oper_up,
            reenumerated: true,
            at_binding_hash,
            usb_profile,
            contexts,
            dns_profile,
            hotspot,
            hotspot_enabled,
        };
        observation.revision = observation_revision(&observation);
        Ok(NativeScan {
            device,
            observation,
        })
    }

    fn dispatch_for_scan(action: &RepairAction, scan: &NativeScan) -> DispatchResult {
        match action {
            RepairAction::RefreshDhcp => {
                if !scan.observation.adapter_up {
                    return DispatchResult::Rejected(ErrorCode::VerificationFailed);
                }
                let adapter = match resolve_adapter(&scan.device, scan.observation.epoch) {
                    Ok(adapter) => adapter,
                    Err(error) => return uncertain_or_rejected(error),
                };
                if !adapter.dhcp_v4 || adapter.identity.ipv4_index().is_none() {
                    return DispatchResult::Rejected(ErrorCode::Unsupported);
                }
                match renew_dhcp(&adapter.identity) {
                    Ok(()) => DispatchResult::Applied,
                    Err(error) => uncertain_or_failed(error),
                }
            }
            RepairAction::ApplyDnsProfile { profile } => {
                let adapter = match resolve_adapter(&scan.device, scan.observation.epoch) {
                    Ok(adapter) => adapter,
                    Err(error) => return uncertain_or_rejected(error),
                };
                match set_dns_profile(&adapter.identity, profile) {
                    Ok(()) => DispatchResult::Applied,
                    Err(error) => uncertain_or_failed(error),
                }
            }
            RepairAction::RestartAdapter => match crate::pnp::restart_exact_net(&scan.device) {
                Ok(NetDevnodeRestart::Applied) => DispatchResult::Applied,
                Ok(NetDevnodeRestart::DisableFailed { os_code }) => {
                    if is_device_removed_status(os_code) {
                        DispatchResult::Unknown(ErrorCode::DeviceRemoved)
                    } else {
                        DispatchResult::Failed {
                            code: map_win32_code(os_code),
                            rollback: RollbackOutcome::NotAttempted,
                        }
                    }
                }
                Ok(NetDevnodeRestart::EnableFailed { os_code, recovery }) => {
                    if is_device_removed_status(os_code) {
                        DispatchResult::Unknown(ErrorCode::DeviceRemoved)
                    } else {
                        DispatchResult::Failed {
                            code: map_win32_code(os_code),
                            rollback: if recovery {
                                RollbackOutcome::Applied
                            } else {
                                RollbackOutcome::Failed {
                                    code: map_win32_code(os_code),
                                }
                            },
                        }
                    }
                }
                Err(error) => uncertain_or_failed(map_platform_error(error)),
            },
            RepairAction::ReenumerateDevice => {
                match crate::pnp::reenumerate_exact_dji(&scan.device) {
                    Ok(()) => DispatchResult::Applied,
                    Err(error) => {
                        let error = map_platform_error(error);
                        if matches!(
                            error,
                            RepairError::TargetNotFound | RepairError::DeviceRemoved
                        ) {
                            DispatchResult::Unknown(error.code())
                        } else {
                            DispatchResult::Failed {
                                code: error.code(),
                                rollback: RollbackOutcome::NotAttempted,
                            }
                        }
                    }
                }
            }
            RepairAction::SetUsbNetProfile { profile } => typed_at_write(
                &scan.device,
                scan.observation.epoch,
                AtCommand::SetUsbNetProfile(*profile),
            ),
            RepairAction::SetApn { cid, apn } => typed_at_write(
                &scan.device,
                scan.observation.epoch,
                AtCommand::SetApn {
                    cid: *cid,
                    apn: apn.clone(),
                },
            ),
            RepairAction::RestartModule => typed_at_write(
                &scan.device,
                scan.observation.epoch,
                AtCommand::RestartModule,
            ),
            RepairAction::ToggleHotspot { enabled } => {
                let adapter = match resolve_adapter(&scan.device, scan.observation.epoch) {
                    Ok(adapter) => adapter,
                    Err(error) => return uncertain_or_rejected(error),
                };
                let adapter_identity = adapter.identity.clone();
                let result = set_hotspot(&adapter_identity, *enabled);
                match result {
                    Ok(Ok(receipt)) => match receipt.outcome {
                        HotspotOperationOutcome::Applied => DispatchResult::Applied,
                        HotspotOperationOutcome::Failed { code } => DispatchResult::Failed {
                            code,
                            rollback: RollbackOutcome::NotAttempted,
                        },
                        HotspotOperationOutcome::OutcomeUnknown { code } => {
                            DispatchResult::Unknown(code)
                        }
                    },
                    Ok(Err(error)) => DispatchResult::Failed {
                        code: map_hotspot_error(error),
                        rollback: RollbackOutcome::NotAttempted,
                    },
                    Err(error) => DispatchResult::Unknown(error.code()),
                }
            }
        }
    }

    fn resolve_adapter(
        device: &DjiDevice,
        epoch: DeviceEpoch,
    ) -> Result<AdapterObservation, RepairError> {
        WindowsAdapterResolver
            .resolve(device, epoch)
            .map_err(map_platform_error)
    }

    fn uncertain_or_rejected(error: RepairError) -> DispatchResult {
        if matches!(
            error,
            RepairError::TargetNotFound
                | RepairError::DeviceRemoved
                | RepairError::PeerDisconnected
                | RepairError::Timeout
        ) {
            DispatchResult::Unknown(error.code())
        } else {
            DispatchResult::Rejected(error.code())
        }
    }

    fn uncertain_or_failed(error: RepairError) -> DispatchResult {
        if matches!(
            error,
            RepairError::TargetNotFound
                | RepairError::DeviceRemoved
                | RepairError::PeerDisconnected
                | RepairError::Timeout
        ) {
            DispatchResult::Unknown(error.code())
        } else {
            DispatchResult::Failed {
                code: error.code(),
                rollback: RollbackOutcome::NotAttempted,
            }
        }
    }

    fn typed_at_write(
        device: &DjiDevice,
        epoch: DeviceEpoch,
        command: AtCommand,
    ) -> DispatchResult {
        let selected = match device.select_at_port() {
            Ok(selected) => selected,
            Err(_) => return DispatchResult::Rejected(ErrorCode::Unsupported),
        };
        let actor = match crate::AtSessionActor::open_selected(epoch, &selected) {
            Ok(actor) => actor,
            Err(error) => {
                return DispatchResult::Failed {
                    code: map_actor_error(&error).code(),
                    rollback: RollbackOutcome::NotAttempted,
                };
            }
        };
        if let Err(error) = typed_handshake(&actor) {
            return if matches!(error, RepairError::Timeout | RepairError::DeviceRemoved) {
                DispatchResult::Unknown(error.code())
            } else {
                DispatchResult::Failed {
                    code: error.code(),
                    rollback: RollbackOutcome::NotAttempted,
                }
            };
        }
        let response = match actor.try_execute(command) {
            Ok(receiver) => match receiver.recv_timeout(AT_OPERATION_TIMEOUT) {
                Ok(value) => value,
                Err(RecvTimeoutError::Timeout) => {
                    actor.invalidate_epoch();
                    return DispatchResult::Unknown(ErrorCode::Timeout);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return DispatchResult::Unknown(ErrorCode::DeviceRemoved);
                }
            },
            Err(error) => {
                return DispatchResult::Failed {
                    code: map_actor_error(&error).code(),
                    rollback: RollbackOutcome::NotAttempted,
                };
            }
        };
        match response {
            Ok(AtResponse {
                final_code: AtFinalCode::Ok,
                ..
            }) => DispatchResult::Applied,
            Ok(_) => DispatchResult::Failed {
                code: ErrorCode::VerificationFailed,
                rollback: RollbackOutcome::NotAttempted,
            },
            Err(error) => match map_actor_error(&error) {
                RepairError::Timeout | RepairError::DeviceRemoved => {
                    DispatchResult::Unknown(map_actor_error(&error).code())
                }
                mapped => DispatchResult::Failed {
                    code: mapped.code(),
                    rollback: RollbackOutcome::NotAttempted,
                },
            },
        }
    }

    fn typed_handshake(actor: &crate::AtSessionActor) -> Result<Vec<String>, RepairError> {
        let receiver = actor
            .try_safe_handshake()
            .map_err(|error| map_actor_error(&error))?;
        match receiver.recv_timeout(AT_OPERATION_TIMEOUT) {
            Ok(result) => result.map_err(|error| map_actor_error(&error)),
            Err(RecvTimeoutError::Timeout) => {
                actor.invalidate_epoch();
                Err(RepairError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => Err(RepairError::DeviceRemoved),
        }
    }

    fn at_observation(
        device: &DjiDevice,
        epoch: DeviceEpoch,
    ) -> (
        Option<[u8; 32]>,
        Option<VerifiedUsbNetProfile>,
        Vec<RepairPdpContext>,
    ) {
        let selected = match device.select_at_port() {
            Ok(selected) => selected,
            Err(_) => return (None, None, Vec::new()),
        };
        let actor = match crate::AtSessionActor::open_selected(epoch, &selected) {
            Ok(actor) => actor,
            Err(_) => return (None, None, Vec::new()),
        };
        let identity = match typed_handshake(&actor) {
            Ok(identity) => identity,
            Err(_) => return (None, None, Vec::new()),
        };
        let binding_hash = Some(at_binding_hash(&identity));
        let usb_profile = execute_read(&actor, AtCommand::UsbNetQuery)
            .ok()
            .and_then(|response| parse_usb_profile(&response));
        let contexts = match (
            execute_read(&actor, AtCommand::PdpContexts),
            execute_read(&actor, AtCommand::PdpActivation),
        ) {
            (Ok(contexts), Ok(activity)) => parse_pdp_contexts_with_activity(&contexts, &activity)
                .ok()
                .map(|contexts| contexts.into_iter().map(repair_context).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        (binding_hash, usb_profile, contexts)
    }

    fn execute_read(
        actor: &crate::AtSessionActor,
        command: AtCommand,
    ) -> Result<AtResponse, RepairError> {
        let receiver = actor
            .try_execute(command)
            .map_err(|error| map_actor_error(&error))?;
        match receiver.recv_timeout(AT_OPERATION_TIMEOUT) {
            Ok(result) => result.map_err(|error| map_actor_error(&error)),
            Err(RecvTimeoutError::Timeout) => {
                actor.invalidate_epoch();
                Err(RepairError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => Err(RepairError::DeviceRemoved),
        }
    }

    fn repair_context(context: dji4g_at_protocol::PdpContext) -> RepairPdpContext {
        let pdp_type = Some(match context.pdp_type() {
            PdpType::Ip => PdpTypeForRepair::Ip,
            PdpType::Ipv6 => PdpTypeForRepair::Ipv6,
            PdpType::Ipv4v6 => PdpTypeForRepair::Ipv4v6,
        });
        RepairPdpContext {
            cid: context.cid(),
            pdp_type,
            apn: Some(context.apn().clone()),
            active: matches!(context.state(), PdpContextState::Active),
        }
    }

    fn parse_usb_profile(response: &AtResponse) -> Option<VerifiedUsbNetProfile> {
        if response.command != AtCommand::UsbNetQuery
            || response.final_code != AtFinalCode::Ok
            || response.lines.len() != 1
        {
            return None;
        }
        let payload = response.lines[0].strip_prefix("+QCFG:")?.trim();
        let fields: Vec<_> = payload.split(',').map(str::trim).collect();
        if fields.len() != 2 || fields[0] != "\"usbnet\"" {
            return None;
        }
        match fields[1].parse::<u8>().ok()? {
            0 => Some(VerifiedUsbNetProfile::DjiNdis),
            1 => Some(VerifiedUsbNetProfile::Ecm),
            _ => None,
        }
    }

    fn target_identity_hash(device: &DjiDevice) -> [u8; 32] {
        super::authoritative_identity_hash(device)
    }

    fn derive_epoch(identity: [u8; 32]) -> DeviceEpoch {
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&identity[..8]);
        DeviceEpoch(u64::from_le_bytes(bytes).max(1))
    }

    fn adapter_identity_hash(adapter: &AdapterObservation) -> [u8; 32] {
        let identity = &adapter.identity;
        let mut material = Vec::with_capacity(96);
        material.extend_from_slice(b"dji4g-adapter-v1\0");
        append_len_prefixed(&mut material, identity.guid_string().as_bytes());
        material.extend_from_slice(&identity.luid().to_le_bytes());
        material.extend_from_slice(&identity.ipv4_index().unwrap_or(0).to_le_bytes());
        material.extend_from_slice(&identity.ipv6_index().unwrap_or(0).to_le_bytes());
        material.extend_from_slice(&identity.epoch().0.to_le_bytes());
        sha256(&material)
    }

    fn at_binding_hash(identity: &[String]) -> [u8; 32] {
        let mut material = Vec::with_capacity(128);
        material.extend_from_slice(b"dji4g-at-binding-v1\0");
        for line in identity {
            append_len_prefixed(&mut material, line.as_bytes());
        }
        sha256(&material)
    }

    fn observation_revision(observation: &RepairObservation) -> u64 {
        let mut material = Vec::with_capacity(256);
        material.extend_from_slice(b"dji4g-repair-revision-v1\0");
        material.extend_from_slice(&observation.target.identity_hash());
        material.extend_from_slice(&observation.adapter.identity_hash());
        material.push(u8::from(observation.adapter_up));
        material.push(u8::from(observation.reenumerated));
        encode_optional_hash(&mut material, observation.at_binding_hash);
        encode_dns_profile(&mut material, observation.dns_profile.as_ref());
        material.push(
            observation
                .usb_profile
                .map_or(0xff, |profile| match profile {
                    VerifiedUsbNetProfile::DjiNdis => 0,
                    VerifiedUsbNetProfile::Ecm => 1,
                }),
        );
        for context in &observation.contexts {
            material.push(context.cid.get());
            material.push(u8::from(context.active));
            material.push(match context.pdp_type {
                Some(PdpTypeForRepair::Ip) => 0,
                Some(PdpTypeForRepair::Ipv6) => 1,
                Some(PdpTypeForRepair::Ipv4v6) => 2,
                None => 0xff,
            });
            if let Some(apn) = &context.apn {
                append_len_prefixed(&mut material, apn.as_str().as_bytes());
            } else {
                material.extend_from_slice(&0_u32.to_le_bytes());
            }
        }
        if let Some(proof) = observation.hotspot.as_ref() {
            material.extend_from_slice(&proof.source_profile_hash());
            material.extend_from_slice(&proof.capability_hash());
            material.push(u8::from(observation.hotspot_enabled));
        }
        let digest = sha256(&material);
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_le_bytes(bytes).max(1)
    }

    fn append_len_prefixed(material: &mut Vec<u8>, value: &[u8]) {
        material.extend_from_slice(&(value.len() as u32).to_le_bytes());
        material.extend_from_slice(value);
    }

    fn hotspot_observation(adapter: &AdapterIdentity) -> (Option<HotspotProof>, bool) {
        let adapter = adapter.clone();
        let result = read_hotspot_state(&adapter);
        let Ok(Ok((capability, status))) = result else {
            return (None, false);
        };
        if !matches!(capability.state, HotspotCapabilityState::Enabled)
            || capability.source_adapter_id != adapter.guid_string()
        {
            return (None, false);
        }
        let enabled = match status.status {
            HotspotStatus::On { .. } => true,
            HotspotStatus::Off => false,
            _ => return (None, false),
        };
        let source_hash = hash_hotspot_source(&capability.source_adapter_id);
        let capability_hash = hash_hotspot_capability(capability.raw_capability);
        (
            Some(HotspotProof::fixture(source_hash, capability_hash)),
            enabled,
        )
    }

    fn hash_hotspot_source(value: &str) -> [u8; 32] {
        let mut material = b"dji4g-hotspot-source-v1\0".to_vec();
        append_len_prefixed(&mut material, value.as_bytes());
        sha256(&material)
    }

    fn hash_hotspot_capability(raw: u32) -> [u8; 32] {
        sha256(
            &[
                b"dji4g-hotspot-capability-v1\0".as_slice(),
                &raw.to_le_bytes(),
            ]
            .concat(),
        )
    }

    fn read_hotspot_state(
        adapter: &AdapterIdentity,
    ) -> Result<
        Result<
            (
                crate::HotspotCapabilityObservation,
                crate::HotspotStatusObservation,
            ),
            crate::PlatformError,
        >,
        RepairError,
    > {
        let adapter = adapter.clone();
        std::thread::Builder::new()
            .name("dji4g-repair-winrt-read".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| RepairError::Internal)?;
                Ok::<_, RepairError>(runtime.block_on(async move {
                    let control = WindowsHotspotControl::new();
                    let capability = control.capability(&adapter).await?;
                    let status = control.status(&adapter).await?;
                    Ok::<_, crate::PlatformError>((capability, status))
                }))
            })
            .map_err(|_| RepairError::Internal)?
            .join()
            .map_err(|_| RepairError::Internal)?
    }

    fn set_hotspot(
        adapter: &AdapterIdentity,
        enabled: bool,
    ) -> Result<Result<crate::HotspotOperationReceipt, crate::PlatformError>, RepairError> {
        let adapter = adapter.clone();
        std::thread::Builder::new()
            .name("dji4g-repair-winrt-write".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| RepairError::Internal)?;
                Ok::<_, RepairError>(runtime.block_on(async move {
                    // An unsigned development build may change hotspot state unpackaged — the
                    // same development-mode exemption as the helper boundary, gated on this
                    // process carrying no Authenticode signature. Signed installations keep the
                    // strict packaged-only posture.
                    let policy = crate::HotspotPolicy {
                        allow_unpacked_state_change: crate::is_dev_build(),
                        ..crate::HotspotPolicy::default()
                    };
                    WindowsHotspotControl::with_policy(policy)
                        .set_enabled(&adapter, enabled)
                        .await
                }))
            })
            .map_err(|_| RepairError::Internal)?
            .join()
            .map_err(|_| RepairError::Internal)?
    }

    fn renew_dhcp(identity: &AdapterIdentity) -> Result<(), RepairError> {
        let index = identity.ipv4_index().ok_or(RepairError::Unsupported)?;
        let mut size = 0_u32;
        let first = unsafe { GetInterfaceInfo(ptr::null_mut(), &mut size) };
        if first != ERROR_INSUFFICIENT_BUFFER || size == 0 || size > 1024 * 1024 {
            return Err(map_win32_repair_error(first));
        }
        let mut storage = vec![0_u8; size as usize];
        let info = storage.as_mut_ptr().cast::<IP_INTERFACE_INFO>();
        let status = unsafe { GetInterfaceInfo(info, &mut size) };
        if status != ERROR_SUCCESS {
            return Err(map_win32_repair_error(status));
        }
        let count = unsafe { (*info).NumAdapters };
        if count <= 0 || count > 1024 {
            return Err(RepairError::TargetNotFound);
        }
        let adapters = unsafe {
            slice::from_raw_parts(
                ptr::addr_of!((*info).Adapter).cast::<IP_ADAPTER_INDEX_MAP>(),
                count as usize,
            )
        };
        let adapter = adapters
            .iter()
            .find(|adapter| adapter.Index == index)
            .ok_or(RepairError::TargetNotFound)?;
        let status = unsafe { IpRenewAddress(adapter) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(map_win32_repair_error(status))
        }
    }

    fn read_dns_profile(identity: &AdapterIdentity) -> Result<DnsProfile, RepairError> {
        let guid = windows_guid(&identity.guid_string())?;
        let mut settings = DNS_INTERFACE_SETTINGS {
            Version: DNS_INTERFACE_SETTINGS_VERSION1,
            ..DNS_INTERFACE_SETTINGS::default()
        };
        let status = unsafe { GetInterfaceDnsSettings(guid, &mut settings) };
        if status != ERROR_SUCCESS {
            return Err(map_win32_repair_error(status));
        }
        let profile = read_dns_profile_inner(&settings);
        unsafe { FreeInterfaceDnsSettings(&mut settings) };
        profile
    }

    fn read_dns_profile_inner(
        settings: &DNS_INTERFACE_SETTINGS,
    ) -> Result<DnsProfile, RepairError> {
        let flags = settings.Flags as u32;
        if flags & (DNS_SETTING_NAMESERVER | DNS_SETTING_PROFILE_NAMESERVER) == 0 {
            return Ok(DnsProfile::Automatic);
        }
        let pointer = if flags & DNS_SETTING_NAMESERVER != 0 {
            settings.NameServer
        } else {
            settings.ProfileNameServer
        };
        let text = read_wide(pointer)?;
        let servers = parse_dns_servers(&text)?;
        Ok(DnsProfile::Static { servers })
    }

    fn set_dns_profile(
        identity: &AdapterIdentity,
        profile: &DnsProfile,
    ) -> Result<(), RepairError> {
        let guid = windows_guid(&identity.guid_string())?;
        let mut name_server = Vec::new();
        let flags = match profile {
            DnsProfile::Automatic => 0,
            DnsProfile::Static { servers } => {
                if servers.is_empty() || servers.len() > 3 || !valid_dns_profile(profile) {
                    return Err(RepairError::InvalidDnsProfile);
                }
                let families = servers
                    .iter()
                    .map(|address| matches!(address, IpAddr::V6(_)))
                    .collect::<Vec<_>>();
                if families.iter().any(|family| *family != families[0]) {
                    return Err(RepairError::Unsupported);
                }
                let joined = servers
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                name_server.extend(joined.encode_utf16());
                name_server.push(0);
                DNS_SETTING_NAMESERVER | if families[0] { DNS_SETTING_IPV6 } else { 0 }
            }
        };
        let settings = DNS_INTERFACE_SETTINGS {
            Version: DNS_INTERFACE_SETTINGS_VERSION1,
            Flags: u64::from(flags),
            NameServer: if name_server.is_empty() {
                ptr::null_mut()
            } else {
                name_server.as_mut_ptr()
            },
            ..DNS_INTERFACE_SETTINGS::default()
        };
        let status = unsafe { SetInterfaceDnsSettings(guid, &settings) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(map_win32_repair_error(status))
        }
    }

    fn parse_dns_servers(text: &str) -> Result<Vec<IpAddr>, RepairError> {
        let servers: Result<Vec<_>, _> = text
            .split([',', ' ', '\t'])
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<IpAddr>()
                    .map_err(|_| RepairError::InvalidDnsProfile)
            })
            .collect();
        let servers = servers?;
        let profile = DnsProfile::Static { servers };
        valid_dns_profile(&profile)
            .then_some(match profile {
                DnsProfile::Static { servers } => servers,
                DnsProfile::Automatic => unreachable!(),
            })
            .ok_or(RepairError::InvalidDnsProfile)
    }

    fn read_wide(pointer: *mut u16) -> Result<String, RepairError> {
        if pointer.is_null() {
            return Err(RepairError::DnsStateUnavailable);
        }
        let mut length = 0_usize;
        while length < MAX_DNS_TEXT {
            let value = unsafe { *pointer.add(length) };
            if value == 0 {
                let slice = unsafe { slice::from_raw_parts(pointer, length) };
                return String::from_utf16(slice).map_err(|_| RepairError::DnsStateUnavailable);
            }
            length += 1;
        }
        Err(RepairError::DnsStateUnavailable)
    }

    fn windows_guid(value: &str) -> Result<GUID, RepairError> {
        let inner = value
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .ok_or(RepairError::TargetIdentityChanged)?;
        let parts = inner.split('-').collect::<Vec<_>>();
        if parts.len() != 5 || parts[0].len() != 8 || parts[1].len() != 4 || parts[2].len() != 4 {
            return Err(RepairError::TargetIdentityChanged);
        }
        let data1 =
            u32::from_str_radix(parts[0], 16).map_err(|_| RepairError::TargetIdentityChanged)?;
        let data2 =
            u16::from_str_radix(parts[1], 16).map_err(|_| RepairError::TargetIdentityChanged)?;
        let data3 =
            u16::from_str_radix(parts[2], 16).map_err(|_| RepairError::TargetIdentityChanged)?;
        if parts[3].len() != 4 || parts[4].len() != 12 {
            return Err(RepairError::TargetIdentityChanged);
        }
        let mut data4 = [0_u8; 8];
        for (index, byte) in data4.iter_mut().enumerate() {
            let text = if index < 2 {
                &parts[3][index * 2..index * 2 + 2]
            } else {
                &parts[4][(index - 2) * 2..(index - 2) * 2 + 2]
            };
            *byte = u8::from_str_radix(text, 16).map_err(|_| RepairError::TargetIdentityChanged)?;
        }
        Ok(GUID {
            data1,
            data2,
            data3,
            data4,
        })
    }

    fn map_platform_error(error: crate::PlatformError) -> RepairError {
        match error.code {
            code if code.contains("permission") || code.contains("access_denied") => {
                RepairError::PermissionDenied
            }
            code if code.contains("ambiguous") => RepairError::TargetAmbiguous,
            code if code.contains("identity_mismatch") || code.contains("identity_changed") => {
                RepairError::TargetIdentityChanged
            }
            code if code.contains("not_found")
                || code.contains("target_removed")
                || code.contains("no_such") =>
            {
                RepairError::TargetNotFound
            }
            code if code.contains("unsupported") => RepairError::Unsupported,
            _ => RepairError::Internal,
        }
    }

    pub(crate) fn map_hotspot_error(error: crate::PlatformError) -> ErrorCode {
        // Exact stable codes, never substring heuristics: every hotspot: code the platform can
        // produce is classified so a capability/policy gap can never masquerade as an internal
        // error (which read as 「应用发生内部错误」 to the user).
        match error.code {
            "hotspot:access_denied" => ErrorCode::PermissionDenied,
            "hotspot:entitlement_timeout" | "hotspot:operation_timeout" => ErrorCode::Timeout,
            "hotspot:missing_package_identity"
            | "hotspot:unpackaged_control_disabled"
            | "hotspot:policy_disabled"
            | "hotspot:no_wifi_adapter"
            | "hotspot:wifi_control_capability_missing"
            | "hotspot:required_app_missing"
            | "hotspot:operator_disabled"
            | "hotspot:sku_unsupported"
            | "hotspot:unsupported_os"
            | "hotspot:unsupported_platform"
            | "hotspot:unsupported"
            | "hotspot:busy"
            | "hotspot:mobile_broadband_off"
            | "hotspot:bluetooth_device_off"
            | "hotspot:network_limited_connectivity"
            | "hotspot:radio_restriction"
            | "hotspot:band_interference"
            | "hotspot:source_profile_ambiguous"
            | "hotspot:source_profile_unavailable"
            | "hotspot:profile_adapter_id_invalid"
            | "hotspot:target_adapter_id_invalid"
            | "hotspot:profile_enumeration_failed"
            | "hotspot:profile_enumeration_too_large"
            | "hotspot:capability_unknown" => ErrorCode::CapabilityUnavailable,
            // Everything below is a genuine internal defect or an indeterminate outcome and
            // must surface as Internal honestly.
            _ => ErrorCode::Internal,
        }
    }

    fn map_actor_error(error: &crate::ActorError) -> RepairError {
        if let Some(kind) = error.protocol_kind() {
            return match kind {
                dji4g_at_protocol::ProtocolErrorKind::Timeout => RepairError::Timeout,
                dji4g_at_protocol::ProtocolErrorKind::DeviceRemoved => RepairError::DeviceRemoved,
                _ => RepairError::VerificationFailed,
            };
        }
        match error {
            crate::ActorError::Io(std::io::ErrorKind::PermissionDenied) => {
                RepairError::PermissionDenied
            }
            crate::ActorError::FinalCode(_) => RepairError::VerificationFailed,
            _ => RepairError::Internal,
        }
    }

    fn map_win32_repair_error(status: u32) -> RepairError {
        match status {
            ERROR_ACCESS_DENIED => RepairError::PermissionDenied,
            ERROR_DEVICE_NOT_CONNECTED | ERROR_NO_SUCH_DEVICE => RepairError::DeviceRemoved,
            _ => RepairError::Internal,
        }
    }

    fn map_win32_code(status: u32) -> ErrorCode {
        match map_win32_repair_error(status) {
            RepairError::PermissionDenied => ErrorCode::PermissionDenied,
            RepairError::DeviceRemoved => ErrorCode::DeviceRemoved,
            _ => ErrorCode::Internal,
        }
    }

    fn is_device_removed_status(status: u32) -> bool {
        matches!(
            status,
            ERROR_DEVICE_NOT_CONNECTED | ERROR_NO_SUCH_DEVICE | CR_NO_SUCH_DEVNODE
        )
    }
}

// SHA-256 is kept local to make evidence hashes deterministic without widening the dependency
// surface of the platform crate.
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
        0xbb67ae85_u32,
        0x3c6ef372_u32,
        0xa54ff53a_u32,
        0x510e527f_u32,
        0x9b05688c_u32,
        0x1f83d9ab_u32,
        0x5be0cd19_u32,
    ];
    for chunk in data.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes([
                chunk[index * 4],
                chunk[index * 4 + 1],
                chunk[index * 4 + 2],
                chunk[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
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
    for (index, word) in h.into_iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_sha256_and_repair_debug_is_redacted() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
        let action = RepairAction::SetApn {
            cid: PdpContextId::try_from(1).expect("valid CID"),
            apn: Apn::try_from("private.example").expect("valid APN"),
        };
        assert!(!format!("{action:?}").contains("private.example"));
    }

    #[test]
    fn hotspot_platform_errors_never_masquerade_as_internal() {
        let platform_error = |code| crate::PlatformError {
            code,
            os_code: None,
        };
        // The failures a real machine actually hits on a development build — and the
        // capability/policy gaps — must surface as CapabilityUnavailable, never Internal.
        for code in [
            "hotspot:unpackaged_control_disabled",
            "hotspot:missing_package_identity",
            "hotspot:policy_disabled",
            "hotspot:no_wifi_adapter",
            "hotspot:wifi_control_capability_missing",
            "hotspot:busy",
            "hotspot:source_profile_unavailable",
            "hotspot:profile_enumeration_failed",
        ] {
            assert_eq!(
                super::native::map_hotspot_error(platform_error(code)),
                ErrorCode::CapabilityUnavailable,
                "{code} must read as a capability gap"
            );
        }
        assert_eq!(
            super::native::map_hotspot_error(platform_error("hotspot:access_denied")),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            super::native::map_hotspot_error(platform_error("hotspot:entitlement_timeout")),
            ErrorCode::Timeout
        );
        // A genuine internal defect stays Internal.
        assert_eq!(
            super::native::map_hotspot_error(platform_error("hotspot:wrong_apartment")),
            ErrorCode::Internal
        );
    }
}
