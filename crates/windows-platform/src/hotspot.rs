//! Windows Wi-Fi tethering adapter.
//!
//! The adapter deliberately keeps the WinRT boundary small.  A connection profile is selected
//! only by the adapter GUID produced by the inventory resolver; no friendly name, default route,
//! profile ordering, or internet-profile fallback is accepted.  WinRT objects are created from a
//! fresh enumeration for every operation, so a device epoch cannot accidentally retain a stale
//! profile or tethering manager.

use std::time::Duration;

use dji4g_domain::{ErrorCode, HotspotStatus, HotspotUnsupportedReason};

use crate::{AdapterGuid, AdapterIdentity, PlatformError};

const CODE_OK: &str = "hotspot:ok";

/// A profile descriptor used by the deterministic profile-selection seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileDescriptor {
    /// The enumeration ordinal.  It is an opaque lookup key, not a selection signal.
    pub ordinal: usize,
    /// `ConnectionProfile.NetworkAdapter.NetworkAdapterId` in GUID form.
    pub adapter_id: String,
}

/// Capability state returned by the tethering capability API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotspotCapabilityState {
    Enabled,
    Unsupported(HotspotUnsupportedReason),
    /// A known disabled cause which is not represented by the domain's user-facing reasons.
    Disabled {
        code: &'static str,
    },
}

/// Direction supplied while reading an in-transition tethering manager state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotspotTransitionHint {
    Starting,
    Stopping,
}

/// Result of mapping one operational-state read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotspotStateMapping {
    pub status: HotspotStatus,
    pub client_count: Option<u32>,
    pub client_count_error: Option<&'static str>,
    pub stable_code: &'static str,
}

/// Outcome of a start/stop operation status plus its final-state verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotspotOperationOutcome {
    Applied,
    Failed { code: ErrorCode },
    OutcomeUnknown { code: ErrorCode },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotspotOperationMapping {
    pub outcome: HotspotOperationOutcome,
    pub stable_code: &'static str,
}

/// Fresh capability data tied to the selected source adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotspotCapabilityObservation {
    pub source_adapter_id: String,
    pub state: HotspotCapabilityState,
    pub raw_capability: u32,
}

/// Fresh status data tied to the selected source adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotspotStatusObservation {
    pub source_adapter_id: String,
    pub capability: HotspotCapabilityState,
    pub status: HotspotStatus,
    pub raw_operational_state: u32,
    pub client_count: Option<u32>,
    pub client_count_error: Option<&'static str>,
    pub stable_code: &'static str,
}

/// Package/runtime policy for state-changing WinRT calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotspotPolicy {
    /// A caller may turn this on only when a product policy and explicit HIL authorization allow
    /// state changes from an unpackaged process.  Read-only capability/status calls are unaffected.
    pub allow_unpacked_state_change: bool,
    pub operation_timeout: Duration,
}

impl Default for HotspotPolicy {
    fn default() -> Self {
        Self {
            allow_unpacked_state_change: false,
            operation_timeout: Duration::from_secs(30),
        }
    }
}

/// Whether this process currently has a package identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageIdentityState {
    Packaged,
    Unpackaged,
}

/// Receipt for a single, non-retried start/stop request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotspotOperationReceipt {
    pub requested_enabled: bool,
    pub operation_status: u32,
    pub outcome: HotspotOperationOutcome,
    pub stable_code: &'static str,
    pub final_status: Option<HotspotStatus>,
}

/// Concrete Windows tethering boundary.
///
/// This type owns policy only.  It intentionally does not cache `ConnectionProfile` or
/// `NetworkOperatorTetheringManager` instances; each method re-enumerates the profile set.
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsHotspotControl {
    policy: HotspotPolicy,
}

impl WindowsHotspotControl {
    #[must_use]
    pub fn new() -> Self {
        Self {
            policy: HotspotPolicy::default(),
        }
    }

    #[must_use]
    pub const fn with_policy(policy: HotspotPolicy) -> Self {
        Self { policy }
    }

    #[must_use]
    pub const fn policy(self) -> HotspotPolicy {
        self.policy
    }

    /// Read package identity without changing any state.
    pub fn package_identity(self) -> Result<PackageIdentityState, PlatformError> {
        package_identity()
    }

    /// Read capability from a fresh, exact-GUID profile enumeration.
    pub async fn capability(
        self,
        adapter: &AdapterIdentity,
    ) -> Result<HotspotCapabilityObservation, PlatformError> {
        let _ = self;
        capability(adapter)
    }

