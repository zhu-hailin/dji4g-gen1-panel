//! Production composition for the panel process.
//!
//! The application crate owns the port traits, so the concrete Windows implementations live at
//! this higher-level composition boundary.  This keeps the dependency direction one-way while
//! making it impossible for the ordinary executable path to accidentally select the deterministic
//! test controller.
//!
//! Every port below delegates to a real Windows backend.  Privileged repairs fail closed unless an
//! installed, signature-verified helper exists; a development or portable checkout has no such
//! helper and therefore cannot mutate device or network state.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use dji4g_application::{
    ActionExecutor, ActionPreconditions, AdapterContext, AdapterMetrics, AdapterObservationDto,
    AdapterPort, AdapterStateDto, AtControlAvailability, AtObservation, AtPort, Clock,
    DefaultRouteDto, DeviceEpoch, DevicePresenceDto, DeviceToolsPort, ExecutionReceipt,
    ExecutionReceiptOutcome, FailureCode, HotspotControl, HotspotObservation, InventoryObservation,
    InventoryPort, MonitorPorts, MonoTime, NetworkProbePort, PortError, PortFuture,
    PrivilegedExecutor, ProbeObservationDto, ProbeStageDto, SmsListing, SmsPort, SmsSendReceipt,
    SmsSendResult, StableCode, SystemRouteDto, TargetContext, ToolControl, ToolOperation,
    ToolOutcome, ToolReceipt, ToolRequest, ToolTranscript, ValidatedActionToken, mask_recipient,
};
use dji4g_at_protocol::{
    Apn, AtCommand, AtFinalCode, AtResponse, PdpContextId, PdpContextState, ProtocolErrorKind,
    SensorTemperature, ToolParseError, ToolWireRequest, VerifiedUsbNetProfile, parse_cnum_lines,
    parse_iccid_line, parse_pdp_contexts_with_activity, parse_qtemp_lines, parse_serving_cell_line,
};
use dji4g_domain::{
    ActionKind, AdapterBinding, AfterStateHash, DefaultRouteOwner, DnsProfile, ErrorCode,
    FeatureStatus, NumberLookup, OperationOutcome, ProtocolCoverage, SimState, SmsMessage,
    SmsStatus, SmsStorageId, StableDeviceIdentity, UsbNetworkProfile,
};
use dji4g_ipc::{
    BoundedDnsServers, DnsProfileV1, Hash32, HelperActionV1, HelperRequestV1, HelperResponseV1,
    HelperResultV1, OperationCode, OperationNonce, OperationResultV1, PdpContextIdV1,
    ProtocolVersion, RequestId, RequestRejectCode, SupportedProfileV1, TargetProofV1, UnixMillis,
    UsbNetProfileV1, ValidatedApn,
};
use dji4g_windows_platform::{
    AdapterIdentity, AdapterObservation, AddressFamily, BoundProbeResult, DjiDevice,
    EndpointAttempt, InterfaceMetrics, PlatformError, PrivilegeError, ProbePolicy, ProbeStage,
    RepairAction, RepairError, SmsRecord, TlsEvidence, ToolExchangeOutcome, TrustedHelper,
    UnansweredReason, WindowsAdapterResolver, WindowsDeviceInventory, WindowsHotspotControl,
    WindowsNativeRepairBackend, WindowsNetworkProbe, WindowsRepairExecutor, launch_elevated_helper,
    probe_at_port, read_interface_metrics, select_at_port_verified, sms_delete, sms_query_pdu_mode,
    sms_read_verified, sms_set_pdu_mode, tool_exchange,
};

/// Maximum helper request lifetime, bounded by `dji4g_ipc::MAX_OPERATION_LIFETIME`.  The window
/// covers the UAC prompt plus one execution and readback; `validate_request` rejects anything
/// longer, so this is set to the protocol maximum.
const OPERATION_LIFETIME_MS: u64 = 60_000;

/// Display bound for the raw `+QTEMP:` line kept beside a temperature reading.  The line is
/// firmware chatter shown on hover, so it is stored bounded rather than whole.
#[cfg(windows)]
const TEMPERATURE_RAW_MAX_CHARS: usize = 128;

/// A real wall/monotonic clock used by the production controller.
#[derive(Debug)]
pub struct SystemClock {
    started: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn system_now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn monotonic_now(&self) -> MonoTime {
        let millis = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        MonoTime::from_ticks(millis)
    }
}

#[derive(Clone, Debug)]
struct EpochState {
    epoch: DeviceEpoch,
    present: bool,
    identity: Option<StableDeviceIdentity>,
}

impl Default for EpochState {
    fn default() -> Self {
        Self {
            epoch: DeviceEpoch(0),
            present: false,
            identity: None,
        }
    }
}

/// Application-owned inventory adapter backed by a fresh SetupAPI/Configuration Manager scan.
///
/// The platform snapshot is converted into public application DTOs immediately; no `DjiDevice`
/// or platform-owned handle escapes this adapter.  Epoch state is retained only to reject stale
/// evidence after removal, replug, or identity drift.
pub struct ProductionInventory {
    native: WindowsDeviceInventory,
    epoch: Mutex<EpochState>,
}

impl Default for ProductionInventory {
    fn default() -> Self {
        Self {
            native: WindowsDeviceInventory,
            epoch: Mutex::new(EpochState::default()),
        }
    }
}

impl ProductionInventory {
    fn scan_observation(&self) -> Result<InventoryObservation, PortError> {
        let snapshot = self.native.scan_now().map_err(map_platform_error)?;
        let mut state = self
            .epoch
            .lock()
            .map_err(|_| PortError::new(ErrorCode::Internal, "pnp:epoch_state_unavailable"))?;

        let device = match snapshot.devices() {
            [] => {
                if state.present {
                    state.epoch = next_epoch(state.epoch);
                }
                state.present = false;
                state.identity = None;
                return Ok(InventoryObservation {
                    epoch: state.epoch,
                    presence: DevicePresenceDto::NotDetected,
                    identity: None,
                    problem_code: None,
                    at_port: None,
                    adapter_id: None,
                });
            }
            [device] => device,
            many => {
                return Err(PortError::new(
                    ErrorCode::CapabilityUnavailable,
                    if many.len() > 1 {
                        "pnp:ambiguous_device"
                    } else {
                        "pnp:enumerate_failed"
                    },
                ));
            }
        };

        let identity = stable_identity(device);
        if !state.present || state.identity.as_ref() != Some(&identity) {
            state.epoch = next_epoch(state.epoch);
        }
        state.present = true;
        state.identity = Some(identity.clone());

        let at_port = device
            .select_at_port()
            .ok()
            .map(|port| port.port_name().to_owned());
        let adapter_id = unique_adapter_id(device);
        Ok(InventoryObservation {
            epoch: state.epoch,
            presence: DevicePresenceDto::Supported(dji4g_domain::DJI_GEN1),
            identity: Some(identity),
            problem_code: device.problem_code(),
            at_port,
            adapter_id,
        })
    }
}

impl InventoryPort for ProductionInventory {
    fn scan(&self) -> PortFuture<'_, Result<InventoryObservation, PortError>> {
        Box::pin(async move { self.scan_observation() })
    }
}

/// AT port implementation.  The full implementation is intentionally kept behind this
/// application boundary so it must rediscover the exact device before opening a fresh actor.
///
/// The port carries one piece of memory: which COM interface the handshake verified for the
/// current epoch, and which unverified candidates already failed for it.  That is port-selection
/// bookkeeping only — no serial handle, actor, or AT session is ever retained, and an epoch
/// change clears it wholesale on the next observation.
pub struct ProductionAt {
    inventory: Arc<ProductionInventory>,
    selection: Mutex<AtSelectionMemory>,
    /// Optional-feature probe bookkeeping shared with the UI (CNUM/QCCID/serving-cell statuses).
    feature_probe: Arc<Mutex<crate::feature_probe::FeatureProbeState>>,
}

#[derive(Default)]
struct AtSelectionMemory {
    epoch: Option<DeviceEpoch>,
    verified: Option<String>,
    failed: Vec<String>,
}

impl AtSelectionMemory {
    fn reset_for(&mut self, epoch: DeviceEpoch) {
        if self.epoch != Some(epoch) {
            *self = Self {
                epoch: Some(epoch),
                ..Self::default()
            };
        }
    }
}

impl ProductionAt {
    fn observe_current(&self, target: &TargetContext) -> Result<AtObservation, PortError> {
        let device = current_device(&self.inventory, target)?;
        let epoch = target.epoch();
        let mut memory = self
            .selection
            .lock()
            .map_err(|_| PortError::new(ErrorCode::Internal, "at:selection_memory_unavailable"))?;
        memory.reset_for(epoch);

        // A previously handshake-verified port is reused for its epoch; if it vanished with a
        // replug, selection starts over below.
        if let Some(path) = memory.verified.clone() {
            let reused = device
                .com_candidates()
                .iter()
                .find(|candidate| candidate.interface_path() == path)
                .map(|candidate| select_at_port_verified(std::slice::from_ref(candidate), &[], 1));
            match reused {
                Some(Ok(dji4g_windows_platform::AtPortSelection::Classified(port))) => {
                    return observe_at_session(epoch, &port, &self.feature_probe);
                }
                _ => memory.verified = None,
            }
        }

        match device.select_at_port() {
            Ok(selected) => {
                // Classified tiers are deterministic, so they are re-derived every cycle; only
                // handshake-verified ports enter the cache below.
                return observe_at_session(epoch, &selected, &self.feature_probe);
            }
            Err(dji4g_windows_platform::PortSelectionError::NoSafePort) => {
                return Err(PortError::new(
                    ErrorCode::CapabilityUnavailable,
                    "pnp:no_safe_at_port",
                ));
            }
            // Ambiguous: the handshake-verified tier below decides, not a blanket refusal.
            Err(dji4g_windows_platform::PortSelectionError::AmbiguousPort { .. }) => {}
        }

        let selection = select_at_port_verified(device.com_candidates(), &memory.failed, 2)
            .map_err(|_| {
                PortError::new(ErrorCode::CapabilityUnavailable, "pnp:at_port_unverified")
            })?;
        let dji4g_windows_platform::AtPortSelection::HandshakeCandidates(probed) = selection else {
            // Classified selections were already handled by `select_at_port` above.
            return Err(PortError::new(
                ErrorCode::CapabilityUnavailable,
                "pnp:at_port_unverified",
            ));
        };

        let mut newly_failed = Vec::new();
        for port in &probed {
            if probe_at_port(epoch, port).is_ok() {
                memory.verified = Some(port.interface_path().to_owned());
                // The probe actor is dropped above; the observation opens a fresh one and
                // handshakes again.  The duplicate bounded handshake only happens on the
                // cache-miss cycle and keeps every serial handle short-lived.
                return observe_at_session(epoch, port, &self.feature_probe);
            }
            newly_failed.push(port.interface_path().to_owned());
        }
        memory.failed.extend(newly_failed);
        Err(PortError::new(
            ErrorCode::CapabilityUnavailable,
            "pnp:at_port_unverified",
        ))
    }
}

impl AtPort for ProductionAt {
    fn observe(&self, target: &TargetContext) -> PortFuture<'_, Result<AtObservation, PortError>> {
        let target = target.clone();
        Box::pin(async move { self.observe_current(&target) })
    }

    /// A deliberate, provable no-op for AT state.
    ///
    /// [`ProductionAt::observe_current`] opens a brand-new [`dji4g_windows_platform::AtSessionActor`]
    /// for every observation and drops it before returning, and the port retains no serial handle,
    /// actor, or cached AT session.  The only retained state is the port-selection memory keyed by
    /// epoch (which interface path the handshake verified / which candidates failed), which an
    /// epoch change clears wholesale on the next observation.  The
    /// `at_port_invalidate_is_a_stateless_noop` test pins this contract.
    fn invalidate(&self, _epoch: DeviceEpoch) {}
}

/// Adapter resolver implementation.  It never selects by FriendlyName or global route.
pub struct ProductionAdapter {
    inventory: Arc<ProductionInventory>,
    resolver: WindowsAdapterResolver,
    /// Exact GUID → LUID mapping retained from the most recent full resolve.
    ///
    /// [`AdapterPort::read_metrics`] may only address the adapter the reducer has already bound,
    /// and it must never re-enumerate interfaces.  The platform's enumeration-free lookup is
    /// private to the resolver, so the mapping the last resolve computed for the bound adapter is
    /// retained here instead: every resolve overwrites it (so replug and epoch changes are
    /// followed automatically), and a metrics read for a GUID this process has not bound reports
    /// `Unsupported` — an honest gap, never a guessed or substituted interface.
    metrics_luid: Mutex<Option<(String, u64)>>,
}

impl ProductionAdapter {
    fn resolve_current(&self, target: &TargetContext) -> Result<AdapterObservationDto, PortError> {
        let device = current_device(&self.inventory, target)?;
        let observation = self
            .resolver
            .resolve(&device, target.epoch())
            .map_err(map_platform_error)?;
        if let Ok(mut slot) = self.metrics_luid.lock() {
            *slot = Some((
                observation.identity.guid_string(),
                observation.identity.luid(),
            ));
        }
        Ok(adapter_dto(observation, target))
    }

    /// Read-only counter sample for the 1 s rates tick. Delegates to the platform resolver, which
    /// reads only the bound adapter's `GetIfEntry2` octet counters by GUID. An unreadable counter is
    /// reported as an honest `Err` (the reducer derives `None` rates) — never a fabricated zero.
    fn read_byte_counters_now(&self, adapter_id: &str) -> Result<(u64, u64), PortError> {
        self.resolver.read_byte_counters(adapter_id).ok_or_else(|| {
            PortError::new(
                ErrorCode::CapabilityUnavailable,
                "net:rate_counters_unavailable",
            )
        })
    }

    /// Full interface metrics for the bound adapter, addressed by its authoritative GUID.
    ///
    /// The LUID comes from the retained resolve mapping ([`Self::metrics_luid`]); the actual read
    /// is the platform's exact `GetIfEntry2` by LUID, so a failed or missing interface is an
    /// honest `Err` and never a fallback to another adapter.
    fn read_metrics_now(&self, adapter_id: &str) -> Result<AdapterMetrics, PortError> {
        let luid = {
            let mapping = self.metrics_luid.lock().map_err(|_| {
                PortError::new(ErrorCode::Internal, "net:metrics_mapping_unavailable")
            })?;
            bound_luid(mapping.as_ref(), adapter_id)?
        };
        read_interface_metrics(luid)
            .map(adapter_metrics)
            .map_err(map_platform_error)
    }
}

impl AdapterPort for ProductionAdapter {
    fn resolve(
        &self,
        target: &TargetContext,
    ) -> PortFuture<'_, Result<AdapterObservationDto, PortError>> {
        let target = target.clone();
        Box::pin(async move { self.resolve_current(&target) })
    }

    fn read_byte_counters(
        &self,
        adapter_id: &str,
    ) -> PortFuture<'_, Result<(u64, u64), PortError>> {
        let adapter_id = adapter_id.to_owned();
        Box::pin(async move { self.read_byte_counters_now(&adapter_id) })
    }

    fn read_metrics(&self, adapter_id: &str) -> PortFuture<'_, Result<AdapterMetrics, PortError>> {
        let adapter_id = adapter_id.to_owned();
        Box::pin(async move { self.read_metrics_now(&adapter_id) })
    }
}