    /// Read operational state and (when On) client count from fresh WinRT objects.
    pub async fn status(
        self,
        adapter: &AdapterIdentity,
    ) -> Result<HotspotStatusObservation, PlatformError> {
        let _ = self;
        status(adapter)
    }

    /// Perform exactly one start or stop and then perform a fresh final readback.
    pub async fn set_enabled(
        self,
        adapter: &AdapterIdentity,
        enabled: bool,
    ) -> Result<HotspotOperationReceipt, PlatformError> {
        set_enabled(self.policy, adapter, enabled).await
    }
}

/// Select exactly one profile whose NetworkAdapterId is the resolver-owned adapter GUID.
///
/// Every descriptor is parsed before selection.  This intentionally fails closed when WinRT
/// returns a malformed/non-GUID adapter id rather than silently selecting a different profile.
pub fn select_source_profile_index(
    target_adapter_id: &str,
    profiles: &[ProfileDescriptor],
) -> Result<usize, PlatformError> {
    let target = parse_profile_guid(target_adapter_id, "hotspot:target_adapter_id_invalid")?;
    let mut selected = None;
    for profile in profiles {
        let candidate =
            parse_profile_guid(&profile.adapter_id, "hotspot:profile_adapter_id_invalid")?;
        if candidate == target {
            if selected.is_some() {
                return Err(PlatformError {
                    code: "hotspot:source_profile_ambiguous",
                    os_code: None,
                });
            }
            selected = Some(profile.ordinal);
        }
    }
    selected.ok_or(PlatformError {
        code: "hotspot:source_profile_unavailable",
        os_code: None,
    })
}

/// Parse a WinRT GUID in either its `xxxxxxxx-xxxx-...` or `{xxxxxxxx-xxxx-...}` form.
///
/// `AdapterGuid` intentionally remains strict for the NetCfg parser.  WinRT's projection uses
/// the unbraced form in some versions and the debug form in others, so the hotspot seam performs
/// this narrow normalization before comparing binary GUID values.
fn parse_profile_guid(value: &str, error_code: &'static str) -> Result<AdapterGuid, PlatformError> {
    let inner = if let Some(inner) = value.strip_prefix('{') {
        inner.strip_suffix('}').ok_or(PlatformError {
            code: error_code,
            os_code: None,
        })?
    } else {
        if value.contains(['{', '}']) {
            return Err(PlatformError {
                code: error_code,
                os_code: None,
            });
        }
        value
    };
    let normalized = format!("{{{inner}}}");
    normalized
        .parse::<AdapterGuid>()
        .map_err(|_| PlatformError {
            code: error_code,
            os_code: None,
        })
}

/// Map the documented WinRT `TetheringCapability` values.
#[must_use]
pub fn map_capability(raw: u32) -> HotspotCapabilityState {
    match raw {
        0 => HotspotCapabilityState::Enabled,
        1 => HotspotCapabilityState::Unsupported(HotspotUnsupportedReason::PolicyDisabled),
        2 => HotspotCapabilityState::Unsupported(HotspotUnsupportedReason::NoWifiAdapter),
        3 => HotspotCapabilityState::Disabled {
            code: "hotspot:operator_disabled",
        },
        4 => HotspotCapabilityState::Disabled {
            code: "hotspot:sku_unsupported",
        },
        5 => HotspotCapabilityState::Disabled {
            code: "hotspot:required_app_missing",
        },
        6 => HotspotCapabilityState::Disabled {
            code: "hotspot:capability_unknown",
        },
        7 => HotspotCapabilityState::Unsupported(
            HotspotUnsupportedReason::MissingWifiControlCapability,
        ),
        _ => HotspotCapabilityState::Disabled {
            code: "hotspot:capability_unknown",
        },
    }
}