/// SMS storage boundary backed by the platform's PDU-mode command layer.
///
/// Each operation re-finds the exact device and epoch the caller validated and then delegates one
/// typed, bounded module transaction to the platform; no raw AT string crosses this boundary.
/// PDU decode, multipart reassembly, read-state bookkeeping and the store live above this port
/// (application/UI), so this adapter only maps platform records into domain messages.
pub struct ProductionSms {
    inventory: Arc<ProductionInventory>,
}

impl ProductionSms {
    fn target_device(&self, target: &TargetContext) -> Result<(DjiDevice, DeviceEpoch), PortError> {
        let device = current_device(&self.inventory, target)?;
        Ok((device, target.epoch()))
    }
}

impl SmsPort for ProductionSms {
    fn list_controlled(
        &self,
        target: &TargetContext,
        storage: Option<SmsStorageId>,
        control: dji4g_domain::SmsReadControl,
    ) -> PortFuture<'_, Result<dji4g_application::SmsReadResult, PortError>> {
        let target = target.clone();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            let listing = dji4g_windows_platform::sms_list_controlled(
                &device,
                epoch,
                target.sim_fingerprint(),
                storage,
                control,
            )
            .map_err(map_platform_error)?;
            Ok(dji4g_application::SmsReadResult {
                report: listing.report,
                listing: SmsListing {
                    messages: listing
                        .records
                        .into_iter()
                        .map(|record| sms_message(record, epoch))
                        .collect(),
                    capacity: listing.capacity,
                },
            })
        })
    }
    fn query_pdu_mode(
        &self,
        target: &TargetContext,
    ) -> PortFuture<'_, Result<Option<bool>, PortError>> {
        let target = target.clone();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            sms_query_pdu_mode(&device, epoch).map_err(map_platform_error)
        })
    }

    fn enable_pdu_mode(&self, target: &TargetContext) -> PortFuture<'_, Result<(), PortError>> {
        let target = target.clone();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            sms_set_pdu_mode(&device, epoch).map_err(map_platform_error)
        })
    }

    fn list(&self, target: &TargetContext) -> PortFuture<'_, Result<SmsListing, PortError>> {
        let target = target.clone();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            let listing = dji4g_windows_platform::sms_list_controlled(
                &device,
                epoch,
                target.sim_fingerprint(),
                None,
                dji4g_domain::SmsReadControl::new(Duration::from_secs(60)),
            )
            .map_err(map_platform_error)?;
            Ok(SmsListing {
                messages: listing
                    .records
                    .into_iter()
                    .map(|record| sms_message(record, epoch))
                    .collect(),
                capacity: listing.capacity,
            })
        })
    }

    fn read(
        &self,
        target: &TargetContext,
        index: u32,
    ) -> PortFuture<'_, Result<SmsMessage, PortError>> {
        let target = target.clone();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            let record = sms_read_verified(&device, epoch, target.sim_fingerprint(), index)
                .map_err(map_platform_error)?;
            Ok(sms_message(record, epoch))
        })
    }

    fn delete(&self, target: &TargetContext, index: u32) -> PortFuture<'_, Result<(), PortError>> {
        let target = target.clone();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            sms_delete(&device, epoch, index).map_err(map_platform_error)
        })
    }

    fn delete_checked(
        &self,
        target: &TargetContext,
        fragment: &dji4g_domain::SmsFragmentKey,
        control: dji4g_domain::SmsDeleteControl,
    ) -> PortFuture<'_, dji4g_domain::SmsDeleteReceipt> {
        let target = target.clone();
        let fragment = fragment.clone();
        Box::pin(async move {
            match self.target_device(&target) {
                Ok((device, epoch)) => {
                    dji4g_windows_platform::sms_delete_checked(&device, epoch, &fragment, control)
                }
                Err(error) => dji4g_domain::SmsDeleteReceipt {
                    result: dji4g_domain::SmsDeleteItemResult::Failed,
                    code: Some(error.code.stable.as_str().to_owned()),
                },
            }
        })
    }

    fn send(
        &self,
        target: &TargetContext,
        recipient: &str,
        body: &str,
    ) -> PortFuture<'_, Result<SmsSendReceipt, PortError>> {
        self.send_controlled(
            target,
            recipient,
            body,
            dji4g_domain::SmsTransactionControl::new(dji4g_domain::SMS_SEND_TIMEOUT),
        )
    }

    fn send_controlled(
        &self,
        target: &TargetContext,
        recipient: &str,
        body: &str,
        control: dji4g_domain::SmsTransactionControl,
    ) -> PortFuture<'_, Result<SmsSendReceipt, PortError>> {
        let target = target.clone();
        let recipient = recipient.to_owned();
        let body = body.to_owned();
        Box::pin(async move {
            let (device, epoch) = self.target_device(&target)?;
            let started = std::time::Instant::now();
            let request_id = control.request_id();
            let receipt = dji4g_windows_platform::sms_send_controlled(
                &device, epoch, &recipient, &body, control,
            );
            let result = match receipt.result {
                dji4g_windows_platform::SmsSendResult::Submitted => SmsSendResult::Submitted,
                dji4g_windows_platform::SmsSendResult::Failed => SmsSendResult::Failed,
                dji4g_windows_platform::SmsSendResult::OutcomeUnknown => {
                    SmsSendResult::OutcomeUnknown
                }
            };
            tracing::info!(event = "sms_send_finished", elapsed_ms = started.elapsed().as_millis() as u64,
                result = ?result, failure = ?receipt.failure);
            crate::logging::append_event(&format!(
                "sms_send_finished request_id={request_id} elapsed_ms={} result={result:?} failure={:?}",
                started.elapsed().as_millis(),
                receipt.failure
            ));
            Ok(SmsSendReceipt {
                result,
                recipient_masked: mask_recipient(&recipient),
                failure: receipt.failure,
            })
        })
    }
}
/// Production device-tool port.
///
/// Each request opens its own short-lived, handshake-verified AT session on the port the inventory
/// proves belongs to the target, writes the validated line once, and gives the worker back. The
/// request's frozen context is re-checked against the live target before anything is written: a
/// task that started against one module must never be completed against another.
pub struct ProductionDeviceTools {
    inventory: Arc<ProductionInventory>,
}

impl DeviceToolsPort for ProductionDeviceTools {
    fn execute(
        &self,
        target: &TargetContext,
        request: ToolRequest,
        control: ToolControl,
    ) -> PortFuture<'_, Result<ToolReceipt, PortError>> {
        let target = target.clone();
        Box::pin(async move { self.run(&target, request, control) })
    }
}

impl ProductionDeviceTools {
    fn run(
        &self,
        target: &TargetContext,
        request: ToolRequest,
        control: ToolControl,
    ) -> Result<ToolReceipt, PortError> {
        // The whole task is bound to the device the context names, and the port the target proves.
        if request.context.device_epoch != target.epoch()
            || request.context.identity != *target.identity()
        {
            return Ok(context_changed_receipt(&request));
        }
        if let Some(known) = target.at_port() {
            if request.context.at_port != known {
                return Ok(context_changed_receipt(&request));
            }
        }
        let started = std::time::Instant::now();
        let wire = match &request.operation {
            ToolOperation::Read(id) => ToolWireRequest::from_read(*id),
            ToolOperation::Expert(line) => ToolWireRequest::from_expert(line.clone()),
            // The runner expands a sweep into individual reads before it reaches a port; a bulk
            // request here is an internal wiring error, not something to guess about.
            ToolOperation::ProbeAll => {
                return Err(PortError::new(
                    ErrorCode::Internal,
                    "device_tools:probe_not_expanded",
                ));
            }
        };
        let device = current_device(&self.inventory, target)?;
        let exchange = tool_exchange(&device, target.epoch(), wire, control.clone())
            .map_err(map_platform_error)?;
        let elapsed = started.elapsed();
        let operation = request.operation.kind();
        let receipt = match exchange.outcome {
            ToolExchangeOutcome::Answered(response) => ToolReceipt {
                id: request.id,
                context: request.context.clone(),
                operation,
                outcome: ToolOutcome::from_final_code(&response.final_code),
                elapsed,
                transcript: Arc::new(dji4g_application::transcript_from_response(&response)),
                saw_final_code: true,
                payload_lines: response.lines.len(),
            },
            ToolExchangeOutcome::Malformed(error) => ToolReceipt {
                id: request.id,
                context: request.context.clone(),
                operation,
                outcome: malformed_outcome(error),
                elapsed,
                transcript: Arc::new(ToolTranscript::new()),
                saw_final_code: false,
                payload_lines: 0,
            },
            ToolExchangeOutcome::Unanswered { wrote, reason } => ToolReceipt {
                id: request.id,
                context: request.context.clone(),
                operation,
                outcome: unanswered_outcome(wrote, reason),
                elapsed,
                transcript: Arc::new(ToolTranscript::new()),
                saw_final_code: false,
                payload_lines: 0,
            },
        };
        Ok(receipt)
    }
}

/// A request whose device context no longer matches the live target. Nothing was written.
fn context_changed_receipt(request: &ToolRequest) -> ToolReceipt {
    ToolReceipt {
        id: request.id,
        context: request.context.clone(),
        operation: request.operation.kind(),
        outcome: ToolOutcome::ContextChanged,
        elapsed: std::time::Duration::ZERO,
        transcript: Arc::new(ToolTranscript::new()),
        saw_final_code: false,
        payload_lines: 0,
    }
}

/// A response this tool path cannot read as one text exchange.
///
/// The request was written but no final code was parsed. A prompt, CONNECT or broken response
/// cannot prove a refusal or that the command had no effect. The session is retired, without retry.
fn malformed_outcome(error: ToolParseError) -> ToolOutcome {
    match error {
        ToolParseError::UnsupportedInteraction
        | ToolParseError::LineTooLong
        | ToolParseError::ResponseTooLarge
        | ToolParseError::TooManyLines
        | ToolParseError::UnexpectedData => ToolOutcome::OutcomeUnknown,
    }
}

#[cfg(test)]
mod tool_outcome_regressions {
    use super::*;

    #[test]
    fn malformed_response_without_a_final_code_does_not_claim_a_known_effect() {
        for error in [
            ToolParseError::UnsupportedInteraction,
            ToolParseError::LineTooLong,
            ToolParseError::ResponseTooLarge,
            ToolParseError::TooManyLines,
            ToolParseError::UnexpectedData,
        ] {
            assert_eq!(malformed_outcome(error), ToolOutcome::OutcomeUnknown);
        }
    }
}

/// No usable answer arrived. Whether anything was written decides between "nothing happened" and
/// "the effect is unknown", and the second case is never retried automatically.
fn unanswered_outcome(wrote: bool, reason: UnansweredReason) -> ToolOutcome {
    if reason == UnansweredReason::SessionUnavailable && !wrote {
        return ToolOutcome::TransportFailure;
    }
    if wrote {
        return ToolOutcome::OutcomeUnknown;
    }
    match reason {
        UnansweredReason::Cancelled => ToolOutcome::CancelledBeforeWrite,
        UnansweredReason::Deadline | UnansweredReason::Transport => ToolOutcome::TransportFailure,
        UnansweredReason::SessionUnavailable => ToolOutcome::TransportFailure,
    }
}

/// Map one platform PDU-mode storage record into the domain message.
///
/// The PDU itself carries no read state, so the `CMGL`/`CMGR` `<stat>` index is the only
/// evidence: TS 27.005 defines 0 = received unread and 1 = received read.  Retrieval of a listed
/// or read message may itself mark it read, so this is evidence, never a guarantee.
///
/// `sim_epoch` is not knowable from the port signature (it carries no snapshot): messages leave
/// this boundary with 0 and the application ingest stamps the exact SIM epoch when it stores
/// them.  Multipart reassembly is application-side, so every record is `Received` here; an
/// incomplete long message is derived later from the fragment metadata.
fn sms_message(record: SmsRecord, device_epoch: DeviceEpoch) -> SmsMessage {
    let mut message = SmsMessage::new(
        record.index,
        SmsStorageId(record.storage),
        device_epoch.0,
        0,
        record.decoded.sender,
        record.decoded.body,
        record.decoded.encoding,
        SmsStatus::Received,
    );
    message.service_centre_timestamp = record.decoded.timestamp;
    message.multipart = record.decoded.multipart;
    message.read = Some(record.stat == 1);
    message
}

/// Adapter-bound network probe implementation.
///
/// When active probing is enabled it resolves the exact adapter for the bound target and only then
/// runs the bound probe against that precise [`AdapterIdentity`]; a GUID mismatch between the
/// freshly resolved adapter and the bound adapter id is treated as identity drift and fails closed.
/// When active probing is disabled it performs no network I/O and reports an explicit `Unexecuted`
/// state for every stage.  The global default route is carried only as explanation and is never
/// treated as proof that the module itself is online.
pub struct ProductionProbe {
    inventory: Arc<ProductionInventory>,
    resolver: WindowsAdapterResolver,
}

impl NetworkProbePort for ProductionProbe {
    fn observe(
        &self,
        adapter: &AdapterContext,
        active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, PortError>> {
        let adapter = adapter.clone();
        Box::pin(async move {
            if !active {
                let code = probe_failure(ErrorCode::ProbeFailed, "probe:disabled_by_setting");
                return Ok(ProbeObservationDto {
                    route_choices: Vec::new(),
                    epoch: adapter.epoch(),
                    adapter_id: adapter.binding().adapter_id.clone(),
                    gateway: ProbeStageDto::Unexecuted { code: code.clone() },
                    public: ProbeStageDto::Unexecuted { code: code.clone() },
                    dns: ProbeStageDto::Unexecuted { code },
                    protocol_coverage: None,
                    system_route: None,
                });
            }
            let target = adapter.target_context()?;
            let device = current_device(&self.inventory, &target)?;
            let resolved = self
                .resolver
                .resolve(&device, adapter.epoch())
                .map_err(map_platform_error)?;
            if !same_guid(
                &resolved.identity.guid_string(),
                &adapter.binding().adapter_id,
            ) {
                return Err(PortError::new(
                    ErrorCode::DeviceIdentityChanged,
                    "probe:route_identity_mismatch",
                ));
            }
            let probe = WindowsNetworkProbe
                .observe_now(&resolved.identity, &ProbePolicy::default())
                .map_err(map_platform_error)?;
            Ok(probe_dto(probe))
        })
    }
}

/// Package-aware hotspot implementation.  Observation and the in-process toggle stay behind the
/// typed WinRT control; the before-state proof is produced by the same native repair executor the
/// elevated helper runs, so it is identical to the execution side rather than a fabricated value.
pub struct ProductionHotspot {
    inventory: Arc<ProductionInventory>,
    resolver: WindowsAdapterResolver,
    control: WindowsHotspotControl,
}

impl HotspotControl for ProductionHotspot {
    fn observe(
        &self,
        adapter: Option<&AdapterContext>,
    ) -> PortFuture<'_, Result<HotspotObservation, PortError>> {
        let adapter = adapter.cloned();
        Box::pin(async move {
            let adapter = adapter.ok_or_else(|| {
                PortError::new(ErrorCode::CapabilityUnavailable, "app:hotspot_unavailable")
            })?;
            let identity = self.resolve_identity(&adapter)?;
            let status = self
                .control
                .status(&identity)
                .await
                .map_err(map_platform_error)?;
            Ok(HotspotObservation {
                status: status.status,
            })
        })
    }