/// Map `TetheringOperationalState`.  An On state is retained even when ClientCount fails.
#[must_use]
pub fn map_operational_state(
    raw: u32,
    transition_hint: Option<HotspotTransitionHint>,
    client_count: Result<u32, PlatformError>,
) -> HotspotStateMapping {
    match raw {
        0 => HotspotStateMapping {
            status: HotspotStatus::Failed {
                code: ErrorCode::Internal,
            },
            client_count: None,
            client_count_error: None,
            stable_code: "hotspot:operational_state_unknown",
        },
        1 => match client_count {
            Ok(count) => HotspotStateMapping {
                status: HotspotStatus::On {
                    clients: Some(count),
                },
                client_count: Some(count),
                client_count_error: None,
                stable_code: CODE_OK,
            },
            Err(error) => HotspotStateMapping {
                status: HotspotStatus::On { clients: None },
                client_count: None,
                client_count_error: Some(error.code),
                stable_code: error.code,
            },
        },
        2 => HotspotStateMapping {
            status: HotspotStatus::Off,
            client_count: None,
            client_count_error: None,
            stable_code: CODE_OK,
        },
        3 => match transition_hint {
            Some(HotspotTransitionHint::Starting) => HotspotStateMapping {
                status: HotspotStatus::Starting,
                client_count: None,
                client_count_error: None,
                stable_code: CODE_OK,
            },
            Some(HotspotTransitionHint::Stopping) => HotspotStateMapping {
                status: HotspotStatus::Stopping,
                client_count: None,
                client_count_error: None,
                stable_code: CODE_OK,
            },
            None => HotspotStateMapping {
                status: HotspotStatus::Failed {
                    code: ErrorCode::Internal,
                },
                client_count: None,
                client_count_error: None,
                stable_code: "hotspot:transition_direction_unknown",
            },
        },
        _ => HotspotStateMapping {
            status: HotspotStatus::Failed {
                code: ErrorCode::Internal,
            },
            client_count: None,
            client_count_error: None,
            stable_code: "hotspot:operational_state_unknown",
        },
    }
}

/// Map `TetheringOperationStatus` and verify the fresh final state.
#[must_use]
pub fn map_operation_status(
    raw: u32,
    requested_enabled: bool,
    final_status: Option<HotspotStatus>,
) -> HotspotOperationMapping {
    let expected = if requested_enabled {
        HotspotStatusExpectation::On
    } else {
        HotspotStatusExpectation::Off
    };
    match raw {
        0 => match final_status {
            Some(status) if expected.matches(status) => HotspotOperationMapping {
                outcome: HotspotOperationOutcome::Applied,
                stable_code: CODE_OK,
            },
            Some(_) => HotspotOperationMapping {
                outcome: HotspotOperationOutcome::Failed {
                    code: ErrorCode::VerificationFailed,
                },
                stable_code: "hotspot:final_state_mismatch",
            },
            None => HotspotOperationMapping {
                outcome: HotspotOperationOutcome::OutcomeUnknown {
                    code: ErrorCode::VerificationFailed,
                },
                stable_code: "hotspot:final_state_unavailable",
            },
        },
        9 => {
            if requested_enabled && matches!(final_status, Some(HotspotStatus::On { .. })) {
                HotspotOperationMapping {
                    outcome: HotspotOperationOutcome::Applied,
                    stable_code: "hotspot:already_on",
                }
            } else {
                HotspotOperationMapping {
                    outcome: HotspotOperationOutcome::Failed {
                        code: ErrorCode::VerificationFailed,
                    },
                    stable_code: "hotspot:already_on",
                }
            }
        }
        2 => failed_operation(
            ErrorCode::CapabilityUnavailable,
            "hotspot:mobile_broadband_off",
        ),
        3 => failed_operation(ErrorCode::CapabilityUnavailable, "hotspot:no_wifi_adapter"),
        4 => failed_operation(ErrorCode::Timeout, "hotspot:entitlement_timeout"),
        5 => failed_operation(
            ErrorCode::CapabilityUnavailable,
            "hotspot:entitlement_failure",
        ),
        6 => HotspotOperationMapping {
            outcome: HotspotOperationOutcome::OutcomeUnknown {
                code: ErrorCode::Timeout,
            },
            stable_code: "hotspot:operation_in_progress",
        },
        7 => failed_operation(
            ErrorCode::CapabilityUnavailable,
            "hotspot:bluetooth_device_off",
        ),
        8 => failed_operation(
            ErrorCode::CapabilityUnavailable,
            "hotspot:network_limited_connectivity",
        ),
        10 => failed_operation(
            ErrorCode::CapabilityUnavailable,
            "hotspot:radio_restriction",
        ),
        11 => failed_operation(
            ErrorCode::CapabilityUnavailable,
            "hotspot:band_interference",
        ),
        _ => HotspotOperationMapping {
            outcome: HotspotOperationOutcome::OutcomeUnknown {
                code: ErrorCode::Internal,
            },
            stable_code: "hotspot:operation_status_unknown",
        },
    }
}

fn failed_operation(code: ErrorCode, stable_code: &'static str) -> HotspotOperationMapping {
    HotspotOperationMapping {
        outcome: HotspotOperationOutcome::Failed { code },
        stable_code,
    }
}

#[derive(Clone, Copy)]
enum HotspotStatusExpectation {
    On,
    Off,
}

impl HotspotStatusExpectation {
    fn matches(self, status: HotspotStatus) -> bool {
        matches!(
            (self, status),
            (Self::On, HotspotStatus::On { .. }) | (Self::Off, HotspotStatus::Off)
        )
    }
}

/// Convert a Win32/HRESULT value to a stable, non-localized platform error.
#[must_use]
pub fn map_hresult(raw: u32, _phase: &str) -> PlatformError {
    let code = match raw {
        15_700 | 0x8007_3d54 => "hotspot:missing_package_identity",
        5 | 0x8007_0005 => "hotspot:access_denied",
        1_260 | 0x8007_04ec => "hotspot:policy_disabled",
        170 | 0x8007_00aa => "hotspot:busy",
        0x8001_010e => "hotspot:wrong_apartment",
        120 | 0x8007_0078 | 50 | 0x8007_0032 | 0x8000_000f | 0x8004_0154 | 0x8000_4001 => {
            "hotspot:unsupported_os"
        }
        _ => "hotspot:winrt_api_failed",
    };
    PlatformError {
        code,
        os_code: Some(raw),
    }
}

fn package_identity() -> Result<PackageIdentityState, PlatformError> {
    #[cfg(windows)]
    {
        native::package_identity()
    }
    #[cfg(not(windows))]
    {
        Ok(PackageIdentityState::Unpackaged)
    }
}

fn capability(adapter: &AdapterIdentity) -> Result<HotspotCapabilityObservation, PlatformError> {
    #[cfg(windows)]
    {
        native::capability(adapter)
    }
    #[cfg(not(windows))]
    {
        let _ = adapter;
        Err(PlatformError {
            code: "hotspot:unsupported_platform",
            os_code: None,
        })
    }
}

fn status(adapter: &AdapterIdentity) -> Result<HotspotStatusObservation, PlatformError> {
    #[cfg(windows)]
    {
        native::status(adapter)
    }
    #[cfg(not(windows))]
    {
        let _ = adapter;
        Err(PlatformError {
            code: "hotspot:unsupported_platform",
            os_code: None,
        })
    }
}