    fn revalidate_toggle(
        &self,
        target: &TargetContext,
        enabled: bool,
    ) -> PortFuture<'_, Result<ActionPreconditions, PortError>> {
        let epoch = target.epoch();
        Box::pin(async move {
            // Derive the before-state hash from a fresh native observation of the exact hotspot
            // source profile, capability, and status, using the same executor and hashing the
            // elevated helper applies at execution time.  No fixed/zero hash is ever returned: if
            // the observation cannot be produced, this fails closed.
            let executor =
                WindowsRepairExecutor::new(WindowsNativeRepairBackend::with_epoch(epoch));
            let plan = executor
                .prepare(RepairAction::ToggleHotspot { enabled })
                .map_err(map_repair_error)?;
            if plan.epoch() != epoch {
                return Err(PortError::new(
                    ErrorCode::EvidenceExpired,
                    "hotspot:epoch_changed",
                ));
            }
            Ok(ActionPreconditions {
                epoch: plan.epoch(),
                before_state_hash: plan.before_state_hash(),
            })
        })
    }

    fn set_enabled_once(
        &self,
        token: &ValidatedActionToken,
        enabled: bool,
    ) -> PortFuture<'_, Result<ExecutionReceipt, PortError>> {
        // The confirmed toggle executes through the ActionExecution port, which runs this action
        // in process (`execute_hotspot_in_process`); the WinRT tethering future is not `Send`, so
        // it can never be driven inside this `Send` port future directly.  The token carries the
        // authoritative direction, so `enabled` is only cross-checked here and a mismatch fails
        // closed. This must stay on the in-process path: the hotspot toggle needs no elevation,
        // and routing it through the privileged helper would break it in every unsigned build.
        let token = token.clone();
        Box::pin(async move {
            let matches_direction = matches!(
                token.action(),
                ActionKind::ToggleHotspot { enabled: direction } if *direction == enabled
            );
            if !matches_direction {
                return Err(PortError::new(
                    ErrorCode::VerificationFailed,
                    "hotspot:toggle_direction_mismatch",
                ));
            }
            execute_hotspot_in_process(&self.inventory, &token)
        })
    }
}

impl ProductionHotspot {
    fn resolve_identity(&self, adapter: &AdapterContext) -> Result<AdapterIdentity, PortError> {
        let target = adapter.target_context()?;
        let device = current_device(&self.inventory, &target)?;
        let resolved = self
            .resolver
            .resolve(&device, adapter.epoch())
            .map_err(map_platform_error)?;
        if !same_guid(
            &resolved.identity.guid_string(),
            &adapter.binding().adapter_id,
        ) {
            return Err(PortError::new(
                ErrorCode::DeviceIdentityChanged,
                "hotspot:source_profile_unavailable",
            ));
        }
        Ok(resolved.identity)
    }
}

#[cfg(windows)]
fn observe_at_session(
    epoch: DeviceEpoch,
    selected: &dji4g_windows_platform::SelectedPort,
    probe_state: &Arc<Mutex<crate::feature_probe::FeatureProbeState>>,
) -> Result<AtObservation, PortError> {
    let actor = dji4g_windows_platform::AtSessionActor::open_selected(epoch, selected)
        .map_err(|error| map_actor_error(&error))?;
    let identity = actor
        .safe_handshake()
        .map_err(|error| map_actor_error(&error))?;
    if identity.is_empty() {
        return Err(PortError::new(
            ErrorCode::VerificationFailed,
            "at:identity_empty",
        ));
    }

    let sim_response = execute_at_read(&actor, AtCommand::SimState)?;
    let signal_response = execute_at_read(&actor, AtCommand::SignalQuality)?;
    let operator_response = execute_at_read(&actor, AtCommand::Operator)?;
    let registration_response = execute_at_read(&actor, AtCommand::EpsRegistration)?;
    let attach_response = execute_at_read(&actor, AtCommand::PacketAttach)?;
    let contexts_response = execute_at_read(&actor, AtCommand::PdpContexts)?;
    let activity_response = execute_at_read(&actor, AtCommand::PdpActivation)?;
    let address_response = execute_at_read(&actor, AtCommand::PdpAddresses)?;
    // Firmware identity is diagnostic garnish: a module that refuses CGMR degrades the detail
    // instead of failing the whole observation.  The optional CNUM/QCCID/serving-cell probes are
    // classified individually below (research §8.1): a refused, mistimed or malformed optional
    // query must never fail the observation or be silently conflated with 「no data」.
    let revision_response = execute_at_read(&actor, AtCommand::Revision).ok();

    let sim_state = parse_sim_state(&sim_response)?;
    let sim_ready = sim_state == SimState::Ready;
    // The CNUM/QCCID reads require an initialised SIM (research §4.1).  When the SIM is not
    // ready the probes are skipped with an honest `TemporarilyUnavailable` classification
    // instead of being sent and guessed at.
    let (numbers_status, numbers_probe) = feature_probe(
        sim_ready,
        || actor.execute(AtCommand::SubscriberNumber),
        parse_cnum_probe,
    );
    let (iccid_status, iccid_probe) = feature_probe(
        sim_ready,
        || actor.execute(AtCommand::Iccid),
        parse_iccid_probe,
    );
    // A successful CNUM with no records is `NumberLookup::Empty` — the model's dedicated
    // `Empty` status (research §8.1: 命令成功，但当前无数据), never `Supported`.
    let numbers_status = if matches!(numbers_probe, Some(NumberLookup::Empty)) {
        FeatureStatus::Empty
    } else {
        numbers_status
    };
    let (serving_cell_status, serving_cell_probe) = feature_probe(
        true,
        || actor.execute(AtCommand::ServingCellInfo),
        parse_serving_cell_probe,
    );
    let (serving_cell, serving_cell_raw) = match serving_cell_probe {
        Some((cell, raw)) => (Some(cell), Some(raw)),
        None => (None, None),
    };
    // Module temperature is another optional, firmware-defined probe (research §7.5).  QTEMP is a
    // module-level query rather than a SIM read, so it runs whenever the handshake succeeded; a
    // missing command, a refusal or an unreadable layout degrades exactly like the other optional
    // probes and never fails the observation.  Only the first reported value is stored as the row's
    // value — the sensor layout and thresholds stay firmware-defined, so the UI labels the row as a
    // module reading and shows the readings it came from, never a case-temperature claim.
    let (temperature_status, temperature_probe) = feature_probe(
        true,
        || actor.execute(AtCommand::Temperature),
        parse_temperature_probe,
    );
    let (temperature_celsius, temperature_sensors, temperature_raw) = match temperature_probe {
        Some(probe) => (Some(probe.celsius), probe.sensors, probe.raw),
        None => (None, Vec::new(), None),
    };

    let primary_context = parse_pdp_for_display(&contexts_response, &activity_response)?;
    let primary = primary_context.as_ref();
    let firmware = revision_response.and_then(|response| {
        response
            .lines
            .first()
            .map(|line| line.chars().take(64).collect::<String>())
    });
    let observation = AtObservation {
        availability: AtControlAvailability::Available,
        cellular: Some(dji4g_domain::CellularSnapshot {
            sim: sim_state,
            registration: parse_registration_state(&registration_response)?,
            attached: parse_attach_state(&attach_response)?,
            carrier: parse_operator(&operator_response).0,
            radio_access_technology: parse_operator(&operator_response).1,
            signal_rssi_dbm: parse_signal(&signal_response)?,
            apn: primary.map(|context| context.apn().as_str().to_owned()),
            pdp_address: parse_pdp_address(&address_response),
            pdp_state: primary.map(|context| match context.state() {
                PdpContextState::Active => "active".to_owned(),
                PdpContextState::Inactive => "inactive".to_owned(),
            }),
            firmware,
            serving_cell,
            sim_identity: iccid_probe
                .as_deref()
                .map(crate::feature_probe::sim_identity_for),
            numbers: numbers_probe,
            temperature_celsius,
            temperature_status,
        }),
    };
    // Mirror the classified probes plus the exact values stored into this snapshot so the UI can
    // explain these rows and only these rows.  Best-effort: a poisoned slot must never fail the
    // observation, and the record is naturally overwritten by the next cycle.
    if let Ok(mut slot) = probe_state.lock() {
        *slot = crate::feature_probe::FeatureProbeState {
            epoch,
            numbers: observation
                .cellular
                .as_ref()
                .and_then(|cell| cell.numbers.clone()),
            sim_identity: observation
                .cellular
                .as_ref()
                .and_then(|cell| cell.sim_identity.clone()),
            serving_cell: observation
                .cellular
                .as_ref()
                .and_then(|cell| cell.serving_cell.clone()),
            numbers_status,
            iccid_status,
            serving_cell_status,
            iccid_full: iccid_probe,
            serving_cell_raw,
            temperature_celsius,
            temperature_sensors,
            temperature_raw,
        };
    }
    Ok(observation)
}

#[cfg(windows)]
fn parse_pdp_for_display(
    contexts: &AtResponse,
    activity: &AtResponse,
) -> Result<Option<dji4g_at_protocol::PdpContext>, PortError> {
    use dji4g_at_protocol::PdpParseError;
    match parse_pdp_contexts_with_activity(contexts, activity) {
        Ok(contexts) => Ok(contexts.into_iter().next()),
        Err(error @ (PdpParseError::WrongEpoch | PdpParseError::WrongCommand)) => {
            Err(PortError::new(
                ErrorCode::VerificationFailed,
                match error {
                    PdpParseError::WrongEpoch => "at:pdp_epoch_mismatch",
                    _ => "at:pdp_wrong_command",
                },
            ))
        }
        Err(error) => {
            // This read-only display is not APN repair: unknown PDP details do not invalidate
            // the completed handshake or independently parsed SIM/registration observations.
            // Keep the strict repair parser and record only a content-free typed error.
            crate::logging::append_event(&format!("pdp_display_unavailable code={error}"));
            Ok(None)
        }
    }
}

#[cfg(not(windows))]
fn observe_at_session(
    _epoch: DeviceEpoch,
    _selected: &dji4g_windows_platform::SelectedPort,
    _feature_probe: &Arc<Mutex<crate::feature_probe::FeatureProbeState>>,
) -> Result<AtObservation, PortError> {
    Err(PortError::new(
        ErrorCode::CapabilityUnavailable,
        "at:unsupported_platform",
    ))
}

/// Run one optional read-only probe with outcome classification (research §8.1/§10).
///
/// The probe is executed only when the SIM is ready; otherwise it is classified
/// `TemporarilyUnavailable` without ever touching the serial session.  A successful response is
/// handed to `parse`, whose verdict decides between `Supported`/`Empty` (a recognised value or a
/// clean no-data response) and `FormatMismatch` (a response that is not in the recognised
/// layout).  A transport failure is mapped by [`map_feature_failure`].  No outcome of this
/// function may fail the observation that ran the probe.
#[cfg(windows)]
fn feature_probe<T>(
    sim_ready: bool,
    execute: impl FnOnce() -> Result<AtResponse, dji4g_windows_platform::ActorError>,
    parse: impl FnOnce(&AtResponse) -> Result<Option<T>, ()>,
) -> (FeatureStatus, Option<T>) {
    if !sim_ready {
        return (FeatureStatus::TemporarilyUnavailable, None);
    }
    match execute() {
        Ok(response) => match parse(&response) {
            Ok(Some(value)) => (FeatureStatus::Supported, Some(value)),
            Ok(None) => (FeatureStatus::Empty, None),
            Err(()) => (FeatureStatus::FormatMismatch, None),
        },
        Err(error) => (map_feature_failure(&error), None),
    }
}

/// Classify one failed optional probe.  Only an explicit `+CME ERROR` with the 3GPP
/// "operation not supported" code (4) counts as confirmed unsupported; a generic `ERROR` or any
/// other CME code is ambiguous and degrades to `TemporarilyUnavailable` (research §8.1: an ERROR
/// must never be read as "not supported" without interpretable evidence).
#[cfg(windows)]
fn map_feature_failure(error: &dji4g_windows_platform::ActorError) -> FeatureStatus {
    match error {
        dji4g_windows_platform::ActorError::FinalCode(AtFinalCode::CmeError(detail)) => {
            if cme_code_is_unsupported(detail) {
                FeatureStatus::UnsupportedConfirmed
            } else {
                FeatureStatus::TemporarilyUnavailable
            }
        }
        dji4g_windows_platform::ActorError::FinalCode(
            AtFinalCode::Error | AtFinalCode::CmsError(_),
        )
        | dji4g_windows_platform::ActorError::QueueFull => FeatureStatus::TemporarilyUnavailable,
        dji4g_windows_platform::ActorError::FinalCode(
            AtFinalCode::Ok
            | AtFinalCode::NoCarrier
            | AtFinalCode::NoAnswer
            | AtFinalCode::Busy
            | AtFinalCode::NoDialTone,
        ) => FeatureStatus::TemporarilyUnavailable,
        dji4g_windows_platform::ActorError::Protocol(error) => match error.kind {
            ProtocolErrorKind::Timeout | ProtocolErrorKind::DeviceRemoved => {
                FeatureStatus::TransportFailure
            }
            ProtocolErrorKind::WrongPortData
            | ProtocolErrorKind::LineTooLong
            | ProtocolErrorKind::ResponseTooLarge
            | ProtocolErrorKind::UnexpectedData => FeatureStatus::FormatMismatch,
        },
        dji4g_windows_platform::ActorError::Io(_)
        | dji4g_windows_platform::ActorError::Closed
        | dji4g_windows_platform::ActorError::LeaseBusy
        | dji4g_windows_platform::ActorError::CloseTimeout
        | dji4g_windows_platform::ActorError::OsIo { .. } => FeatureStatus::TransportFailure,
        // The feature probe never issues a tool transaction; the arm keeps the mapping total.
        dji4g_windows_platform::ActorError::Tool(_) => FeatureStatus::FormatMismatch,
    }
}

/// 3GPP/TS 27.007 `+CME ERROR` code 4 is "operation not supported" — the interpretable evidence
/// the classification requires.  The detail text is the raw error payload (e.g. `4`); only a
/// trailing numeric code of exactly 4 confirms unsupported.
#[cfg(windows)]
fn cme_code_is_unsupported(detail: &str) -> bool {
    let trailing_digits = detail
        .trim()
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    trailing_digits.parse::<u16>().ok() == Some(4)
}