async fn set_enabled(
    policy: HotspotPolicy,
    adapter: &AdapterIdentity,
    enabled: bool,
) -> Result<HotspotOperationReceipt, PlatformError> {
    #[cfg(windows)]
    {
        native::set_enabled(policy, adapter, enabled).await
    }
    #[cfg(not(windows))]
    {
        let _ = (policy, adapter, enabled);
        Err(PlatformError {
            code: "hotspot:unsupported_platform",
            os_code: None,
        })
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::ptr;

    use windows::{
        Networking::{
            Connectivity::{ConnectionProfile, NetworkInformation},
            NetworkOperators::{
                NetworkOperatorTetheringManager, TetheringOperationStatus,
                TetheringOperationalState,
            },
        },
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        core::Interface,
    };
    use windows_future::IAsyncInfo;

    struct ApartmentGuard;

    impl ApartmentGuard {
        fn new() -> Result<Self, PlatformError> {
            // SAFETY: this initializes WinRT for the current worker thread.  The guard balances
            // the successful call and no WinRT object is shared with an uninitialized UI thread.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|error| map_hresult(error.code().0 as u32, "ro_initialize"))?;
            Ok(Self)
        }
    }

    impl Drop for ApartmentGuard {
        fn drop(&mut self) {
            // SAFETY: this is the matching call for the successful RoInitialize on this thread.
            unsafe { RoUninitialize() };
        }
    }

    struct NativeProfile {
        descriptor: ProfileDescriptor,
        profile: ConnectionProfile,
    }

    fn find_profile<'a>(
        adapter: &AdapterIdentity,
        profiles: &'a [NativeProfile],
    ) -> Result<&'a NativeProfile, PlatformError> {
        let descriptors: Vec<ProfileDescriptor> = profiles
            .iter()
            .map(|profile| profile.descriptor.clone())
            .collect();
        let ordinal = select_source_profile_index(&adapter.guid_string(), &descriptors)?;
        profiles
            .iter()
            .find(|profile| profile.descriptor.ordinal == ordinal)
            .ok_or(PlatformError {
                code: "hotspot:source_profile_unavailable",
                os_code: None,
            })
    }

    pub(super) fn capability(
        adapter: &AdapterIdentity,
    ) -> Result<HotspotCapabilityObservation, PlatformError> {
        let _apartment = ApartmentGuard::new()?;
        let profiles = enumerate_profiles_without_reinit()?;
        let selected = find_profile(adapter, &profiles)?;
        let raw = NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(
            &selected.profile,
        )
        .map_err(|error| map_hresult(error.code().0 as u32, "capability"))?;
        let raw_capability = raw.0 as u32;
        Ok(HotspotCapabilityObservation {
            source_adapter_id: adapter.guid_string(),
            state: map_capability(raw_capability),
            raw_capability,
        })
    }

    pub(super) fn status(
        adapter: &AdapterIdentity,
    ) -> Result<HotspotStatusObservation, PlatformError> {
        let _apartment = ApartmentGuard::new()?;
        let profiles = enumerate_profiles_without_reinit()?;
        let selected = find_profile(adapter, &profiles)?;
        let raw_capability =
            NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(
                &selected.profile,
            )
            .map_err(|error| map_hresult(error.code().0 as u32, "capability"))?;
        let capability = map_capability(raw_capability.0 as u32);
        let unsupported = match capability {
            HotspotCapabilityState::Unsupported(reason) => Some(HotspotStatus::Unsupported(reason)),
            HotspotCapabilityState::Disabled { .. } => Some(HotspotStatus::Failed {
                code: ErrorCode::CapabilityUnavailable,
            }),
            HotspotCapabilityState::Enabled => None,
        };
        if let Some(status) = unsupported {
            return Ok(HotspotStatusObservation {
                source_adapter_id: adapter.guid_string(),
                capability,
                status,
                raw_operational_state: TetheringOperationalState::Unknown.0 as u32,
                client_count: None,
                client_count_error: None,
                stable_code: match capability {
                    HotspotCapabilityState::Unsupported(
                        HotspotUnsupportedReason::PolicyDisabled,
                    ) => "hotspot:policy_disabled",
                    HotspotCapabilityState::Unsupported(
                        HotspotUnsupportedReason::NoWifiAdapter,
                    ) => "hotspot:no_wifi_adapter",
                    HotspotCapabilityState::Unsupported(
                        HotspotUnsupportedReason::MissingWifiControlCapability,
                    ) => "hotspot:wifi_control_capability_missing",
                    HotspotCapabilityState::Unsupported(_) => "hotspot:unsupported",
                    HotspotCapabilityState::Disabled { code } => code,
                    HotspotCapabilityState::Enabled => CODE_OK,
                },
            });
        }
        let manager =
            NetworkOperatorTetheringManager::CreateFromConnectionProfile(&selected.profile)
                .map_err(|error| map_hresult(error.code().0 as u32, "manager_create"))?;
        let operational = manager
            .TetheringOperationalState()
            .map_err(|error| map_hresult(error.code().0 as u32, "operational_state"))?;
        let raw_operational_state = operational.0 as u32;
        let clients = if raw_operational_state == TetheringOperationalState::On.0 as u32 {
            manager.ClientCount().map_err(|error| PlatformError {
                code: "hotspot:client_count_failed",
                os_code: Some(error.code().0 as u32),
            })
        } else {
            Ok(0)
        };
        let mapped = map_operational_state(raw_operational_state, None, clients);
        Ok(HotspotStatusObservation {
            source_adapter_id: adapter.guid_string(),
            capability,
            status: mapped.status,
            raw_operational_state,
            client_count: mapped.client_count,
            client_count_error: mapped.client_count_error,
            stable_code: mapped.stable_code,
        })
    }

    pub(super) async fn set_enabled(
        policy: HotspotPolicy,
        adapter: &AdapterIdentity,
        enabled: bool,
    ) -> Result<HotspotOperationReceipt, PlatformError> {
        let package_state = package_identity()?;
        if package_state == PackageIdentityState::Unpackaged && !policy.allow_unpacked_state_change
        {
            return Err(PlatformError {
                code: "hotspot:unpackaged_control_disabled",
                os_code: None,
            });
        }
        // Keep the guard alive for the complete WinRT operation.  The application invokes this
        // future from its worker executor, never from the egui event thread.
        let _apartment = ApartmentGuard::new()?;
        let profiles = enumerate_profiles_without_reinit()?;
        let selected = find_profile(adapter, &profiles)?;
        let raw_capability =
            NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(
                &selected.profile,
            )
            .map_err(|error| map_hresult(error.code().0 as u32, "capability"))?;
        let capability = map_capability(raw_capability.0 as u32);
        if !matches!(capability, HotspotCapabilityState::Enabled) {
            return Err(PlatformError {
                code: match capability {
                    HotspotCapabilityState::Unsupported(
                        HotspotUnsupportedReason::PolicyDisabled,
                    ) => "hotspot:policy_disabled",
                    HotspotCapabilityState::Unsupported(
                        HotspotUnsupportedReason::NoWifiAdapter,
                    ) => "hotspot:no_wifi_adapter",
                    HotspotCapabilityState::Unsupported(
                        HotspotUnsupportedReason::MissingWifiControlCapability,
                    ) => "hotspot:wifi_control_capability_missing",
                    HotspotCapabilityState::Unsupported(_) => "hotspot:unsupported",
                    HotspotCapabilityState::Disabled { code } => code,
                    HotspotCapabilityState::Enabled => CODE_OK,
                },
                os_code: None,
            });
        }
        let manager =
            NetworkOperatorTetheringManager::CreateFromConnectionProfile(&selected.profile)
                .map_err(|error| map_hresult(error.code().0 as u32, "manager_create"))?;
        let before = manager
            .TetheringOperationalState()
            .map_err(|error| map_hresult(error.code().0 as u32, "operational_state"))?;
        if (enabled && before == TetheringOperationalState::On)
            || (!enabled && before == TetheringOperationalState::Off)
        {
            let final_status = fresh_final_status(adapter).await?;
            let operation_status = if enabled {
                TetheringOperationStatus::AlreadyOn.0 as u32
            } else {
                TetheringOperationStatus::Success.0 as u32
            };
            let mapped = map_operation_status(operation_status, enabled, Some(final_status));
            return Ok(HotspotOperationReceipt {
                requested_enabled: enabled,
                operation_status,
                outcome: mapped.outcome,
                stable_code: mapped.stable_code,
                final_status: Some(final_status),
            });
        }
        if !matches!(
            before,
            TetheringOperationalState::On | TetheringOperationalState::Off
        ) {
            return Ok(HotspotOperationReceipt {
                requested_enabled: enabled,
                operation_status: before.0 as u32,
                outcome: HotspotOperationOutcome::OutcomeUnknown {
                    code: ErrorCode::Internal,
                },
                stable_code: "hotspot:operational_state_unknown",
                final_status: None,
            });
        }
        let operation = if enabled {
            manager
                .StartTetheringAsync()
                .map_err(|error| map_hresult(error.code().0 as u32, "start"))?
        } else {
            manager
                .StopTetheringAsync()
                .map_err(|error| map_hresult(error.code().0 as u32, "stop"))?
        };
        let async_info: IAsyncInfo = operation
            .cast()
            .map_err(|error| map_hresult(error.code().0 as u32, "async_info"))?;
        let completed =
            tokio::time::timeout(policy.operation_timeout, operation.into_future()).await;
        let result = match completed {
            Ok(result) => {
                result.map_err(|error| map_hresult(error.code().0 as u32, "operation_result"))?
            }
            Err(_) => {
                let _ = async_info.Cancel();
                let final_status = fresh_final_status(adapter).await.ok();
                return Ok(HotspotOperationReceipt {
                    requested_enabled: enabled,
                    operation_status: TetheringOperationStatus::OperationInProgress.0 as u32,
                    outcome: HotspotOperationOutcome::OutcomeUnknown {
                        code: ErrorCode::Timeout,
                    },
                    stable_code: "hotspot:operation_timeout",
                    final_status,
                });
            }
        };
        let operation_status = result
            .Status()
            .map_err(|error| map_hresult(error.code().0 as u32, "operation_status"))?;
        let final_status = fresh_final_status(adapter).await.ok();
        let mapped = map_operation_status(operation_status.0 as u32, enabled, final_status);
        Ok(HotspotOperationReceipt {
            requested_enabled: enabled,
            operation_status: operation_status.0 as u32,
            outcome: mapped.outcome,
            stable_code: mapped.stable_code,
            final_status,
        })
    }

    // This helper is used only while the outer set_enabled guard is alive.  It must not call
    // RoInitialize again, because that would make the final readback depend on nested apartment
    // state and could return RPC_E_WRONG_THREAD on a migrated executor thread.
    fn enumerate_profiles_without_reinit() -> Result<Vec<NativeProfile>, PlatformError> {
        const MAX_PROFILES: u32 = 1024;
        let profiles = NetworkInformation::GetConnectionProfiles()
            .map_err(|error| map_hresult(error.code().0 as u32, "enumerate_profiles"))?;
        let count = profiles
            .Size()
            .map_err(|error| map_hresult(error.code().0 as u32, "enumerate_profiles"))?;
        if count > MAX_PROFILES {
            return Err(PlatformError {
                code: "hotspot:profile_enumeration_too_large",
                os_code: Some(count),
            });
        }
        let mut result = Vec::with_capacity(count as usize);
        for ordinal in 0..count {
            let profile = profiles
                .GetAt(ordinal)
                .map_err(|error| map_hresult(error.code().0 as u32, "profile_read"))?;
            let adapter = profile
                .NetworkAdapter()
                .map_err(|error| map_hresult(error.code().0 as u32, "profile_adapter"))?;
            let adapter_id = adapter
                .NetworkAdapterId()
                .map_err(|error| map_hresult(error.code().0 as u32, "profile_adapter_id"))?;
            result.push(NativeProfile {
                descriptor: ProfileDescriptor {
                    ordinal: ordinal as usize,
                    adapter_id: format!("{{{adapter_id:?}}}"),
                },
                profile,
            });
        }
        Ok(result)
    }

    async fn fresh_final_status(adapter: &AdapterIdentity) -> Result<HotspotStatus, PlatformError> {
        status(adapter).map(|observation| observation.status)
    }

    pub(super) fn package_identity() -> Result<PackageIdentityState, PlatformError> {
        const APPMODEL_ERROR_NO_PACKAGE: u32 = 15_700;
        const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
        let mut length = 0_u32;
        // SAFETY: the sizing call follows the documented GetCurrentPackageFullName contract.
        let status = unsafe { GetCurrentPackageFullName(&mut length, ptr::null_mut()) };
        if status == APPMODEL_ERROR_NO_PACKAGE {
            return Ok(PackageIdentityState::Unpackaged);
        }
        if status != ERROR_INSUFFICIENT_BUFFER || length == 0 || length > 32 * 1024 {
            return Err(map_hresult(status, "package_identity"));
        }
        let mut name = vec![0_u16; length as usize];
        // SAFETY: the buffer has the exact capacity requested by the sizing call.
        let status = unsafe { GetCurrentPackageFullName(&mut length, name.as_mut_ptr()) };
        if status == 0 {
            Ok(PackageIdentityState::Packaged)
        } else {
            Err(map_hresult(status, "package_identity"))
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentPackageFullName(
            package_full_name_length: *mut u32,
            package_full_name: *mut u16,
        ) -> u32;
    }
}