/// Parse a successful CNUM response.  An empty `OK` (no data lines) is a valid
/// [`NumberLookup::Empty`] — never a failure (research §4.1).
#[cfg(windows)]
fn parse_cnum_probe(response: &AtResponse) -> Result<Option<NumberLookup>, ()> {
    let lines = response
        .lines
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    parse_cnum_lines(&lines).map(Some).map_err(|_| ())
}

/// Parse a successful QCCID response.  An `OK` without a `+QCCID:` line is out of the vendor
/// layout and counts as `FormatMismatch` (the response carries no record to read); a totally
/// empty response is `Empty` only if the stream itself had no lines.
#[cfg(windows)]
fn parse_iccid_probe(response: &AtResponse) -> Result<Option<String>, ()> {
    match response
        .lines
        .iter()
        .find_map(|line| parse_iccid_line(line))
    {
        Some(iccid) => Ok(Some(iccid)),
        None if response.lines.is_empty() => Ok(None),
        None => Err(()),
    }
}

/// Parse a successful serving-cell response into the parsed cell plus the raw reported line that
/// produced it (the raw line is retained UI-locally for hover detail, never exported).
#[cfg(windows)]
fn parse_serving_cell_probe(
    response: &AtResponse,
) -> Result<Option<(dji4g_domain::ServingCell, String)>, ()> {
    if response.lines.is_empty() {
        return Ok(None);
    }
    response
        .lines
        .iter()
        .find_map(|line| parse_serving_cell_line(line).map(|cell| (cell, line.clone())))
        .map(Some)
        .ok_or(())
}

/// A successful QTEMP probe: the value the overview shows plus the evidence its rows explain
/// themselves with.
#[cfg(windows)]
#[derive(Clone, Debug, PartialEq)]
struct TemperatureProbe {
    /// The first reported value — the module temperature the overview row shows.  Which sensor that
    /// is stays firmware-defined, so the UI never claims more than "the first reported value".
    celsius: i16,
    /// Every reading of this observation, in report order, for the 「模块温度」 area.
    sensors: Vec<SensorTemperature>,
    /// The raw `+QTEMP:` line as reported, for the row's hover detail.
    raw: Option<String>,
}

/// Parse a successful QTEMP response.  The first reported value becomes the module temperature
/// (research §7.5), while the readings and the raw line travel with it so the UI can say exactly
/// what the module sent instead of guessing.  A response with content but no readable reading is a
/// format mismatch; a genuinely empty response is a clean empty result.
#[cfg(windows)]
fn parse_temperature_probe(response: &AtResponse) -> Result<Option<TemperatureProbe>, ()> {
    let lines = response
        .lines
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let sensors = parse_qtemp_lines(&lines);
    match sensors.first() {
        Some(first) => Ok(Some(TemperatureProbe {
            celsius: first.celsius,
            raw: temperature_raw_line(response),
            sensors,
        })),
        None if response.lines.is_empty() => Ok(None),
        None => Err(()),
    }
}

/// The first raw `+QTEMP:` line of a response, bounded for display.  It is UI-local detail — never
/// logged, serialized or exported — and a longer line is cut on a character boundary.
#[cfg(windows)]
fn temperature_raw_line(response: &AtResponse) -> Option<String> {
    let line = response
        .lines
        .iter()
        .map(String::as_str)
        .find(|line| line.trim_start().starts_with("+QTEMP:"))?;
    let mut chars = line.chars();
    let mut raw: String = chars.by_ref().take(TEMPERATURE_RAW_MAX_CHARS).collect();
    if chars.next().is_some() {
        raw.push('…');
    }
    Some(raw)
}

#[cfg(windows)]
fn execute_at_read(
    actor: &dji4g_windows_platform::AtSessionActor,
    command: AtCommand,
) -> Result<AtResponse, PortError> {
    actor
        .execute(command)
        .map_err(|error| map_actor_error(&error))
}

#[cfg(windows)]
fn map_actor_error(error: &dji4g_windows_platform::ActorError) -> PortError {
    let category = match error.protocol_kind() {
        Some(ProtocolErrorKind::Timeout) => ErrorCode::Timeout,
        Some(ProtocolErrorKind::DeviceRemoved) => ErrorCode::DeviceRemoved,
        Some(_) => ErrorCode::VerificationFailed,
        None => match error {
            dji4g_windows_platform::ActorError::QueueFull
            | dji4g_windows_platform::ActorError::LeaseBusy
            | dji4g_windows_platform::ActorError::CloseTimeout
            | dji4g_windows_platform::ActorError::OsIo { .. } => ErrorCode::CapabilityUnavailable,
            dji4g_windows_platform::ActorError::Closed
            | dji4g_windows_platform::ActorError::Io(_) => ErrorCode::CapabilityUnavailable,
            dji4g_windows_platform::ActorError::FinalCode(_) => ErrorCode::VerificationFailed,
            dji4g_windows_platform::ActorError::Protocol(_) => ErrorCode::VerificationFailed,
            dji4g_windows_platform::ActorError::Tool(_) => ErrorCode::VerificationFailed,
        },
    };
    PortError::new(category, error.code())
}

#[cfg(windows)]
fn parse_sim_state(response: &AtResponse) -> Result<dji4g_domain::SimState, PortError> {
    let value = response_value(response, "+CPIN:")?.to_ascii_uppercase();
    Ok(match value.as_str() {
        "READY" => dji4g_domain::SimState::Ready,
        "SIM PIN" => dji4g_domain::SimState::PinRequired,
        "SIM PUK" => dji4g_domain::SimState::PukRequired,
        "SIM NOT INSERTED" | "NOT INSERTED" => dji4g_domain::SimState::Missing,
        "SIM REJECTED" => dji4g_domain::SimState::Rejected,
        _ => dji4g_domain::SimState::Unknown,
    })
}

#[cfg(windows)]
fn parse_signal(response: &AtResponse) -> Result<Option<i16>, PortError> {
    let value = response_value(response, "+CSQ:")?;
    let raw = value
        .split(',')
        .next()
        .ok_or_else(|| PortError::new(ErrorCode::VerificationFailed, "at:signal_parse_failed"))?;
    let level = raw
        .trim()
        .parse::<u8>()
        .map_err(|_| PortError::new(ErrorCode::VerificationFailed, "at:signal_parse_failed"))?;
    Ok((level <= 31).then_some(-113 + i16::from(level) * 2))
}

#[cfg(windows)]
fn parse_operator(response: &AtResponse) -> (Option<String>, Option<String>) {
    let Some(value) = response
        .lines
        .iter()
        .find_map(|line| line.strip_prefix("+COPS:"))
    else {
        return (None, None);
    };
    let fields = split_at_csv(value);
    let carrier = fields
        .get(2)
        .and_then(|field| field.trim().strip_prefix('"'))
        .and_then(|field| field.strip_suffix('"'))
        .filter(|field| !field.is_empty())
        .map(str::to_owned);
    let rat = fields
        .get(3)
        .and_then(|field| field.trim().parse::<u8>().ok())
        .map(|value| match value {
            7 => "LTE".to_owned(),
            13 => "NR".to_owned(),
            other => format!("RAT {other}"),
        });
    (carrier, rat)
}

#[cfg(windows)]
fn parse_registration_state(
    response: &AtResponse,
) -> Result<dji4g_domain::RegistrationState, PortError> {
    let value = response_value(response, "+CEREG:")?;
    let status = value
        .split(',')
        .next_back()
        .ok_or_else(|| {
            PortError::new(
                ErrorCode::VerificationFailed,
                "at:registration_parse_failed",
            )
        })?
        .trim()
        .parse::<u8>()
        .map_err(|_| {
            PortError::new(
                ErrorCode::VerificationFailed,
                "at:registration_parse_failed",
            )
        })?;
    Ok(match status {
        1 => dji4g_domain::RegistrationState::RegisteredHome,
        5 => dji4g_domain::RegistrationState::RegisteredRoaming,
        2 => dji4g_domain::RegistrationState::Searching,
        3 => dji4g_domain::RegistrationState::Denied,
        0 | 4 => dji4g_domain::RegistrationState::NotRegistered,
        _ => dji4g_domain::RegistrationState::Unknown,
    })
}

#[cfg(windows)]
fn parse_attach_state(response: &AtResponse) -> Result<dji4g_domain::AttachState, PortError> {
    let value = response_value(response, "+CGATT:")?;
    Ok(match value.trim() {
        "1" => dji4g_domain::AttachState::Attached,
        "0" => dji4g_domain::AttachState::Detached,
        _ => dji4g_domain::AttachState::Unknown,
    })
}

#[cfg(windows)]
fn response_value<'a>(response: &'a AtResponse, prefix: &str) -> Result<&'a str, PortError> {
    response
        .lines
        .iter()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
        .ok_or_else(|| PortError::new(ErrorCode::VerificationFailed, "at:response_missing"))
}

#[cfg(windows)]
fn split_at_csv(value: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                fields.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(value[start..].trim());
    fields
}

#[cfg(windows)]
fn parse_pdp_address(response: &AtResponse) -> Option<String> {
    response.lines.iter().find_map(|line| {
        let value = line.strip_prefix("+CGPADDR:")?;
        let address = value.split(',').nth(1)?.trim().trim_matches('"');
        address
            .parse::<std::net::IpAddr>()
            .ok()
            .map(|_| address.to_owned())
    })
}

/// Production action boundary used by both the synchronous controller path and the privileged
/// async path.  It re-finds the exact target, recomputes the authoritative identity/before-state
/// proof with the same native executor the helper runs, and fails closed unless an installed,
/// signature-verified helper exists.  It never substitutes a fake executor or a fixed error.
pub struct ProductionActionExecutor {
    inventory: Arc<ProductionInventory>,
}

impl ProductionActionExecutor {
    fn execute(&self, token: &ValidatedActionToken) -> Result<ExecutionReceipt, PortError> {
        // The hotspot toggle needs no elevation (WinRT tethering control), so it executes in
        // process through the same native executor the helper would run. Every other action
        // keeps the authenticated helper path and fails closed without a signed install.
        if executes_in_process(token.action()) {
            execute_hotspot_in_process(&self.inventory, token)
        } else {
            execute_via_helper(&self.inventory, token)
        }
    }
}

impl ActionExecutor for ProductionActionExecutor {
    fn execute_once(&self, token: ValidatedActionToken) -> Result<ExecutionReceipt, PortError> {
        self.execute(&token)
    }
}

impl PrivilegedExecutor for ProductionActionExecutor {
    fn execute_once(
        &self,
        token: ValidatedActionToken,
    ) -> PortFuture<'_, Result<ExecutionReceipt, PortError>> {
        Box::pin(async move { self.execute(&token) })
    }
}

/// The complete non-demo controller/port composition used by debug and release executables.
pub struct ProductionComposition {
    controller: dji4g_application::Controller,
    ports: MonitorPorts,
    feature_probe: Arc<Mutex<crate::feature_probe::FeatureProbeState>>,
}

impl ProductionComposition {
    #[must_use]
    pub fn new(now: SystemTime) -> Self {
        let inventory = Arc::new(ProductionInventory::default());
        let resolver = WindowsAdapterResolver;
        let feature_probe = Arc::new(Mutex::new(
            crate::feature_probe::FeatureProbeState::default(),
        ));
        let controller = dji4g_application::Controller::new(
            dji4g_application::ReducerState::new(now),
            Arc::new(ProductionActionExecutor {
                inventory: Arc::clone(&inventory),
            }),
            Arc::new(SystemClock::default()),
        );
        let ports = MonitorPorts {
            inventory: Arc::clone(&inventory) as Arc<dyn InventoryPort>,
            at: Arc::new(ProductionAt {
                inventory: Arc::clone(&inventory),
                selection: Mutex::default(),
                feature_probe: Arc::clone(&feature_probe),
            }),
            adapter: Arc::new(ProductionAdapter {
                inventory: Arc::clone(&inventory),
                resolver,
                metrics_luid: Mutex::new(None),
            }),
            probe: Arc::new(ProductionProbe {
                inventory: Arc::clone(&inventory),
                resolver,
            }),
            hotspot: Some(Arc::new(ProductionHotspot {
                inventory: Arc::clone(&inventory),
                resolver,
                control: WindowsHotspotControl::new(),
            })),
            sms: Some(Arc::new(ProductionSms {
                inventory: Arc::clone(&inventory),
            })),
            device_tools: Some(Arc::new(ProductionDeviceTools {
                inventory: Arc::clone(&inventory),
            })),
        };
        Self {
            controller,
            ports,
            feature_probe,
        }
    }

    #[must_use]
    pub fn has_all_real_ports(&self) -> bool {
        self.ports.hotspot.is_some()
    }

    /// The optional-probe record shared with the UI.  Call before `into_parts`; the composition
    /// is consumed there.
    #[must_use]
    pub fn feature_probe_state(&self) -> Arc<Mutex<crate::feature_probe::FeatureProbeState>> {
        Arc::clone(&self.feature_probe)
    }

    #[must_use]
    pub fn into_parts(self) -> (dji4g_application::Controller, MonitorPorts) {
        (self.controller, self.ports)
    }
}

/// Drive one confirmed action through the authenticated, one-shot elevated helper.
///
/// The flow is deliberately fail-closed at every step: re-find the exact target, map the closed
/// application action onto the typed repair action and the closed helper request variant, recompute
/// the authoritative identity/before-state proof from a fresh native observation, require an
/// installed signature-verified helper, then send exactly one bounded request and map the helper's
/// three-state result back.  A user-cancelled UAC prompt and any untrusted/absent helper produce a
/// definite failure with no automatic retry.
///
/// The trust gate runs **before** the fresh prepare/scan, so a machine without a usable helper
/// fails fast instead of spending seconds on serial work first. A portable, unsigned development
/// build (the panel itself carries no Authenticode signature) may elevate its unsigned sibling
/// helper with the documented development-mode exemption; a signed installation never can.
fn execute_via_helper(
    inventory: &ProductionInventory,
    token: &ValidatedActionToken,
) -> Result<ExecutionReceipt, PortError> {
    let target = token.target_context()?;
    let device = current_device(inventory, &target)?;

    let repair_action = repair_action(token.action())?;
    let helper_action = helper_action(token.action())?;

    let helper = match TrustedHelper::installed() {
        Ok(helper) => helper,
        Err(_privilege_error) if dji4g_windows_platform::is_dev_build() => {
            TrustedHelper::dev_sibling().map_err(map_privilege_error)?
        }
        Err(privilege_error) => return Err(map_privilege_error(privilege_error)),
    };

    let executor =
        WindowsRepairExecutor::new(WindowsNativeRepairBackend::with_epoch(token.epoch()));
    let plan = executor
        .prepare(repair_action.clone())
        .map_err(map_repair_error)?;
    if plan.epoch() != token.epoch() {
        return Err(PortError::new(
            ErrorCode::EvidenceExpired,
            "privilege:epoch_changed",
        ));
    }
    // Cross-check the freshly re-enumerated target against the exact identity the user
    // confirmed, not just the epoch. `current_device` above proves the device is present; this
    // proves the plan the helper will execute is about that same device and action.
    if dji4g_windows_platform::authoritative_identity_hash(&device) != plan.target_identity_hash() {
        return Err(PortError::new(
            ErrorCode::DeviceIdentityChanged,
            "privilege:target_identity_changed",
        ));
    }
    if plan.action() != &repair_action {
        return Err(PortError::new(
            ErrorCode::Unsupported,
            "privilege:action_mismatch",
        ));
    }

    let now = SystemTime::now();
    let request = build_helper_request(
        plan.epoch(),
        plan.target_identity_hash(),
        plan.before_state_hash().0,
        helper_action,
        now,
    )?;
    let response = launch_elevated_helper(&helper, request, now).map_err(map_privilege_error)?;
    map_helper_response(response)
}

/// The closed set of actions executed in process instead of through the elevated helper.
fn executes_in_process(action: &ActionKind) -> bool {
    matches!(action, ActionKind::ToggleHotspot { .. })
}

/// Execute one confirmed hotspot toggle in process, without elevation.
///
/// WinRT tethering control needs no administrator rights, so this action does not need the
/// privileged helper. The non-`Send` WinRT objects are created, prepared, executed and read back
/// on one dedicated worker thread and the receipt crosses by `join`; every other step mirrors
/// `execute_via_helper` exactly: re-find the exact target, map onto the typed repair action,
/// fresh native prepare, epoch re-check, then the one-shot execute whose backend revalidates the
/// before-state hash and produces the fresh readback behind the three-state result.
fn execute_hotspot_in_process(
    inventory: &ProductionInventory,
    token: &ValidatedActionToken,
) -> Result<ExecutionReceipt, PortError> {
    let target = token.target_context()?;
    let _device = current_device(inventory, &target)?;
    let repair_action = repair_action(token.action())?;
    let epoch = token.epoch();

    let worker = std::thread::Builder::new()
        .name("dji4g-hotspot-toggle".to_owned())
        .spawn(move || {
            let executor =
                WindowsRepairExecutor::new(WindowsNativeRepairBackend::with_epoch(epoch));
            let plan = executor.prepare(repair_action).map_err(map_repair_error)?;
            if plan.epoch() != epoch {
                return Err(PortError::new(
                    ErrorCode::EvidenceExpired,
                    "hotspot:epoch_changed",
                ));
            }
            let result = executor.execute(&plan);
            Ok(match result.outcome() {
                OperationOutcome::Applied { after_state_hash } => ExecutionReceipt {
                    outcome: ExecutionReceiptOutcome::Applied,
                    after_state_hash: Some(*after_state_hash),
                },
                OperationOutcome::Failed { code, .. } => ExecutionReceipt {
                    outcome: ExecutionReceiptOutcome::Failed { code: *code },
                    after_state_hash: None,
                },
                OperationOutcome::OutcomeUnknown { code } => ExecutionReceipt {
                    outcome: ExecutionReceiptOutcome::OutcomeUnknown { code: *code },
                    after_state_hash: None,
                },
            })
        })
        .map_err(|_| PortError::new(ErrorCode::Internal, "hotspot:worker_unavailable"))?;
    worker
        .join()
        .unwrap_or_else(|_| Err(PortError::new(ErrorCode::Internal, "hotspot:worker_failed")))
}

/// Map the closed application action onto the typed repair action used for the fresh local prepare.
fn repair_action(action: &ActionKind) -> Result<RepairAction, PortError> {
    Ok(match action {
        ActionKind::RenewDhcp => RepairAction::RefreshDhcp,
        ActionKind::ApplyDnsProfile { profile } => RepairAction::ApplyDnsProfile {
            profile: profile.clone(),
        },
        ActionKind::RestartAdapter => RepairAction::RestartAdapter,
        ActionKind::ReenumerateDevice => RepairAction::ReenumerateDevice,
        ActionKind::RestartModule => RepairAction::RestartModule,
        ActionKind::EditApn { cid, apn } => RepairAction::SetApn {
            cid: PdpContextId::try_from(*cid).map_err(|_| invalid_action_arguments())?,
            apn: Apn::try_from(apn.as_str()).map_err(|_| invalid_action_arguments())?,
        },
        ActionKind::SetVerifiedUsbNetworkProfile { profile } => RepairAction::SetUsbNetProfile {
            profile: match profile {
                UsbNetworkProfile::DjiNdis => VerifiedUsbNetProfile::DjiNdis,
                UsbNetworkProfile::Ecm => VerifiedUsbNetProfile::Ecm,
            },
        },
        ActionKind::ToggleHotspot { enabled } => RepairAction::ToggleHotspot { enabled: *enabled },
        ActionKind::Refresh => {
            return Err(PortError::new(
                ErrorCode::Unsupported,
                "app:refresh_not_action",
            ));
        }
    })
}

/// Map the closed application action onto the closed helper request variant.  No raw command,
/// path, or free-form string ever crosses this boundary.
fn helper_action(action: &ActionKind) -> Result<HelperActionV1, PortError> {
    Ok(match action {
        ActionKind::RenewDhcp => HelperActionV1::RenewDhcp,
        ActionKind::ApplyDnsProfile { profile } => HelperActionV1::ApplyDnsProfile {
            profile: match profile {
                DnsProfile::Automatic => DnsProfileV1::Automatic,
                DnsProfile::Static { servers } => DnsProfileV1::Static {
                    servers: BoundedDnsServers::try_from(servers.clone())
                        .map_err(|_| invalid_action_arguments())?,
                },
            },
        },
        ActionKind::RestartAdapter => HelperActionV1::RestartAdapter,
        ActionKind::ReenumerateDevice => HelperActionV1::ReenumerateDevice,
        ActionKind::RestartModule => HelperActionV1::RestartModule,
        ActionKind::EditApn { cid, apn } => HelperActionV1::EditApn {
            cid: PdpContextIdV1::new(*cid).map_err(|_| invalid_action_arguments())?,
            apn: ValidatedApn::try_from(apn.clone()).map_err(|_| invalid_action_arguments())?,
        },
        ActionKind::SetVerifiedUsbNetworkProfile { profile } => HelperActionV1::SetUsbNetProfile {
            profile: match profile {
                UsbNetworkProfile::DjiNdis => UsbNetProfileV1::DjiNdis,
                UsbNetworkProfile::Ecm => UsbNetProfileV1::Ecm,
            },
        },
        ActionKind::ToggleHotspot { enabled } => {
            HelperActionV1::ToggleHotspot { enabled: *enabled }
        }
        ActionKind::Refresh => {
            return Err(PortError::new(
                ErrorCode::Unsupported,
                "app:refresh_not_action",
            ));
        }
    })
}

fn invalid_action_arguments() -> PortError {
    PortError::new(
        ErrorCode::VerificationFailed,
        "privilege:invalid_action_arguments",
    )
}

/// Build one bounded, self-describing helper request carrying the freshly proven epoch, identity
/// hash, and before-state hash.  The nonce and request id are random per call so a request can never
/// be replayed, and the lifetime never exceeds the protocol maximum.
fn build_helper_request(
    epoch: DeviceEpoch,
    identity_hash: [u8; 32],
    before_state_hash: [u8; 32],
    action: HelperActionV1,
    now: SystemTime,
) -> Result<HelperRequestV1, PortError> {
    let nonce = OperationNonce::random()
        .map_err(|_| PortError::new(ErrorCode::Internal, "privilege:nonce_unavailable"))?;
    let request_id = RequestId::random()
        .map_err(|_| PortError::new(ErrorCode::Internal, "privilege:request_id_unavailable"))?;
    let issued_at = UnixMillis::from_system_time(now)
        .ok_or_else(|| PortError::new(ErrorCode::Internal, "privilege:clock_unavailable"))?;
    let expires_at = UnixMillis(issued_at.as_u64().saturating_add(OPERATION_LIFETIME_MS));
    Ok(HelperRequestV1 {
        version: ProtocolVersion::V1,
        request_id,
        nonce,
        issued_at,
        expires_at,
        target: TargetProofV1 {
            profile: SupportedProfileV1::DjiGen1,
            epoch: epoch.0,
            identity_hash: Hash32::from_bytes(identity_hash),
            before_state_hash: Hash32::from_bytes(before_state_hash),
        },
        action,
    })
}

/// Map the helper's three-state result back onto the application receipt.  A rejection means the
/// helper did not execute, so it is a definite failure rather than an unknown outcome.
fn map_helper_response(response: HelperResponseV1) -> Result<ExecutionReceipt, PortError> {
    match response.result {
        HelperResultV1::Completed(OperationResultV1::Applied {
            after_state_hash, ..
        }) => Ok(ExecutionReceipt {
            outcome: ExecutionReceiptOutcome::Applied,
            after_state_hash: Some(AfterStateHash(*after_state_hash.as_bytes())),
        }),
        HelperResultV1::Completed(OperationResultV1::Failed { code, .. }) => Ok(ExecutionReceipt {
            outcome: ExecutionReceiptOutcome::Failed {
                code: operation_error_code(code),
            },
            after_state_hash: None,
        }),
        HelperResultV1::Completed(OperationResultV1::OutcomeUnknown { code }) => {
            Ok(ExecutionReceipt {
                outcome: ExecutionReceiptOutcome::OutcomeUnknown {
                    code: operation_error_code(code),
                },
                after_state_hash: None,
            })
        }
        // A state-changing request never yields an inspection result; treat it as verification
        // failure so it can never be mistaken for success.
        HelperResultV1::Inspected { .. } => Err(PortError::new(
            ErrorCode::VerificationFailed,
            "privilege:unexpected_inspection",
        )),
        HelperResultV1::Rejected { code } => Err(reject_port_error(code)),
    }
}

fn operation_error_code(code: OperationCode) -> ErrorCode {
    match code {
        OperationCode::InvalidActionArguments => ErrorCode::VerificationFailed,
        OperationCode::UnsupportedDevice => ErrorCode::Unsupported,
        OperationCode::TargetNotFound => ErrorCode::DeviceRemoved,
        OperationCode::TargetAmbiguous => ErrorCode::CapabilityUnavailable,
        OperationCode::TargetIdentityChanged => ErrorCode::DeviceIdentityChanged,
        OperationCode::EpochChanged | OperationCode::BeforeStateChanged => {
            ErrorCode::EvidenceExpired
        }
        OperationCode::AtPortUnavailable => ErrorCode::CapabilityUnavailable,
        OperationCode::PermissionDenied => ErrorCode::PermissionDenied,
        OperationCode::OperationCancelled => ErrorCode::OperationCancelled,
        OperationCode::Timeout => ErrorCode::Timeout,
        OperationCode::PeerDisconnected => ErrorCode::DeviceRemoved,
        OperationCode::VerificationFailed => ErrorCode::VerificationFailed,
        OperationCode::RollbackFailed => ErrorCode::RollbackFailed,
        OperationCode::ProtocolRejected => ErrorCode::VerificationFailed,
        OperationCode::Internal => ErrorCode::Internal,
    }
}

fn reject_port_error(code: RequestRejectCode) -> PortError {
    let (category, stable) = match code {
        RequestRejectCode::UacCancelled => (
            ErrorCode::OperationCancelled,
            "privilege:operation_cancelled",
        ),
        RequestRejectCode::HelperUntrusted => {
            (ErrorCode::PermissionDenied, "privilege:helper_untrusted")
        }
        RequestRejectCode::Expired | RequestRejectCode::FutureIssuedAt => {
            (ErrorCode::EvidenceExpired, "helper:request_expired")
        }
        RequestRejectCode::EpochChanged | RequestRejectCode::BeforeStateChanged => {
            (ErrorCode::EvidenceExpired, "helper:before_state_changed")
        }
        RequestRejectCode::TargetIdentityChanged => (
            ErrorCode::DeviceIdentityChanged,
            "helper:target_identity_changed",
        ),
        RequestRejectCode::TargetNotFound => (ErrorCode::DeviceRemoved, "helper:target_not_found"),
        RequestRejectCode::TargetAmbiguous => {
            (ErrorCode::CapabilityUnavailable, "helper:target_ambiguous")
        }
        RequestRejectCode::UnsupportedDevice => {
            (ErrorCode::Unsupported, "helper:unsupported_device")
        }
        RequestRejectCode::PermissionDenied
        | RequestRejectCode::RemoteClient
        | RequestRejectCode::PeerPidMismatch
        | RequestRejectCode::PeerCreationChanged
        | RequestRejectCode::UserMismatch
        | RequestRejectCode::SessionMismatch
        | RequestRejectCode::IntegrityMismatch
        | RequestRejectCode::PeerImageMismatch => {
            (ErrorCode::PermissionDenied, "privilege:peer_rejected")
        }
        RequestRejectCode::Timeout => (ErrorCode::Timeout, "privilege:timeout"),
        RequestRejectCode::PeerDisconnected => {
            (ErrorCode::DeviceRemoved, "helper:peer_disconnected")
        }
        RequestRejectCode::Internal => (ErrorCode::Internal, "privilege:internal"),
        _ => (ErrorCode::VerificationFailed, "privilege:protocol_rejected"),
    };
    PortError::new(category, stable)
}

fn map_privilege_error(error: PrivilegeError) -> PortError {
    match error {
        PrivilegeError::OperationCancelled => PortError::new(
            ErrorCode::OperationCancelled,
            "privilege:operation_cancelled",
        ),
        PrivilegeError::HelperUntrusted => {
            PortError::new(ErrorCode::PermissionDenied, "privilege:helper_untrusted")
        }
        PrivilegeError::HelperUnsigned => {
            PortError::new(ErrorCode::PermissionDenied, "privilege:helper_unsigned")
        }
        PrivilegeError::HelperVerificationUnavailable => {
            PortError::new(ErrorCode::Internal, "privilege:helper_unverified")
        }
        PrivilegeError::Timeout => PortError::new(ErrorCode::Timeout, "privilege:timeout"),
        PrivilegeError::LaunchFailed => {
            PortError::new(ErrorCode::PermissionDenied, "privilege:launch_failed")
        }
        PrivilegeError::UnsupportedPlatform => PortError::new(
            ErrorCode::CapabilityUnavailable,
            "privilege:unsupported_platform",
        ),
        PrivilegeError::Protocol(_) => {
            PortError::new(ErrorCode::VerificationFailed, "privilege:protocol_rejected")
        }
        PrivilegeError::Peer(_) => {
            PortError::new(ErrorCode::PermissionDenied, "privilege:peer_rejected")
        }
        PrivilegeError::Internal => PortError::new(ErrorCode::Internal, "privilege:internal"),
    }
}

fn map_repair_error(error: RepairError) -> PortError {
    // `RepairError`'s Display is already a closed, lowercase stable code (e.g. `repair:no_change`).
    let stable_text = error.to_string();
    let category = error.code();
    let stable = StableCode::try_from_owned(stable_text).unwrap_or_else(|_| {
        StableCode::try_from_static("repair:internal").expect("valid static stable code")
    });
    PortError {
        code: FailureCode::new(category, stable),
        os_code: None,
    }
}

fn next_epoch(current: DeviceEpoch) -> DeviceEpoch {
    DeviceEpoch(current.0.saturating_add(1).max(1))
}