#[cfg(test)]
trait WinRtHotspotBackend {
    fn package_identity(&mut self) -> Result<PackageIdentityState, PlatformError>;
    fn profiles(&mut self) -> Result<Vec<ProfileDescriptor>, PlatformError>;
    fn capability(&mut self, ordinal: usize) -> Result<CapabilityFixture, PlatformError>;
    fn manager(&mut self, ordinal: usize) -> Result<FakeManager, PlatformError>;

    /// This method exists only so the fake can prove the production selection path never falls
    /// back to a global/default connection profile.
    #[allow(dead_code)]
    fn global_connection_profile(&mut self) -> Result<(), PlatformError>;
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CapabilityFixture {
    raw: u32,
    state: HotspotCapabilityState,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct OperationFixture {
    status: u32,
    completes: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct FakeManager {
    operational_state: u32,
    clients: Result<u32, PlatformError>,
    start_results: std::collections::VecDeque<OperationFixture>,
    stop_results: std::collections::VecDeque<OperationFixture>,
}

#[cfg(test)]
fn select_profile_from_backend<B: WinRtHotspotBackend>(
    backend: &mut B,
    target_adapter_id: &str,
) -> Result<usize, PlatformError> {
    let profiles = backend.profiles()?;
    select_source_profile_index(target_adapter_id, &profiles)
}

#[cfg(test)]
mod test_seam {
    use super::*;

    struct RecordingBackend {
        profile_sets: std::collections::VecDeque<Vec<ProfileDescriptor>>,
        capability_sets: std::collections::VecDeque<CapabilityFixture>,
        manager_sets: std::collections::VecDeque<FakeManager>,
        package_state: PackageIdentityState,
        profile_calls: usize,
        global_calls: usize,
    }

    impl Default for RecordingBackend {
        fn default() -> Self {
            Self {
                profile_sets: std::collections::VecDeque::new(),
                capability_sets: std::collections::VecDeque::new(),
                manager_sets: std::collections::VecDeque::new(),
                package_state: PackageIdentityState::Unpackaged,
                profile_calls: 0,
                global_calls: 0,
            }
        }
    }

    impl WinRtHotspotBackend for RecordingBackend {
        fn package_identity(&mut self) -> Result<PackageIdentityState, PlatformError> {
            Ok(self.package_state)
        }

        fn profiles(&mut self) -> Result<Vec<ProfileDescriptor>, PlatformError> {
            self.profile_calls += 1;
            self.profile_sets.pop_front().ok_or(PlatformError {
                code: "hotspot:profile_enumeration_failed",
                os_code: None,
            })
        }

        fn capability(&mut self, _ordinal: usize) -> Result<CapabilityFixture, PlatformError> {
            self.capability_sets.pop_front().ok_or(PlatformError {
                code: "hotspot:capability_fixture_missing",
                os_code: None,
            })
        }

        fn manager(&mut self, _ordinal: usize) -> Result<FakeManager, PlatformError> {
            self.manager_sets.pop_front().ok_or(PlatformError {
                code: "hotspot:manager_fixture_missing",
                os_code: None,
            })
        }

        fn global_connection_profile(&mut self) -> Result<(), PlatformError> {
            self.global_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn fake_profile_facade_reenumerates_and_never_uses_global_profile() {
        let mut backend = RecordingBackend {
            profile_sets: [
                vec![ProfileDescriptor {
                    ordinal: 2,
                    adapter_id: "00112233-4455-6677-8899-aabbccddeeff".to_owned(),
                }],
                vec![ProfileDescriptor {
                    ordinal: 7,
                    adapter_id: "{00112233-4455-6677-8899-aabbccddeeff}".to_owned(),
                }],
            ]
            .into_iter()
            .collect(),
            ..RecordingBackend::default()
        };

        assert_eq!(
            select_profile_from_backend(&mut backend, "{00112233-4455-6677-8899-aabbccddeeff}"),
            Ok(2)
        );
        assert_eq!(
            select_profile_from_backend(&mut backend, "{00112233-4455-6677-8899-aabbccddeeff}"),
            Ok(7)
        );
        assert_eq!(backend.profile_calls, 2);
        assert_eq!(backend.global_calls, 0);
    }

    #[test]
    fn fake_facade_keeps_package_capability_and_manager_calls_on_the_selected_ordinal() {
        let manager = FakeManager {
            operational_state: 1,
            clients: Ok(2),
            start_results: [OperationFixture {
                status: 0,
                completes: true,
            }]
            .into_iter()
            .collect(),
            stop_results: std::collections::VecDeque::new(),
        };
        let mut backend = RecordingBackend {
            profile_sets: [vec![ProfileDescriptor {
                ordinal: 4,
                adapter_id: "{00112233-4455-6677-8899-aabbccddeeff}".to_owned(),
            }]]
            .into_iter()
            .collect(),
            capability_sets: [CapabilityFixture {
                raw: 0,
                state: HotspotCapabilityState::Enabled,
            }]
            .into_iter()
            .collect(),
            manager_sets: [manager].into_iter().collect(),
            package_state: PackageIdentityState::Unpackaged,
            ..RecordingBackend::default()
        };

        assert_eq!(
            backend.package_identity(),
            Ok(PackageIdentityState::Unpackaged)
        );
        let ordinal =
            select_profile_from_backend(&mut backend, "{00112233-4455-6677-8899-aabbccddeeff}")
                .expect("fixture profile should match exactly");
        let capability = backend
            .capability(ordinal)
            .expect("capability fixture should be available");
        assert_eq!(capability.raw, 0);
        let manager = backend
            .manager(ordinal)
            .expect("manager fixture should be available");
        assert_eq!(manager.operational_state, 1);
        assert_eq!(manager.clients, Ok(2));
        assert_eq!(
            manager.start_results.front().map(|result| result.status),
            Some(0)
        );
        assert_eq!(
            manager.start_results.front().map(|result| result.completes),
            Some(true)
        );
        assert_eq!(backend.global_calls, 0);
    }
}