fn stable_identity(device: &DjiDevice) -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: device.container_id().unwrap_or_default().to_owned(),
        device_instance_id: device.root_instance_id().to_owned(),
        vid: dji4g_domain::DJI_GEN1.vid,
        pid: dji4g_domain::DJI_GEN1.pid,
    }
}

fn unique_adapter_id(device: &DjiDevice) -> Option<String> {
    let mut ids = device
        .net_candidates()
        .iter()
        .filter_map(|candidate| candidate.net_cfg_instance_id())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    (ids.len() == 1).then(|| ids.remove(0))
}

fn current_device(
    inventory: &ProductionInventory,
    target: &TargetContext,
) -> Result<DjiDevice, PortError> {
    let snapshot = inventory.native.scan_now().map_err(map_platform_error)?;
    let matches = snapshot
        .devices()
        .iter()
        .filter(|device| {
            device.root_instance_id() == target.identity().device_instance_id
                && device
                    .container_id()
                    .unwrap_or_default()
                    .eq_ignore_ascii_case(&target.identity().container_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [device] => Ok(device.clone()),
        [] => Err(PortError::new(
            ErrorCode::DeviceRemoved,
            "pnp:target_not_found",
        )),
        _ => Err(PortError::new(
            ErrorCode::CapabilityUnavailable,
            "pnp:ambiguous_device",
        )),
    }
}

fn adapter_dto(observation: AdapterObservation, target: &TargetContext) -> AdapterObservationDto {
    let dns_automatic =
        dji4g_windows_platform::repair::observe_dns_automatic(&observation.identity);
    let identity = observation.identity;
    let binding = AdapterBinding {
        target: target.identity().clone(),
        adapter_id: identity.guid_string(),
    };
    let addresses = identity
        .unicast_addresses()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    AdapterObservationDto {
        details: Some(dji4g_application::AdapterNetworkDetails {
            link_up: observation.oper_up,
            dhcp_v4: observation.dhcp_v4,
            dns_automatic,
        }),
        epoch: identity.epoch(),
        binding,
        state: if observation.usable_families.is_empty() {
            AdapterStateDto::NoUsableAddressOrRoute
        } else {
            AdapterStateDto::UsableAddressAndRoute
        },
        addresses,
        gateways: observation
            .gateways
            .iter()
            .map(ToString::to_string)
            .collect(),
        dns_servers: observation
            .dns_servers
            .iter()
            .map(ToString::to_string)
            .collect(),
        ipv4: observation.usable_families.contains(&AddressFamily::Ipv4),
        ipv6: observation.usable_families.contains(&AddressFamily::Ipv6),
        rx_bytes: observation.rx_bytes,
        tx_bytes: observation.tx_bytes,
    }
}

/// Resolve the retained GUID→LUID mapping for the adapter the reducer already bound.
///
/// A GUID the resolver has not produced in this process — or a poisoned/empty mapping — is an
/// honest `Unsupported` gap, never a substituted interface.
fn bound_luid(mapping: Option<&(String, u64)>, adapter_id: &str) -> Result<u64, PortError> {
    match mapping {
        Some((guid, luid)) if same_guid(guid, adapter_id) => Ok(*luid),
        _ => Err(PortError::new(
            ErrorCode::Unsupported,
            "net:metrics_unavailable",
        )),
    }
}

/// Field-for-field mapping from the platform's per-interface MIB-II sample.
fn adapter_metrics(metrics: InterfaceMetrics) -> AdapterMetrics {
    AdapterMetrics {
        rx_bytes: metrics.rx_bytes,
        tx_bytes: metrics.tx_bytes,
        in_errors: metrics.in_errors,
        out_errors: metrics.out_errors,
        in_discards: metrics.in_discards,
        out_discards: metrics.out_discards,
        link_rx_bits_per_second: metrics.link_rx_bits_per_second,
        link_tx_bits_per_second: metrics.link_tx_bits_per_second,
    }
}

fn map_platform_error(error: PlatformError) -> PortError {
    let category = if error.code.contains("permission") {
        ErrorCode::PermissionDenied
    } else if error.code.contains("not_found") || error.code.contains("removed") {
        ErrorCode::DeviceRemoved
    } else if error.code.starts_with("probe:") {
        ErrorCode::ProbeFailed
    } else {
        ErrorCode::CapabilityUnavailable
    };
    if error.code.starts_with("sms:") {
        crate::logging::append_event(&format!(
            "sms_inbox error={} os_code={:?}",
            error.code, error.os_code
        ));
    }
    let mut mapped = PortError::new(category, error.code);
    mapped.os_code = error.os_code;
    mapped
}

/// Compare two adapter GUID strings without depending on brace/case formatting.  Both sides
/// originate from `AdapterIdentity::guid_string`, so this is a defensive normalization rather than
/// a permissive match: an empty value never matches.
fn same_guid(left: &str, right: &str) -> bool {
    fn normalize(value: &str) -> String {
        value
            .trim()
            .trim_matches(|character| character == '{' || character == '}')
            .to_ascii_lowercase()
    }
    !left.trim().is_empty() && normalize(left) == normalize(right)
}

fn probe_failure(category: ErrorCode, stable: &'static str) -> FailureCode {
    PortError::new(category, stable).code
}

/// Translate a bound probe result into the application DTO.  Gateway, public, and DNS stages are
/// aggregated from the per-family bound evidence; protocol coverage records which required families
/// actually proved bound public reachability; and the global default route is carried only as
/// explanation (`explanation_only`), never as proof of module availability.
fn probe_dto(result: BoundProbeResult) -> ProbeObservationDto {
    let gateway = aggregate_stage(
        result.families.iter().flat_map(|family| {
            family
                .endpoints
                .iter()
                .map(|attempt| &attempt.route.outcome)
        }),
        "probe:route_unavailable",
    );
    let public = public_stage(&result);
    let dns = aggregate_stage(
        result.families.iter().map(|family| &family.dns.outcome),
        "probe:dns_failed",
    );
    let protocol_coverage = protocol_coverage(&result);
    let system_route = system_route(&result);
    let route_choices = result
        .global_routes
        .iter()
        .map(|comparison| {
            let evidence = match &comparison.route.outcome {
                ProbeStage::Succeeded(e) => Some(e),
                _ => None,
            };
            dji4g_application::NetworkRouteChoice {
                family: match comparison.family {
                    AddressFamily::Ipv4 => dji4g_domain::IpFamily::V4,
                    AddressFamily::Ipv6 => dji4g_domain::IpFamily::V6,
                },
                luid: evidence.and_then(|e| e.global_luid),
                owner: evidence.map(|e| match e.owner {
                    DefaultRouteOwner::TargetAdapter => DefaultRouteDto::TargetAdapter,
                    DefaultRouteOwner::VpnOrTun => DefaultRouteDto::VpnOrTun,
                    DefaultRouteOwner::Other => DefaultRouteDto::Other,
                }),
            }
        })
        .collect();
    ProbeObservationDto {
        route_choices,
        epoch: result.epoch,
        adapter_id: result.adapter_guid,
        gateway,
        public,
        dns,
        protocol_coverage,
        system_route,
    }
}

/// Collapse a set of bound probe stages into one DTO stage.  Any success proves the stage; otherwise
/// a concrete failure outranks an unavailable dependency, which outranks an unexecuted stage.
fn aggregate_stage<'a, T: 'a>(
    stages: impl Iterator<Item = &'a ProbeStage<T>>,
    default: &'static str,
) -> ProbeStageDto {
    let mut failed: Option<&'static str> = None;
    let mut unavailable: Option<&'static str> = None;
    let mut unexecuted: Option<&'static str> = None;
    let mut observed = false;
    for stage in stages {
        observed = true;
        match stage {
            ProbeStage::Succeeded(_) => return ProbeStageDto::Passed,
            ProbeStage::Failed { code, .. } => {
                if failed.is_none() {
                    failed = Some(*code);
                }
            }
            ProbeStage::Unavailable { code } => {
                if unavailable.is_none() {
                    unavailable = Some(*code);
                }
            }
            ProbeStage::Unexecuted { code } => {
                if unexecuted.is_none() {
                    unexecuted = Some(*code);
                }
            }
        }
    }
    if !observed {
        return ProbeStageDto::Unavailable {
            code: probe_failure(ErrorCode::CapabilityUnavailable, "probe:no_interface"),
        };
    }
    if let Some(code) = failed {
        ProbeStageDto::Failed {
            code: probe_failure(ErrorCode::ProbeFailed, code),
        }
    } else if let Some(code) = unavailable {
        ProbeStageDto::Unavailable {
            code: probe_failure(ErrorCode::CapabilityUnavailable, code),
        }
    } else {
        ProbeStageDto::Unexecuted {
            code: probe_failure(ErrorCode::ProbeFailed, unexecuted.unwrap_or(default)),
        }
    }
}

fn public_stage(result: &BoundProbeResult) -> ProbeStageDto {
    if result.chosen_family.is_some() {
        return ProbeStageDto::Passed;
    }
    // No endpoint completed route+connect+TLS+HTTP.  If a TCP connect still succeeded, the public
    // verification did not complete; otherwise propagate the connect-stage outcome.
    match aggregate_stage(
        result.families.iter().flat_map(|family| {
            family
                .endpoints
                .iter()
                .map(|attempt| &attempt.connect.outcome)
        }),
        "probe:connect_failed",
    ) {
        ProbeStageDto::Passed => ProbeStageDto::Failed {
            code: probe_failure(ErrorCode::ProbeFailed, "probe:public_verification_failed"),
        },
        other => other,
    }
}

fn protocol_coverage(result: &BoundProbeResult) -> Option<ProtocolCoverage> {
    let exposed = result.families.len();
    if exposed == 0 {
        return None;
    }
    let succeeded = result
        .families
        .iter()
        .filter(|family| family.endpoints.iter().any(endpoint_public_success))
        .count();
    match succeeded {
        0 => None,
        count if count == exposed => Some(ProtocolCoverage::AllRequiredFamilies),
        _ => Some(ProtocolCoverage::SingleFamilyOnly),
    }
}

fn endpoint_public_success(attempt: &EndpointAttempt) -> bool {
    matches!(attempt.route.outcome, ProbeStage::Succeeded(_))
        && matches!(attempt.connect.outcome, ProbeStage::Succeeded(_))
        && matches!(
            attempt.tls.outcome,
            ProbeStage::Succeeded(TlsEvidence { validated: true })
        )
        && matches!(attempt.http.outcome, ProbeStage::Succeeded(_))
}

fn system_route(result: &BoundProbeResult) -> Option<SystemRouteDto> {
    let comparison = result
        .global_routes
        .iter()
        .find(|comparison| {
            Some(comparison.family) == result.chosen_family
                && matches!(comparison.route.outcome, ProbeStage::Succeeded(_))
        })
        .or_else(|| {
            result
                .global_routes
                .iter()
                .find(|comparison| matches!(comparison.route.outcome, ProbeStage::Succeeded(_)))
        })?;
    let ProbeStage::Succeeded(evidence) = &comparison.route.outcome else {
        return None;
    };
    Some(SystemRouteDto {
        owner: match evidence.owner {
            DefaultRouteOwner::TargetAdapter => DefaultRouteDto::TargetAdapter,
            DefaultRouteOwner::VpnOrTun => DefaultRouteDto::VpnOrTun,
            DefaultRouteOwner::Other => DefaultRouteDto::Other,
        },
        // Explanation only: a global default route never counts as bound module evidence.
        explanation_only: comparison.explanation_only,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn pdp_display_parse_failure_does_not_fail_at_observation() {
        use dji4g_at_protocol::{AtCommand, AtFinalCode, AtResponse};
        let contexts = AtResponse {
            epoch: dji4g_domain::DeviceEpoch(7),
            command: AtCommand::PdpContexts,
            lines: vec![r#"+CGDCONT: 1,"NONIP","example""#.into()],
            final_code: AtFinalCode::Ok,
        };
        let activity = AtResponse {
            command: AtCommand::PdpActivation,
            lines: vec!["+CGACT: 1,1".into()],
            ..contexts.clone()
        };
        assert!(
            super::parse_pdp_for_display(&contexts, &activity)
                .unwrap()
                .is_none()
        );
        // Display fallback must not weaken the parser used for APN repair.
        assert!(dji4g_at_protocol::parse_pdp_contexts_with_activity(&contexts, &activity).is_err());
        let stale = AtResponse {
            epoch: dji4g_domain::DeviceEpoch(8),
            ..activity
        };
        assert!(super::parse_pdp_for_display(&contexts, &stale).is_err());
    }
    use super::{
        ProductionAt, ProductionComposition, ProductionInventory, SystemClock, adapter_metrics,
        aggregate_stage, bound_luid, build_helper_request, endpoint_public_success,
        executes_in_process, helper_action, map_privilege_error, operation_error_code,
        parse_temperature_probe, probe_dto, protocol_coverage, reject_port_error, repair_action,
        same_guid, sms_message, system_route,
    };
    use dji4g_application::{AdapterMetrics, AtPort, Clock, ProbeStageDto};
    use dji4g_at_protocol::{AtFinalCode, DecodedSms, SensorTemperature, VerifiedUsbNetProfile};
    use dji4g_domain::{
        ActionKind, DefaultRouteOwner, DeviceEpoch, DnsProfile, ErrorCode, FeatureStatus,
        NumberLookup, ProtocolCoverage, SmsEncoding, SmsStatus, SmsStorageId, UsbNetworkProfile,
    };
    use dji4g_ipc::{HelperActionV1, OperationCode, RequestRejectCode, validate_request};
    use dji4g_windows_platform::{
        AddressFamily, BoundProbeResult, BoundRouteEvidence, ConnectEvidence, EndpointAttempt,
        FamilyProbeResult, GlobalRouteComparison, GlobalRouteEvidence, HttpEvidence,
        InterfaceMetrics, PrivilegeError, ProbeStage, RepairAction, RouteObservation, SmsRecord,
        TimedStage, TlsEvidence, TrustedHelper,
    };
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};

    fn stage<T>(outcome: ProbeStage<T>) -> TimedStage<T> {
        TimedStage {
            outcome,
            started_at: SystemTime::UNIX_EPOCH,
            finished_at: SystemTime::UNIX_EPOCH,
            elapsed: Duration::ZERO,
        }
    }

    fn route_evidence() -> BoundRouteEvidence {
        BoundRouteEvidence {
            luid: 7,
            source: IpAddr::V4(Ipv4Addr::new(192, 168, 225, 2)),
            route: RouteObservation {
                family: AddressFamily::Ipv4,
                interface_index: 12,
                destination: IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                prefix_len: 0,
                next_hop: IpAddr::V4(Ipv4Addr::new(192, 168, 225, 1)),
                route_metric: 1,
                total_metric: 1,
            },
        }
    }

    fn endpoint(
        family: AddressFamily,
        route: ProbeStage<BoundRouteEvidence>,
        connect: ProbeStage<ConnectEvidence>,
        tls: ProbeStage<TlsEvidence>,
        http: ProbeStage<HttpEvidence>,
    ) -> EndpointAttempt {
        EndpointAttempt {
            endpoint_id: "cloudflare-v4",
            family,
            destination: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 443),
            route: stage(route),
            connect: stage(connect),
            tls: stage(tls),
            http: stage(http),
            started_at: SystemTime::UNIX_EPOCH,
            finished_at: SystemTime::UNIX_EPOCH,
            elapsed: Duration::ZERO,
        }
    }

    fn full_success_endpoint(family: AddressFamily) -> EndpointAttempt {
        endpoint(
            family,
            ProbeStage::Succeeded(route_evidence()),
            ProbeStage::Succeeded(ConnectEvidence {
                actual_source: IpAddr::V4(Ipv4Addr::new(192, 168, 225, 2)),
            }),
            ProbeStage::Succeeded(TlsEvidence { validated: true }),
            ProbeStage::Succeeded(HttpEvidence {
                status: 200,
                response_bytes: 16,
            }),
        )
    }

    fn global_route(family: AddressFamily, owner: DefaultRouteOwner) -> GlobalRouteComparison {
        GlobalRouteComparison {
            family,
            target_luid: 7,
            route: stage(ProbeStage::Succeeded(GlobalRouteEvidence {
                global_luid: Some(7),
                interface_index: Some(12),
                owner,
            })),
            explanation_only: true,
        }
    }

    #[test]
    fn composition_uses_production_controller_and_ports() {
        let composition = ProductionComposition::new(SystemTime::UNIX_EPOCH);
        assert!(composition.has_all_real_ports());
        let clock = SystemClock::default();
        assert!(clock.system_now() > SystemTime::UNIX_EPOCH);
    }

    #[test]
    fn at_port_invalidate_is_a_stateless_noop() {
        // The port retains only the shared inventory and epoch-keyed selection memory; invalidate
        // must be callable for any epoch without state or panic because observe opens a fresh
        // actor every time.
        let at = ProductionAt {
            inventory: Arc::new(ProductionInventory::default()),
            selection: Mutex::default(),
            feature_probe: Arc::new(Mutex::new(
                crate::feature_probe::FeatureProbeState::default(),
            )),
        };
        at.invalidate(DeviceEpoch(1));
        at.invalidate(DeviceEpoch(2));
    }

    #[test]
    fn same_guid_normalizes_case_and_braces_but_rejects_empty() {
        let lower = "{aaaa1111-2222-3333-4444-555566667777}";
        let upper = "AAAA1111-2222-3333-4444-555566667777";
        assert!(same_guid(lower, upper));
        assert!(same_guid(lower, lower));
        assert!(!same_guid(lower, "{aaaa1111-2222-3333-4444-555566667778}"));
        assert!(!same_guid("", ""));
        assert!(!same_guid("", upper));
    }

    #[test]
    fn aggregate_stage_prefers_success_then_failure_then_unavailable_then_unexecuted() {
        let succeeded = [ProbeStage::Succeeded(())];
        assert_eq!(
            aggregate_stage(succeeded.iter(), "probe:x"),
            ProbeStageDto::Passed
        );

        let mixed: [ProbeStage<()>; 2] = [
            ProbeStage::Unexecuted {
                code: "probe:total_timeout",
            },
            ProbeStage::Failed {
                code: "probe:dns_failed",
                os_code: None,
            },
        ];
        assert!(matches!(
            aggregate_stage(mixed.iter(), "probe:x"),
            ProbeStageDto::Failed { .. }
        ));

        let unavailable: [ProbeStage<()>; 1] = [ProbeStage::Unavailable {
            code: "net:no_usable_address",
        }];
        assert!(matches!(
            aggregate_stage(unavailable.iter(), "probe:x"),
            ProbeStageDto::Unavailable { .. }
        ));

        let unexecuted: [ProbeStage<()>; 1] = [ProbeStage::Unexecuted {
            code: "probe:total_timeout",
        }];
        assert!(matches!(
            aggregate_stage(unexecuted.iter(), "probe:x"),
            ProbeStageDto::Unexecuted { .. }
        ));

        let empty: [ProbeStage<()>; 0] = [];
        assert!(matches!(
            aggregate_stage(empty.iter(), "probe:x"),
            ProbeStageDto::Unavailable { .. }
        ));
    }

    #[test]
    fn probe_dto_maps_single_family_success_and_explanation_only_route() {
        let ipv4 = FamilyProbeResult {
            family: AddressFamily::Ipv4,
            interface_index: 12,
            selected_source: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 225, 2))),
            dns: stage(ProbeStage::Succeeded(vec![IpAddr::V4(Ipv4Addr::new(
                1, 1, 1, 1,
            ))])),
            endpoints: vec![full_success_endpoint(AddressFamily::Ipv4)],
        };
        let ipv6 = FamilyProbeResult {
            family: AddressFamily::Ipv6,
            interface_index: 13,
            selected_source: Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            dns: stage(ProbeStage::Succeeded(vec![IpAddr::V6(Ipv6Addr::LOCALHOST)])),
            endpoints: vec![endpoint(
                AddressFamily::Ipv6,
                ProbeStage::Unavailable {
                    code: "net:no_usable_address",
                },
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
            )],
        };
        let result = BoundProbeResult {
            epoch: DeviceEpoch(3),
            adapter_guid: "{aaaa1111-2222-3333-4444-555566667777}".to_owned(),
            chosen_family: Some(AddressFamily::Ipv4),
            families: vec![ipv4, ipv6],
            global_routes: vec![
                global_route(AddressFamily::Ipv4, DefaultRouteOwner::TargetAdapter),
                global_route(AddressFamily::Ipv6, DefaultRouteOwner::VpnOrTun),
            ],
            started_at: SystemTime::UNIX_EPOCH,
            finished_at: SystemTime::UNIX_EPOCH,
            elapsed: Duration::ZERO,
        };

        let dto = probe_dto(result);
        assert_eq!(dto.epoch, DeviceEpoch(3));
        assert_eq!(dto.gateway, ProbeStageDto::Passed);
        assert_eq!(dto.public, ProbeStageDto::Passed);
        assert_eq!(dto.dns, ProbeStageDto::Passed);
        // Only IPv4 proved bound public reachability, so coverage is single-family (Limited),
        // never silently AllRequiredFamilies.
        assert_eq!(
            dto.protocol_coverage,
            Some(ProtocolCoverage::SingleFamilyOnly)
        );
        assert_eq!(dto.route_choices.len(), 2);
        assert_eq!(dto.route_choices[0].family, dji4g_domain::IpFamily::V4);
        assert_eq!(
            dto.route_choices[0].owner,
            Some(dji4g_application::DefaultRouteDto::TargetAdapter)
        );
        assert_eq!(dto.route_choices[1].family, dji4g_domain::IpFamily::V6);
        assert_eq!(
            dto.route_choices[1].owner,
            Some(dji4g_application::DefaultRouteDto::VpnOrTun)
        );
        // The chosen family's route is reported and is explanation-only.
        let route = dto.system_route.expect("route present");
        assert!(route.explanation_only);
        assert!(matches!(
            route.owner,
            dji4g_application::DefaultRouteDto::TargetAdapter
        ));
    }

    #[test]
    fn protocol_coverage_requires_every_exposed_family_to_succeed() {
        let both = BoundProbeResult {
            epoch: DeviceEpoch(1),
            adapter_guid: "{g}".to_owned(),
            chosen_family: Some(AddressFamily::Ipv4),
            families: vec![
                FamilyProbeResult {
                    family: AddressFamily::Ipv4,
                    interface_index: 12,
                    selected_source: None,
                    dns: stage(ProbeStage::Unexecuted {
                        code: "probe:total_timeout",
                    }),
                    endpoints: vec![full_success_endpoint(AddressFamily::Ipv4)],
                },
                FamilyProbeResult {
                    family: AddressFamily::Ipv6,
                    interface_index: 13,
                    selected_source: None,
                    dns: stage(ProbeStage::Unexecuted {
                        code: "probe:total_timeout",
                    }),
                    endpoints: vec![full_success_endpoint(AddressFamily::Ipv6)],
                },
            ],
            global_routes: vec![],
            started_at: SystemTime::UNIX_EPOCH,
            finished_at: SystemTime::UNIX_EPOCH,
            elapsed: Duration::ZERO,
        };
        assert_eq!(
            protocol_coverage(&both),
            Some(ProtocolCoverage::AllRequiredFamilies)
        );
        assert!(endpoint_public_success(&both.families[0].endpoints[0]));
        assert_eq!(system_route(&both), None);
    }

    #[test]
    fn repair_and_helper_action_mappings_are_closed_and_reject_refresh() {
        assert!(matches!(
            repair_action(&ActionKind::RenewDhcp),
            Ok(RepairAction::RefreshDhcp)
        ));
        assert!(matches!(
            repair_action(&ActionKind::ToggleHotspot { enabled: true }),
            Ok(RepairAction::ToggleHotspot { enabled: true })
        ));
        assert!(matches!(
            repair_action(&ActionKind::SetVerifiedUsbNetworkProfile {
                profile: UsbNetworkProfile::Ecm
            }),
            Ok(RepairAction::SetUsbNetProfile {
                profile: VerifiedUsbNetProfile::Ecm
            })
        ));
        assert!(matches!(
            repair_action(&ActionKind::ApplyDnsProfile {
                profile: DnsProfile::Automatic
            }),
            Ok(RepairAction::ApplyDnsProfile { .. })
        ));
        assert!(repair_action(&ActionKind::Refresh).is_err());

        assert!(matches!(
            helper_action(&ActionKind::RestartAdapter),
            Ok(HelperActionV1::RestartAdapter)
        ));
        assert!(matches!(
            helper_action(&ActionKind::ToggleHotspot { enabled: false }),
            Ok(HelperActionV1::ToggleHotspot { enabled: false })
        ));
        assert!(helper_action(&ActionKind::Refresh).is_err());
        // An out-of-range PDP id or empty APN must not produce a helper request.
        assert!(
            helper_action(&ActionKind::EditApn {
                cid: 0,
                apn: "cmnet".to_owned()
            })
            .is_err()
        );
    }

    #[test]
    fn build_helper_request_passes_protocol_validation() {
        let now = SystemTime::now();
        let request = build_helper_request(
            DeviceEpoch(4),
            [9; 32],
            [8; 32],
            HelperActionV1::InspectTarget,
            now,
        )
        .expect("request builds");
        assert_eq!(request.target.epoch, 4);
        assert!(!request.nonce.is_zero());
        assert!(!request.request_id.is_zero());
        assert!(validate_request(&request, &request.nonce, now).is_ok());
    }

    #[test]
    fn privilege_and_helper_errors_map_to_stable_categories() {
        let untrusted = map_privilege_error(PrivilegeError::HelperUntrusted);
        assert_eq!(untrusted.code.category, ErrorCode::PermissionDenied);
        assert_eq!(
            untrusted.code.stable().as_str(),
            "privilege:helper_untrusted"
        );

        // The real Authenticode verdict has its own diagnosable stable codes.
        let unsigned = map_privilege_error(PrivilegeError::HelperUnsigned);
        assert_eq!(unsigned.code.category, ErrorCode::PermissionDenied);
        assert_eq!(unsigned.code.stable().as_str(), "privilege:helper_unsigned");
        let unverified = map_privilege_error(PrivilegeError::HelperVerificationUnavailable);
        assert_eq!(unverified.code.category, ErrorCode::Internal);
        assert_eq!(
            unverified.code.stable().as_str(),
            "privilege:helper_unverified"
        );

        let cancelled = map_privilege_error(PrivilegeError::OperationCancelled);
        assert_eq!(cancelled.code.category, ErrorCode::OperationCancelled);

        assert_eq!(
            operation_error_code(OperationCode::BeforeStateChanged),
            ErrorCode::EvidenceExpired
        );
        assert_eq!(
            operation_error_code(OperationCode::OperationCancelled),
            ErrorCode::OperationCancelled
        );
        assert_eq!(
            reject_port_error(RequestRejectCode::UacCancelled)
                .code
                .category,
            ErrorCode::OperationCancelled
        );
        assert_eq!(
            reject_port_error(RequestRejectCode::TargetIdentityChanged)
                .code
                .category,
            ErrorCode::DeviceIdentityChanged
        );
    }

    #[test]
    fn trusted_helper_fails_closed_outside_a_signed_install() {
        // A development/test checkout has no installed, signature-verified sibling helper, so the
        // privileged boundary must refuse rather than fall back to any in-process mutation.
        assert!(TrustedHelper::installed().is_err());
    }

    #[test]
    fn installed_reports_the_unsigned_dev_helper_with_a_diagnosable_code() {
        // The test harness runs from a deps directory with no sibling helper image, so the real
        // WinVerifyTrust check cannot run against any image and the boundary must fail closed
        // with `privilege:helper_unverified`.  When the unsigned dev helper exists next to the
        // panel (dev layout, unsigned MSIX), the same boundary reports `privilege:helper_unsigned`
        // (pinned by the windows-platform wintrust tests).  Either way the old generic
        // `privilege:helper_untrusted` reason is gone.
        let error = TrustedHelper::installed().expect_err("unsigned dev helper must fail closed");
        assert!(
            matches!(
                error.to_string().as_str(),
                "privilege:helper_unsigned" | "privilege:helper_unverified"
            ),
            "expected a diagnosable signature reason, got {error}"
        );
    }

    #[test]
    fn only_the_hotspot_toggle_executes_in_process() {
        // WinRT tethering needs no elevation; every other action must keep the authenticated
        // helper boundary and its fail-closed behaviour.
        assert!(executes_in_process(&ActionKind::ToggleHotspot {
            enabled: true
        }));
        assert!(executes_in_process(&ActionKind::ToggleHotspot {
            enabled: false
        }));
        assert!(!executes_in_process(&ActionKind::RenewDhcp));
        assert!(!executes_in_process(&ActionKind::ApplyDnsProfile {
            profile: dji4g_domain::DnsProfile::Automatic
        }));
        assert!(!executes_in_process(&ActionKind::RestartAdapter));
        assert!(!executes_in_process(&ActionKind::ReenumerateDevice));
        assert!(!executes_in_process(&ActionKind::RestartModule));
        assert!(!executes_in_process(&ActionKind::EditApn {
            cid: 1,
            apn: "internet.example".to_owned()
        }));
        assert!(!executes_in_process(
            &ActionKind::SetVerifiedUsbNetworkProfile {
                profile: dji4g_domain::UsbNetworkProfile::DjiNdis
            }
        ));
    }

    fn probe_response(lines: &[&str]) -> dji4g_at_protocol::AtResponse {
        dji4g_at_protocol::AtResponse {
            epoch: DeviceEpoch(1),
            command: dji4g_at_protocol::AtCommand::SubscriberNumber,
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            final_code: dji4g_at_protocol::AtFinalCode::Ok,
        }
    }

    #[test]
    fn optional_probe_skips_execution_while_the_sim_is_not_ready() {
        let mut executed = false;
        let (status, payload) = super::feature_probe(
            false,
            || {
                executed = true;
                Ok(probe_response(&[]))
            },
            super::parse_cnum_probe,
        );
        assert!(!executed, "a not-ready SIM must never touch the session");
        assert_eq!(status, FeatureStatus::TemporarilyUnavailable);
        assert!(payload.is_none());
    }

    #[test]
    fn cnum_probe_parses_reported_numbers_and_empty_ok() {
        let reported = probe_response(&["+CNUM: ,\"+8613800138000\",145"]);
        let (status, payload) =
            super::feature_probe(true, || Ok(reported), super::parse_cnum_probe);
        assert_eq!(status, FeatureStatus::Supported);
        let Some(NumberLookup::Reported(numbers)) = payload else {
            panic!("expected a reported number");
        };
        assert_eq!(numbers.len(), 1);
        assert_eq!(numbers[0].masked(), "****8000");
        assert_eq!(numbers[0].toa, 145);

        // CNUM empty OK is a valid 「no record」 result, never a failure (research §4.1).
        let empty = probe_response(&[]);
        let (status, payload) = super::feature_probe(true, || Ok(empty), super::parse_cnum_probe);
        assert_eq!(status, FeatureStatus::Supported);
        assert_eq!(payload, Some(NumberLookup::Empty));
    }

    #[test]
    fn cnum_probe_classifies_unrecognized_content_as_format_mismatch() {
        let malformed = probe_response(&["+CNUM: not,a,valid,record,shape,here,now"]);
        let (status, payload) =
            super::feature_probe(true, || Ok(malformed), super::parse_cnum_probe);
        assert_eq!(status, FeatureStatus::FormatMismatch);
        assert!(payload.is_none());
    }

    #[test]
    fn iccid_probe_parses_the_vendor_line_and_masks_it_at_the_domain_boundary() {
        let response = probe_response(&["+QCCID: \"89860123456789012345\""]);
        let (status, payload) =
            super::feature_probe(true, || Ok(response), super::parse_iccid_probe);
        assert_eq!(status, FeatureStatus::Supported);
        let identity =
            crate::feature_probe::sim_identity_for(payload.as_deref().expect("parsed iccid"));
        assert_eq!(identity.iccid_masked, "8986…2345");
        assert_eq!(
            identity.fingerprint,
            crate::feature_probe::iccid_fingerprint("89860123456789012345")
        );
    }

    #[test]
    fn iccid_probe_distinguishes_empty_ok_from_unrecognized_lines() {
        // QCCID answers OK with no data line: clean empty response.
        let (status, payload) =
            super::feature_probe(true, || Ok(probe_response(&[])), super::parse_iccid_probe);
        assert_eq!(status, FeatureStatus::Empty);
        assert!(payload.is_none());
        // QCCID answers OK but the data line is not the recognised vendor layout.
        let (status, payload) = super::feature_probe(
            true,
            || Ok(probe_response(&["+QCCID: nope"])),
            super::parse_iccid_probe,
        );
        assert_eq!(status, FeatureStatus::FormatMismatch);
        assert!(payload.is_none());
    }

    #[test]
    fn serving_cell_probe_keeps_the_raw_line_for_ui_detail() {
        let response = probe_response(&[
            "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,1A2B3C4,123,1650,3,5,5,0ABC,-95,-10,-65,15,20",
        ]);
        let (status, payload) =
            super::feature_probe(true, || Ok(response), super::parse_serving_cell_probe);
        assert_eq!(status, FeatureStatus::Supported);
        let Some((cell, raw)) = payload else {
            panic!("expected a parsed serving cell");
        };
        assert_eq!(cell.state.as_deref(), Some("NOCONN"));
        assert_eq!(cell.cell_id, Some(0x1A2B3C4));
        assert_eq!(cell.mnc.as_deref(), Some("01"));
        assert!(
            raw.starts_with("+QENG: \"servingcell\""),
            "the raw line must be retained verbatim for hover detail"
        );
    }

    #[test]
    fn serving_cell_probe_rejects_unrecognized_content_without_failing() {
        let (status, payload) = super::feature_probe(
            true,
            || Ok(probe_response(&["+QENG: \"servingcell\",\"WEIRD\"])"])),
            super::parse_serving_cell_probe,
        );
        assert_eq!(status, FeatureStatus::FormatMismatch);
        assert!(payload.is_none());
        let (status, payload) = super::feature_probe(
            true,
            || Ok(probe_response(&[])),
            super::parse_serving_cell_probe,
        );
        assert_eq!(status, FeatureStatus::Empty);
        assert!(payload.is_none());
    }

    #[test]
    fn cme_error_mapping_requires_interpretable_unsupported_evidence() {
        use dji4g_windows_platform::ActorError;
        // 3GPP code 4 = operation not supported: the one confirmed-unsupported classification.
        let unsupported = ActorError::FinalCode(AtFinalCode::CmeError("4".to_owned()));
        assert_eq!(
            super::map_feature_failure(&unsupported),
            FeatureStatus::UnsupportedConfirmed
        );
        assert!(super::cme_code_is_unsupported("4"));
        assert!(super::cme_code_is_unsupported(" 4 "));
        // Every other code and a plain ERROR stay ambiguous (research §8.1: never guess
        // "not supported" from any ERROR).
        for detail in ["14", "0", "100", "SIM failure", ""] {
            let error = ActorError::FinalCode(AtFinalCode::CmeError(detail.to_owned()));
            assert_eq!(
                super::map_feature_failure(&error),
                FeatureStatus::TemporarilyUnavailable,
                "cme detail {detail:?} must stay temporarily unavailable"
            );
        }
        let plain = ActorError::FinalCode(AtFinalCode::Error);
        assert_eq!(
            super::map_feature_failure(&plain),
            FeatureStatus::TemporarilyUnavailable
        );
    }

    #[test]
    fn optional_probe_transport_and_format_failures_map_to_distinct_categories() {
        use dji4g_at_protocol::{ProtocolError, ProtocolErrorKind};
        use dji4g_windows_platform::ActorError;
        let timeout = ActorError::Protocol(ProtocolError {
            code: dji4g_domain::ErrorCode::Timeout,
            kind: ProtocolErrorKind::Timeout,
        });
        assert_eq!(
            super::map_feature_failure(&timeout),
            FeatureStatus::TransportFailure
        );
        let removed = ActorError::Protocol(ProtocolError {
            code: dji4g_domain::ErrorCode::DeviceRemoved,
            kind: ProtocolErrorKind::DeviceRemoved,
        });
        assert_eq!(
            super::map_feature_failure(&removed),
            FeatureStatus::TransportFailure
        );
        let noise = ActorError::Protocol(ProtocolError {
            code: dji4g_domain::ErrorCode::VerificationFailed,
            kind: ProtocolErrorKind::UnexpectedData,
        });
        assert_eq!(
            super::map_feature_failure(&noise),
            FeatureStatus::FormatMismatch
        );
        for error in [
            ActorError::Closed,
            ActorError::Io(std::io::ErrorKind::ConnectionReset),
        ] {
            assert_eq!(
                super::map_feature_failure(&error),
                FeatureStatus::TransportFailure
            );
        }
        assert_eq!(
            super::map_feature_failure(&ActorError::QueueFull),
            FeatureStatus::TemporarilyUnavailable
        );
    }

    #[test]
    fn temperature_probe_keeps_the_first_reported_sensor_value() {
        // Sensor names stay firmware-defined: the first reported value becomes the module
        // temperature (research §7.5), while every reading travels with it for the rows' evidence.
        let response = probe_response(&["+QTEMP: \"mdm-case\",42", "+QTEMP: \"pa-therm\",45"]);
        let (status, probe) = super::feature_probe(true, || Ok(response), parse_temperature_probe);
        assert_eq!(status, FeatureStatus::Supported);
        let probe = probe.expect("a reading");
        assert_eq!(probe.celsius, 42);
        assert_eq!(
            probe.sensors,
            [
                SensorTemperature {
                    name: Some("mdm-case".to_owned()),
                    celsius: 42,
                },
                SensorTemperature {
                    name: Some("pa-therm".to_owned()),
                    celsius: 45,
                },
            ]
        );
        assert_eq!(probe.raw.as_deref(), Some("+QTEMP: \"mdm-case\",42"));
    }

    #[test]
    fn temperature_probe_reads_the_unnamed_positional_layout() {
        // The DJI Gen-1 firmware reports its channels positionally (`+QTEMP: 57,51,51`): the first
        // value is the shown module temperature and no channel is given an invented name.
        let response = probe_response(&["+QTEMP: 57,51,51"]);
        let (status, probe) = super::feature_probe(true, || Ok(response), parse_temperature_probe);
        assert_eq!(status, FeatureStatus::Supported);
        let probe = probe.expect("a reading");
        assert_eq!(probe.celsius, 57);
        assert_eq!(
            probe.sensors,
            [
                SensorTemperature {
                    name: None,
                    celsius: 57,
                },
                SensorTemperature {
                    name: None,
                    celsius: 51,
                },
                SensorTemperature {
                    name: None,
                    celsius: 51,
                },
            ]
        );
        assert_eq!(probe.raw.as_deref(), Some("+QTEMP: 57,51,51"));
    }

    #[test]
    fn temperature_probe_survives_a_line_the_build_does_not_model() {
        // Regression for the reported symptom: the overview showed 「未读取到 / 格式不匹配」 while
        // the device-tools query answered fine, because one unmodelled line aborted the whole
        // optional probe.  An unreadable line is now carried along (and shown) while the readable
        // answer still lands.
        let response = probe_response(&[
            "+QTEMP: 57,51,51",
            "+QTEMP: \"modem\"",
            "some vendor notice",
        ]);
        let (status, probe) = super::feature_probe(true, || Ok(response), parse_temperature_probe);
        assert_eq!(status, FeatureStatus::Supported);
        assert_eq!(probe.expect("a reading").celsius, 57);
    }

    #[test]
    fn temperature_probe_distinguishes_empty_from_unrecognized_content() {
        // A genuinely empty OK is a clean no-data result...
        let (status, probe) =
            super::feature_probe(true, || Ok(probe_response(&[])), parse_temperature_probe);
        assert_eq!(status, FeatureStatus::Empty);
        assert_eq!(probe, None);
        // ...while content that carries no readable QTEMP line is a format mismatch, never
        // conflated with "no data".
        let (status, probe) = super::feature_probe(
            true,
            || Ok(probe_response(&["+QTEMP: malformed"])),
            parse_temperature_probe,
        );
        assert_eq!(status, FeatureStatus::FormatMismatch);
        assert_eq!(probe, None);
    }

    #[test]
    fn temperature_raw_line_is_bounded_and_kept_for_display() {
        // The raw line is firmware chatter shown on hover: it is kept bounded, and a line that has
        // to be cut says so instead of silently losing its tail.
        let long = format!("+QTEMP: {}", "5".repeat(400));
        let response = probe_response(&[long.as_str()]);
        let raw = super::temperature_raw_line(&response).expect("a raw QTEMP line");
        assert_eq!(raw.chars().count(), super::TEMPERATURE_RAW_MAX_CHARS + 1);
        assert!(raw.ends_with('…'));
        // A response without a QTEMP line has no raw detail to show.
        assert_eq!(super::temperature_raw_line(&probe_response(&["OK"])), None);
    }

    #[test]
    fn interface_metrics_map_field_for_field() {
        let metrics = InterfaceMetrics {
            rx_bytes: 11,
            tx_bytes: 22,
            in_errors: 33,
            out_errors: 44,
            in_discards: 55,
            out_discards: 66,
            link_rx_bits_per_second: 1_000_000_000,
            link_tx_bits_per_second: 2_000_000_000,
        };
        assert_eq!(
            adapter_metrics(metrics),
            AdapterMetrics {
                rx_bytes: 11,
                tx_bytes: 22,
                in_errors: 33,
                out_errors: 44,
                in_discards: 55,
                out_discards: 66,
                link_rx_bits_per_second: 1_000_000_000,
                link_tx_bits_per_second: 2_000_000_000,
            }
        );
    }

    #[test]
    fn metrics_lookup_only_accepts_the_resolved_adapter_guid() {
        let mapping = Some(("{aaaa1111-2222-3333-4444-555566667777}".to_owned(), 7));
        assert_eq!(
            bound_luid(mapping.as_ref(), "AAAA1111-2222-3333-4444-555566667777"),
            Ok(7)
        );
        // Another adapter, a malformed GUID, or no retained mapping at all is an honest
        // Unsupported gap — never a substituted interface.
        for candidate in ["{bbbb1111-2222-3333-4444-555566667777}", "", "not-a-guid"] {
            let error = bound_luid(mapping.as_ref(), candidate)
                .expect_err("an unbound adapter must not resolve to a LUID");
            assert_eq!(error.code.category, ErrorCode::Unsupported);
            assert_eq!(error.code.stable().as_str(), "net:metrics_unavailable");
        }
        assert!(bound_luid(None, "{aaaa1111-2222-3333-4444-555566667777}").is_err());
    }

    fn decoded_sms(sender: &str, body: &str) -> DecodedSms {
        DecodedSms {
            sender: sender.to_owned(),
            timestamp: Some("2026-09-10 12:00:00".to_owned()),
            body: body.to_owned(),
            encoding: SmsEncoding::Gsm7,
            multipart: None,
            read: None,
        }
    }

    #[test]
    fn sms_record_maps_storage_epoch_read_state_and_timestamp() {
        let record = SmsRecord {
            index: 7,
            storage: "SM".to_owned(),
            stat: 1,
            decoded: decoded_sms("+8613800138000", "验证码 123456"),
        };
        let message = sms_message(record, DeviceEpoch(3));
        assert_eq!(message.index, 7);
        assert_eq!(message.storage, SmsStorageId("SM".to_owned()));
        assert_eq!(message.device_epoch, 3);
        assert_eq!(message.sim_epoch, 0, "exact SIM epoch is stamped at ingest");
        assert_eq!(message.sender(), "+8613800138000");
        assert_eq!(message.body(), "验证码 123456");
        assert_eq!(message.encoding, SmsEncoding::Gsm7);
        assert_eq!(message.status, SmsStatus::Received);
        assert_eq!(
            message.read,
            Some(true),
            "TS 27.005 stat 1 is a read stored message"
        );
        assert_eq!(
            message.service_centre_timestamp.as_deref(),
            Some("2026-09-10 12:00:00")
        );
    }

    #[test]
    fn sms_record_maps_unread_state_and_preserves_multipart_metadata() {
        let mut decoded = decoded_sms("10086", "first fragment");
        decoded.multipart = Some(dji4g_domain::SmsMultipartInfo {
            reference: dji4g_domain::SmsConcatReference::EightBit(9),
            total: 2,
            sequence: 1,
        });
        let unread = SmsRecord {
            index: 1,
            storage: "ME".to_owned(),
            stat: 0,
            decoded,
        };
        let message = sms_message(unread, DeviceEpoch(1));
        assert_eq!(message.read, Some(false));
        assert_eq!(
            message.multipart,
            Some(dji4g_domain::SmsMultipartInfo {
                reference: dji4g_domain::SmsConcatReference::EightBit(9),
                total: 2,
                sequence: 1,
            })
        );
    }
}
