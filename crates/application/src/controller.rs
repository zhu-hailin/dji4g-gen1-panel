use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    time::{Duration, SystemTime},
};

use dji4g_domain::{
    ActionKind, ActionSafetyError, DeviceEpoch, ErrorCode, FeatureStatus, OperationOutcome,
    RollbackOutcome, SmsMessage,
};

use crate::confirmation::{
    ControlledRepairRequest, StoredActionPlan, ValidatedActionToken, before_state_hash, build_plan,
    mint_token, next_plan_id,
};
use crate::ports::Clock;
use crate::{
    ActionExecutor, ActionKindTag, ActionPlanId, ActionRequest, BackendEvent, ConfirmError,
    ConfirmResult, ConfirmationInvalidationReason, ControllerSnapshot, EpochInvalidationReason,
    FailureCode, FakeActionExecutor, FakeClock, FeatureKey, LanguageCode, LogLevel, OperationPhase,
    OperationState, OperationUiSnapshot, PortError, PreparedActionState, ReducerState, StableCode,
};

pub const COMMAND_QUEUE_CAPACITY: usize = 32;
pub const PLAN_LIFETIME: Duration = Duration::from_secs(30);

/// A frozen expert command awaiting its single confirmation.
///
/// Confirmation carries only the id, so the text cannot be edited between the confirmation dialog
/// and the write. The plan expires, is consumed at most once, and dies with its device context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpertToolPlan {
    pub id: u64,
    context: crate::ToolContext,
    line: dji4g_at_protocol::ValidatedToolLine,
    expires_at: SystemTime,
    consumed: bool,
}

/// Why a frozen expert command could not be confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpertToolRefusal {
    UnknownPlan,
    AlreadyConfirmed,
    AlreadyFrozen,
    Expired,
    ContextChanged,
}

impl ExpertToolRefusal {
    const fn code(self) -> &'static str {
        match self {
            Self::UnknownPlan => "tool:unknown_plan",
            Self::AlreadyConfirmed => "tool:already_confirmed",
            Self::AlreadyFrozen => "tool:plan_pending",
            Self::Expired => "tool:plan_expired",
            Self::ContextChanged => "tool:context_changed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrepareError {
    Busy,
    UnsupportedAction,
    MissingTarget,
    Prerequisite(FailureCode),
    Safety(ActionSafetyError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiSendError {
    QueueFull,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandReceipt {
    Accepted,
    Coalesced,
}

#[derive(Clone, Debug)]
pub enum UiCommand {
    Refresh,
    PrepareAction {
        request: ActionRequest,
    },
    PrepareRepair {
        request: ControlledRepairRequest,
    },
    ConfirmAction {
        id: ActionPlanId,
    },
    CancelAction {
        id: ActionPlanId,
    },
    DismissOperation {
        operation_id: u64,
    },
    SetAutostart(bool),
    SetStartMinimized(bool),
    SetActiveProbe(bool),
    SetLanguage(LanguageCode),
    SetLogLevel(LogLevel),
    ExportDiagnostics,
    /// Re-list the module's stored messages. Queued for the runner's `SmsPort`; the controller
    /// itself only records the request. The user consent for the session-setting change is owned
    /// by the UI and must be confirmed before this command is dispatched.
    SmsRefresh,
    /// Read one stored message. Queued for the runner's `SmsPort`; the success path marks the
    /// stored copy read through [`Controller::mark_sms_read`].
    SmsRead {
        index: u32,
    },
    /// Delete one stored message. Queued for the runner's `SmsPort`; the local copy is removed
    /// only after the module confirms the deletion through [`Controller::confirm_sms_delete`].
    SmsDelete {
        index: u32,
    },
    /// Send one message. Dispatched **only after the UI has obtained the single user confirmation
    /// for this exact recipient and body** (research §6.3): the controller itself records the
    /// request and never asks again. The runner executes it against the `SmsPort` and records the
    /// terminal `Submitted`/`Failed`/`OutcomeUnknown` result as an outgoing store entry.
    SmsSend {
        recipient: String,
        body: String,
    },
    /// Run one whitelisted tool read on the module's AT port.
    RunToolRead {
        id: dji4g_at_protocol::ToolReadId,
    },
    /// Run every whitelisted tool read as one ordered batch under a shared budget.
    ProbeDeviceTools,
    /// Freeze one expert command and wait for its own confirmation. The text is validated before
    /// it is accepted and is never logged.
    PrepareExpertTool {
        line: dji4g_at_protocol::ValidatedToolLine,
    },
    /// Execute a frozen expert command. Only the id travels here, so the text cannot be changed
    /// between the confirmation and the write.
    ConfirmExpertTool {
        id: u64,
    },
    /// Stop waiting for a running tool task. The command may already have reached the module.
    CancelDeviceTool {
        id: u64,
    },
    /// Withdraw a frozen expert command before the user confirms it.
    CancelExpertToolPlan {
        id: u64,
    },
    /// Terminal result of the panel-side `config.toml` write for one settings revision.
    SettingsPersisted(crate::SettingsSaveOutcome),
    /// Terminal result of the panel-side autostart registration write.
    AutostartApplied(crate::AutostartApplyOutcome),
}

/// One module-side SMS operation queued by the controller for the runner's `SmsPort`.
///
/// The controller never touches the AT/module path itself: it records the user's intent and the
/// runner drains this queue with [`Controller::take_sms_requests`], executes each request against
/// the port, and feeds the outcome back through [`Controller::confirm_sms_delete`],
/// [`Controller::mark_sms_read`], [`Controller::ingest_sms`], and
/// [`Controller::record_sms_probe`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SmsRequest {
    Refresh,
    Read {
        index: u32,
    },
    Delete {
        index: u32,
    },
    /// One user-confirmed send. The controller never deduplicates `(recipient, body)`: every send
    /// is an explicit user action, so repeats are legitimate distinct submissions.
    Send {
        recipient: String,
        body: String,
    },
}

#[derive(Debug)]
pub(crate) struct RefreshSignal {
    requested: AtomicBool,
}

impl RefreshSignal {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
        }
    }

    pub(crate) fn request(&self) -> CommandReceipt {
        if self.requested.swap(true, Ordering::AcqRel) {
            CommandReceipt::Coalesced
        } else {
            CommandReceipt::Accepted
        }
    }

    pub(crate) fn take(&self) -> bool {
        self.requested.swap(false, Ordering::AcqRel)
    }
}

#[derive(Clone)]
pub struct ControllerHandle {
    sender: SyncSender<UiCommand>,
    refresh: Arc<RefreshSignal>,
    pub(crate) snapshot_tx: crate::sync::watch::Sender<Arc<ControllerSnapshot>>,
}

impl ControllerHandle {
    #[must_use]
    pub fn subscribe(&self) -> crate::sync::watch::Receiver<Arc<ControllerSnapshot>> {
        self.snapshot_tx.subscribe()
    }

    pub fn try_send(&self, command: UiCommand) -> Result<CommandReceipt, UiSendError> {
        if matches!(command, UiCommand::Refresh) {
            return Ok(self.refresh.request());
        }
        self.sender
            .try_send(command)
            .map(|()| CommandReceipt::Accepted)
            .map_err(|error| match error {
                TrySendError::Full(_) => UiSendError::QueueFull,
                TrySendError::Disconnected(_) => UiSendError::Closed,
            })
    }

    pub(crate) fn channels(
        initial: Arc<ControllerSnapshot>,
    ) -> (Self, mpsc::Receiver<UiCommand>, Arc<RefreshSignal>) {
        let (sender, receiver) = crate::sync::bounded(COMMAND_QUEUE_CAPACITY);
        let (snapshot_tx, _snapshot_rx) = crate::sync::watch::channel(initial);
        let refresh = Arc::new(RefreshSignal::new());
        (
            Self {
                sender,
                refresh: Arc::clone(&refresh),
                snapshot_tx,
            },
            receiver,
            refresh,
        )
    }
}

/// Synchronous core used by the async runner and deterministic application tests.
pub struct Controller {
    state: ReducerState,
    prepared: Option<StoredActionPlan>,
    consumed_plans: HashSet<ActionPlanId>,
    operation: Option<OperationUiSnapshot>,
    executor: Arc<dyn ActionExecutor>,
    test_executor: Option<Arc<FakeActionExecutor>>,
    clock: Arc<dyn Clock>,
    test_clock: Option<Arc<FakeClock>>,
    next_operation_id: u64,
    /// Receiver of the background executor's terminal result while a confirmation runs off the
    /// runner thread. Present only between `begin_confirmation` and the result being collected by
    /// `poll_operation_completion`.
    operation_pending: Option<mpsc::Receiver<Result<crate::ExecutionReceipt, PortError>>>,
    /// Last user-visible command rejection, published in every snapshot until the next command
    /// is accepted.
    feedback: Option<crate::UiFeedback>,
    feedback_seq: u64,
    /// SMS operations the runner's `SmsPort` implementation will execute, in request order.
    sms_requests: VecDeque<SmsRequest>,
    sms_send: Option<crate::SmsSendSnapshot>,
    sms_refresh_pending: bool,
    sms_inbox_failure: Option<(DeviceEpoch, u64, crate::PortError)>,
    next_sms_request_id: u64,
    sms_send_context: Option<(DeviceEpoch, u64)>,
    /// Tool tasks the runner will execute, in request order. The controller owns the queue and the
    /// evidence; the runner owns the worker and the port call.
    tool_requests: VecDeque<crate::ToolRequest>,
    device_tools: crate::DeviceToolsSnapshot,
    next_tool_request_id: u64,
    /// A frozen expert command awaiting its single confirmation.
    expert_plan: Option<ExpertToolPlan>,
}

impl Controller {
    #[must_use]
    pub fn for_test(now: SystemTime) -> Self {
        let executor = Arc::new(FakeActionExecutor::new());
        let clock = Arc::new(FakeClock::new(now));
        Self {
            state: ReducerState::test_ready(now),
            prepared: None,
            consumed_plans: HashSet::new(),
            operation: None,
            executor: Arc::clone(&executor) as Arc<dyn ActionExecutor>,
            test_executor: Some(executor),
            clock: Arc::clone(&clock) as Arc<dyn Clock>,
            test_clock: Some(clock),
            next_operation_id: 1,
            operation_pending: None,
            feedback: None,
            feedback_seq: 0,
            sms_requests: VecDeque::new(),
            sms_send: None,
            sms_refresh_pending: false,
            sms_inbox_failure: None,
            next_sms_request_id: 1,
            sms_send_context: None,
            tool_requests: VecDeque::new(),
            device_tools: crate::DeviceToolsSnapshot::default(),
            next_tool_request_id: 1,
            expert_plan: None,
        }
    }

    #[must_use]
    pub fn new(
        state: ReducerState,
        executor: Arc<dyn ActionExecutor>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            state,
            prepared: None,
            consumed_plans: HashSet::new(),
            operation: None,
            executor,
            test_executor: None,
            clock,
            test_clock: None,
            next_operation_id: 1,
            operation_pending: None,
            feedback: None,
            feedback_seq: 0,
            sms_requests: VecDeque::new(),
            sms_send: None,
            sms_refresh_pending: false,
            sms_inbox_failure: None,
            next_sms_request_id: 1,
            sms_send_context: None,
            tool_requests: VecDeque::new(),
            device_tools: crate::DeviceToolsSnapshot::default(),
            next_tool_request_id: 1,
            expert_plan: None,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> ControllerSnapshot {
        let mut snapshot = self.state.snapshot_with_at(
            self.prepared.as_ref().map(|stored| stored.summary.clone()),
            self.operation.clone(),
            self.clock.system_now(),
        );
        snapshot.feedback = self.feedback.clone();
        snapshot.sms_send = self.sms_send.clone();
        snapshot.sms_refresh_pending = self.sms_refresh_pending;
        snapshot.sms_inbox_failure = self
            .sms_inbox_failure
            .as_ref()
            .filter(|(epoch, sim, _)| {
                *epoch == self.state.epoch() && *sim == self.state.sim_epoch()
            })
            .map(|(_, _, error)| error.clone());
        snapshot.device_tools = self.device_tools.clone();
        snapshot
    }

    #[must_use]
    pub fn state(&self) -> &ReducerState {
        &self.state
    }

    pub fn prepare_action(&mut self, action: ActionRequest) -> Result<ActionPlanId, PrepareError> {
        if self.sms_active() || self.tool_active() {
            return Err(PrepareError::Busy);
        }
        if self.prepared.as_ref().is_some_and(|plan| {
            matches!(
                plan.summary.state,
                PreparedActionState::AwaitingConfirmation
            )
        }) || self
            .operation
            .as_ref()
            .is_some_and(|operation| matches!(operation.state, OperationState::Running { .. }))
        {
            return Err(PrepareError::Busy);
        }
        if matches!(action, ActionKind::Refresh) {
            return Err(PrepareError::UnsupportedAction);
        }
        let now = self.clock.system_now();
        self.state
            .action_prerequisite(&action, now)
            .map_err(PrepareError::Prerequisite)?;
        let target = self
            .state
            .target_identity()
            .ok_or(PrepareError::MissingTarget)?;
        let hash = before_state_hash(&self.state, &action).ok_or_else(|| {
            PrepareError::Prerequisite(failure(
                ErrorCode::EvidenceExpired,
                "app:missing_before_state",
            ))
        })?;
        let id = next_plan_id();
        let stored = build_plan(id, action, &self.state, target, hash, now)?;
        self.prepared = Some(stored);
        // Prepared state is publication-only. Evidence revision remains unchanged.
        self.state.publish_only_change();
        Ok(id)
    }

    /// Prepare one of the typed Task11 repairs.  This boundary keeps the historical domain action
    /// enum out of the UI while retaining the reviewed DNS, hotspot, and physical re-enumeration
    /// operations.
    pub fn prepare_repair(
        &mut self,
        request: ControlledRepairRequest,
    ) -> Result<ActionPlanId, PrepareError> {
        self.prepare_action(request.into_action())
    }

    /// Synchronous confirmation: consume the plan, execute on the calling thread, finish.
    ///
    /// Kept for deterministic tests and any blocking consumer; the production runner uses
    /// [`Self::begin_confirmation`] so it never blocks on UAC, IPC, or hotspot work.
    pub fn confirm_action(&mut self, id: ActionPlanId) -> Result<ConfirmResult, ConfirmError> {
        if self.sms_active() {
            return Err(ConfirmError::Busy);
        }
        let (token, _requires_elevation) = self.revalidate_and_consume(id)?;
        let now = self.clock.system_now();
        let receipt = self.executor.execute_once(token);
        self.set_operation_phase(OperationPhase::Verifying);
        let outcome = map_execution(receipt);
        self.finish_operation(outcome, now);
        Ok(ConfirmResult::Executed)
    }

    /// Start one already-confirmed operation on a background executor thread and return
    /// immediately, so the runner thread never blocks on UAC, IPC, or hotspot work.
    ///
    /// The runner polls [`Self::poll_operation_completion`] every loop iteration; the Running
    /// phases are published here so the UI sees honest progress while the executor works.
    pub fn begin_confirmation(&mut self, id: ActionPlanId) -> Result<ConfirmResult, ConfirmError> {
        if self.sms_active() {
            return Err(ConfirmError::Busy);
        }
        let (token, requires_elevation) = self.revalidate_and_consume(id)?;
        if requires_elevation {
            self.set_operation_phase(OperationPhase::AwaitingElevation);
        }
        self.set_operation_phase(OperationPhase::Executing);

        let (sender, receiver) = mpsc::channel();
        self.operation_pending = Some(receiver);
        let executor = Arc::clone(&self.executor);
        let spawn = std::thread::Builder::new()
            .name("dji4g-operation-executor".to_owned())
            .spawn(move || {
                let receipt = executor.execute_once(token);
                // A late result after the runner gave up is dropped deliberately; the terminal
                // state the runner already published is the honest one.
                let _ = sender.send(receipt);
            });
        if spawn.is_err() {
            // A worker that could not start must not leave the operation Running forever: finish
            // it as a definite internal failure and surface the rejection.
            self.operation_pending = None;
            self.finish_operation(
                map_execution(Err(PortError::new(
                    ErrorCode::Internal,
                    "app:executor_worker_unavailable",
                ))),
                self.clock.system_now(),
            );
            return Err(ConfirmError::ExecutionFailed);
        }
        Ok(ConfirmResult::Executed)
    }

    /// Collect the background operation's terminal result when it has arrived. Returns the
    /// operation id when one reached `Finished` in this call.
    ///
    /// A dropped channel means the worker thread died without reporting (panic); the operation
    /// is finished as a definite internal failure rather than left running forever.
    pub fn poll_operation_completion(&mut self) -> Option<u64> {
        let receiver = self.operation_pending.as_ref()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(PortError::new(
                ErrorCode::Internal,
                "app:executor_worker_lost",
            )),
        };
        self.operation_pending = None;
        let operation_id = self
            .operation
            .as_ref()
            .map(|operation| operation.operation_id);
        self.finish_operation(map_execution(result), self.clock.system_now());
        operation_id
    }

    /// Shared revalidation and consume point for both the synchronous test path and the
    /// background runner path: the plan is consumed exactly once, an operation record is
    /// created in `Running { Revalidating }`, and the capability token is minted.
    fn revalidate_and_consume(
        &mut self,
        id: ActionPlanId,
    ) -> Result<(ValidatedActionToken, bool), ConfirmError> {
        let Some(stored) = self.prepared.as_ref() else {
            return Err(if self.consumed_plans.contains(&id) {
                ConfirmError::AlreadyConsumed
            } else {
                ConfirmError::UnknownPlan
            });
        };
        if stored.id != id {
            return Err(if self.consumed_plans.contains(&id) {
                ConfirmError::AlreadyConsumed
            } else {
                ConfirmError::UnknownPlan
            });
        }
        if let PreparedActionState::Invalidated { reason } = stored.summary.state {
            return Err(ConfirmError::Invalidated(reason));
        }

        let now = self.clock.system_now();
        if now > stored.plan.expires_at {
            self.invalidate_prepared(ConfirmationInvalidationReason::Expired);
            return Err(ConfirmError::Invalidated(
                ConfirmationInvalidationReason::Expired,
            ));
        }
        let Some(target) = self.state.target_identity() else {
            self.invalidate_prepared(ConfirmationInvalidationReason::DeviceRemoved);
            return Err(ConfirmError::Invalidated(
                ConfirmationInvalidationReason::DeviceRemoved,
            ));
        };
        let Some(hash) = before_state_hash(&self.state, &stored.plan.kind) else {
            self.invalidate_prepared(ConfirmationInvalidationReason::BeforeStateChanged);
            return Err(ConfirmError::Invalidated(
                ConfirmationInvalidationReason::BeforeStateChanged,
            ));
        };
        if let Err(error) = stored.plan.validate_for_execution(
            self.state.evidence_revision(),
            self.state.epoch(),
            &target,
            hash,
            now,
        ) {
            let reason = ConfirmationInvalidationReason::from(error);
            self.invalidate_prepared(reason);
            return Err(ConfirmError::Invalidated(reason));
        }

        // This is the atomic consume point: no later path can execute the same plan id.
        let consumed = self.prepared.take().expect("prepared plan checked above");
        let requires_elevation = consumed.plan.requires_elevation;
        self.consumed_plans.insert(id);
        let tag =
            ActionKindTag::from_action(&consumed.plan.kind).ok_or(ConfirmError::UnknownPlan)?;
        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.saturating_add(1);
        self.operation = Some(OperationUiSnapshot {
            operation_id,
            action: tag,
            started_at: now,
            state: OperationState::Running {
                phase: OperationPhase::Revalidating,
            },
        });
        self.state.publish_only_change();
        Ok((mint_token(id, &consumed.plan), requires_elevation))
    }

    pub fn cancel_action(&mut self, id: ActionPlanId) -> Result<(), ConfirmError> {
        let Some(stored) = self.prepared.as_ref() else {
            return Err(if self.consumed_plans.contains(&id) {
                ConfirmError::AlreadyConsumed
            } else {
                ConfirmError::UnknownPlan
            });
        };
        if stored.id != id {
            return Err(ConfirmError::UnknownPlan);
        }
        // Cancelling removes the plan outright: an abandoned confirmation must close the modal,
        // never leave a dead 「已过期/已失效」 card that can no longer be dismissed.
        self.prepared = None;
        self.state.publish_only_change();
        Ok(())
    }

    pub fn dismiss_operation(&mut self, operation_id: u64) -> Result<(), ConfirmError> {
        if self
            .operation
            .as_ref()
            .is_some_and(|operation| operation.operation_id == operation_id)
        {
            self.operation = None;
            self.state.publish_only_change();
            Ok(())
        } else {
            Err(ConfirmError::UnknownPlan)
        }
    }

    pub fn set_language(&mut self, language: LanguageCode) {
        self.state.set_language(language);
    }

    pub fn set_start_minimized(&mut self, value: bool) {
        self.state.set_start_minimized(value);
    }

    pub fn set_active_probe(&mut self, value: bool) {
        self.state.set_active_probe(value);
    }

    pub fn set_log_level(&mut self, value: LogLevel) {
        self.state.set_log_level(value);
    }

    pub fn set_autostart_state(&mut self, state: crate::AutostartStatus) {
        self.state.set_autostart(state);
    }

    pub fn handle_command(&mut self, command: UiCommand) -> Result<CommandReceipt, UiSendError> {
        let result = self.handle_command_inner(command);
        match result {
            Ok(receipt) => {
                // A command the controller accepted supersedes the previous rejection notice.
                self.feedback = None;
                Ok(receipt)
            }
            Err(error) => Err(error),
        }
    }

    fn handle_command_inner(&mut self, command: UiCommand) -> Result<CommandReceipt, UiSendError> {
        match command {
            UiCommand::Refresh => Ok(CommandReceipt::Accepted),
            UiCommand::PrepareAction { request } => self
                .prepare_action(request)
                .map(|_| CommandReceipt::Accepted)
                .map_err(|error| {
                    self.report_feedback(prepare_feedback(error));
                    UiSendError::Closed
                }),
            UiCommand::PrepareRepair { request } => self
                .prepare_repair(request)
                .map(|_| CommandReceipt::Accepted)
                .map_err(|error| {
                    self.report_feedback(prepare_feedback(error));
                    UiSendError::Closed
                }),
            UiCommand::ConfirmAction { id } => self
                .begin_confirmation(id)
                .map(|_| CommandReceipt::Accepted)
                .map_err(|error| {
                    self.report_feedback(confirm_feedback(error));
                    UiSendError::Closed
                }),
            UiCommand::CancelAction { id } => self
                .cancel_action(id)
                .map(|_| CommandReceipt::Accepted)
                // A second cancel for a plan that is already gone is a no-op, not a rejection:
                // the modal is already closed, so no feedback toast is warranted.
                .map_err(|_| UiSendError::Closed),
            UiCommand::DismissOperation { operation_id } => self
                .dismiss_operation(operation_id)
                .map(|_| CommandReceipt::Accepted)
                .map_err(|error| {
                    self.report_feedback(confirm_feedback(error));
                    UiSendError::Closed
                }),
            UiCommand::SetAutostart(enabled) => {
                self.state.begin_autostart_change(enabled);
                Ok(CommandReceipt::Accepted)
            }
            // Each user-visible settings change implies a pending `config.toml` write until the
            // panel reports the terminal result. Only a real value change marks it pending: the
            // panel triggers on the settings revision, so marking a no-op change would strand the
            // state in `Saving` forever.
            UiCommand::SetStartMinimized(value) => {
                let revision_before = self.state.settings_revision();
                self.set_start_minimized(value);
                if self.state.settings_revision() != revision_before {
                    self.state.begin_settings_persistence();
                }
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::SetActiveProbe(value) => {
                let revision_before = self.state.settings_revision();
                self.set_active_probe(value);
                if self.state.settings_revision() != revision_before {
                    self.state.begin_settings_persistence();
                }
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::SetLanguage(value) => {
                let revision_before = self.state.settings_revision();
                self.set_language(value);
                if self.state.settings_revision() != revision_before {
                    self.state.begin_settings_persistence();
                }
                Ok(CommandReceipt::Accepted)
            }
            // The new level is stored and persisted immediately, but the logging pipeline is
            // initialized once at startup; the change takes effect on the next start. Reconfiguring
            // in place would be dishonest here: `logging::reload_level` only flips a counter that
            // no production filter currently reads.
            UiCommand::SetLogLevel(value) => {
                let revision_before = self.state.settings_revision();
                self.set_log_level(value);
                if self.state.settings_revision() != revision_before {
                    self.state.begin_settings_persistence();
                }
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::SmsRefresh => {
                if self.sms_active() {
                    return self.reject_sms_busy();
                }
                if self.tool_active() {
                    return self.reject_sms_tool_busy();
                }
                if self.sms_refresh_pending {
                    return Ok(CommandReceipt::Accepted);
                }
                self.sms_refresh_pending = true;
                self.sms_requests.push_back(SmsRequest::Refresh);
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::SmsRead { index } => {
                if self.sms_active() {
                    return self.reject_sms_busy();
                }
                // A tool task holds the port: the read is refused rather than queued behind a
                // device that may be gone by the time the tool finishes.
                if self.tool_active() {
                    return self.reject_sms_tool_busy();
                }
                self.sms_requests.push_back(SmsRequest::Read { index });
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::SmsDelete { index } => {
                if self.sms_active() {
                    return self.reject_sms_busy();
                }
                if self.tool_active() {
                    return self.reject_sms_tool_busy();
                }
                self.sms_requests.push_back(SmsRequest::Delete { index });
                // The local copy is deliberately kept until the module confirms the deletion
                // through `confirm_sms_delete`: a queued or failed delete must never make the
                // inbox lie about what is still stored on the module.
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::SmsSend { recipient, body } => {
                if self.sms_active() || self.tool_active() {
                    return self.reject_sms_tool_busy();
                }
                if self.operation_pending.is_some()
                    || self
                        .operation
                        .as_ref()
                        .is_some_and(|op| matches!(op.state, OperationState::Running { .. }))
                {
                    return self.reject_sms_busy();
                }
                let request_id = self.next_sms_request_id;
                self.sms_send_context = Some((self.state.epoch(), self.snapshot().sim_epoch));
                self.next_sms_request_id = self.next_sms_request_id.saturating_add(1);
                self.sms_send = Some(crate::SmsSendSnapshot {
                    request_id,
                    phase: crate::SmsSendPhase::Queued,
                    result: None,
                    failure: None,
                });
                self.state.publish_only_change();
                // The UI has already taken the single per-send confirmation for this exact
                // recipient and body; the controller only queues the intent. The outgoing store
                // record is written by the runner once the transaction completes, so a queued or
                // impossible send never appears before its attempt.
                self.sms_requests
                    .push_back(SmsRequest::Send { recipient, body });
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::RunToolRead { id } => {
                self.queue_tool_request(crate::ToolOperation::Read(id), 1)
            }
            UiCommand::ProbeDeviceTools => self.queue_tool_request(
                crate::ToolOperation::ProbeAll,
                dji4g_at_protocol::ToolReadId::ALL.len(),
            ),
            UiCommand::PrepareExpertTool { line } => self.prepare_expert_tool(line),
            UiCommand::ConfirmExpertTool { id } => self.confirm_expert_tool(id),
            UiCommand::CancelDeviceTool { id } => self.cancel_device_tool(id),
            UiCommand::CancelExpertToolPlan { id } => self.cancel_expert_plan(id),
            UiCommand::SettingsPersisted(outcome) => {
                self.state
                    .finish_settings_persistence(outcome.revision, outcome.result);
                Ok(CommandReceipt::Accepted)
            }
            UiCommand::AutostartApplied(outcome) => {
                self.state.finish_autostart(outcome);
                Ok(CommandReceipt::Accepted)
            }
            // ExportDiagnostics is fulfilled entirely by the UI from its own snapshot and is
            // intercepted before dispatch, so the controller never receives it. The variant
            // stays in the closed `UiCommand` set for the UI's local path; reaching this arm
            // is a dispatch bug and is rejected instead of silently accepted.
            UiCommand::ExportDiagnostics => Err(UiSendError::Closed),
        }
    }

    // -------------------------------------------------------------------------------------
    // Device tools
    // -------------------------------------------------------------------------------------

    /// Whether a tool task is queued, running or waiting to be reclaimed.
    ///
    /// Every other serial work entry point consults this through `serial_work_busy`, so a tool
    /// task, an SMS transaction and a repair can never overlap on one module.
    #[must_use]
    pub fn tool_active(&self) -> bool {
        !self.tool_requests.is_empty() || self.device_tools.busy()
    }

    /// Whether any serial work is in flight: the one predicate every entry point uses.
    #[must_use]
    pub fn serial_work_busy(&self) -> bool {
        self.sms_active()
            || self.tool_active()
            || self.operation_pending.is_some()
            || self
                .operation
                .as_ref()
                .is_some_and(|op| matches!(op.state, OperationState::Running { .. }))
    }

    #[must_use]
    pub fn device_tools(&self) -> &crate::DeviceToolsSnapshot {
        &self.device_tools
    }

    /// The device context a tool task must match to be allowed to run.
    fn tool_context(&self) -> Option<crate::ToolContext> {
        let identity = self.state.target_identity()?;
        Some(crate::ToolContext {
            device_epoch: self.state.epoch(),
            sim_epoch: self.state.sim_epoch(),
            identity,
            at_port: self
                .state
                .inventory_at_port()
                .unwrap_or_else(|| "[unknown]".to_owned()),
        })
    }

    fn queue_tool_request(
        &mut self,
        operation: crate::ToolOperation,
        total_items: usize,
    ) -> Result<CommandReceipt, UiSendError> {
        if self.serial_work_busy() {
            return self.reject_tool_busy();
        }
        let Some(context) = self.tool_context() else {
            self.device_tools.last_refusal = Some(crate::ToolOutcome::TransportFailure);
            self.state.publish_only_change();
            return Err(UiSendError::Closed);
        };
        let id = self.next_tool_request_id;
        self.next_tool_request_id = self.next_tool_request_id.saturating_add(1);
        let request = crate::ToolRequest {
            id,
            context,
            operation,
        };
        self.device_tools.task = Some(crate::ToolTaskSnapshot::new(&request, total_items));
        self.device_tools.last_refusal = None;
        self.state.publish_only_change();
        self.tool_requests.push_back(request);
        Ok(CommandReceipt::Accepted)
    }

    fn reject_tool_busy(&mut self) -> Result<CommandReceipt, UiSendError> {
        self.device_tools.last_refusal = Some(crate::ToolOutcome::Rejected);
        self.report_feedback(failure(ErrorCode::Internal, "tool:busy"));
        self.state.publish_only_change();
        Err(UiSendError::QueueFull)
    }

    /// Freeze one expert command until the user confirms it.
    fn prepare_expert_tool(
        &mut self,
        line: dji4g_at_protocol::ValidatedToolLine,
    ) -> Result<CommandReceipt, UiSendError> {
        if self.serial_work_busy() {
            return self.reject_tool_busy();
        }
        let Some(context) = self.tool_context() else {
            return Err(UiSendError::Closed);
        };
        if self.expert_plan.is_some() {
            // One frozen command at a time: preparing another replaces nothing and is refused.
            return self.reject_expert(ExpertToolRefusal::AlreadyFrozen);
        }
        let id = self.next_tool_request_id;
        self.next_tool_request_id = self.next_tool_request_id.saturating_add(1);
        let expires_at = self.clock.system_now() + crate::PLAN_LIFETIME;
        self.device_tools.pending_expert = Some(crate::PendingExpertTool {
            id,
            line: line.clone(),
            expires_at,
        });
        // The frozen request is published so a device or SIM change can withdraw it, but it is not
        // queued yet: nothing runs until the user confirms this exact text.
        self.expert_plan = Some(ExpertToolPlan {
            id,
            context,
            line,
            expires_at,
            consumed: false,
        });
        self.device_tools.last_refusal = None;
        self.state.publish_only_change();
        Ok(CommandReceipt::Accepted)
    }

    fn clear_expert_plan(&mut self) {
        self.expert_plan = None;
        self.device_tools.pending_expert = None;
    }

    fn cancel_expert_plan(&mut self, id: u64) -> Result<CommandReceipt, UiSendError> {
        if self.expert_plan.as_ref().is_some_and(|plan| plan.id == id) {
            self.clear_expert_plan();
            self.state.publish_only_change();
        }
        Ok(CommandReceipt::Accepted)
    }

    /// Confirm a frozen expert command exactly once.
    ///
    /// Everything about the request was frozen at prepare time: the text, the device epoch, the
    /// SIM epoch, the identity and the port. Confirmation re-checks all of them, and an expired,
    /// already-confirmed, or context-mismatched plan is refused with a distinct reason.
    fn confirm_expert_tool(&mut self, id: u64) -> Result<CommandReceipt, UiSendError> {
        let Some(plan) = self.expert_plan.as_ref() else {
            return self.reject_expert(ExpertToolRefusal::UnknownPlan);
        };
        if plan.id != id {
            return self.reject_expert(ExpertToolRefusal::UnknownPlan);
        }
        if plan.consumed {
            return self.reject_expert(ExpertToolRefusal::AlreadyConfirmed);
        }
        if self.clock.system_now() > plan.expires_at {
            self.clear_expert_plan();
            return self.reject_expert(ExpertToolRefusal::Expired);
        }
        let frozen_context = plan.context.clone();
        let line = plan.line.clone();
        let Some(context) = self.tool_context() else {
            return self.reject_expert(ExpertToolRefusal::ContextChanged);
        };
        if context != frozen_context {
            self.clear_expert_plan();
            return self.reject_expert(ExpertToolRefusal::ContextChanged);
        }
        if self.serial_work_busy() {
            return self.reject_tool_busy();
        }
        self.clear_expert_plan();
        // Reuse the ordinary queue so an expert command obeys the same mutual exclusion as every
        // other serial transaction.
        let request = crate::ToolRequest {
            id,
            context,
            operation: crate::ToolOperation::Expert(line),
        };
        self.device_tools.task = Some(crate::ToolTaskSnapshot::new(&request, 1));
        self.tool_requests.push_back(request);
        self.state.publish_only_change();
        Ok(CommandReceipt::Accepted)
    }

    fn reject_expert(&mut self, refusal: ExpertToolRefusal) -> Result<CommandReceipt, UiSendError> {
        self.device_tools.last_refusal = Some(crate::ToolOutcome::Rejected);
        self.report_feedback(failure(ErrorCode::Internal, refusal.code()));
        Err(UiSendError::Closed)
    }

    fn cancel_device_tool(&mut self, id: u64) -> Result<CommandReceipt, UiSendError> {
        // Dropping a queued request is an application-side decision; a running task is cancelled
        // by the runner through the same control handle the user's button reaches.
        self.tool_requests.retain(|request| request.id != id);
        if let Some(task) = self.device_tools.task.as_mut() {
            if task.id == id {
                if task.phase == crate::ToolPhase::Queued {
                    task.phase = crate::ToolPhase::Finished;
                    task.outcome = Some(crate::ToolOutcome::CancelledBeforeWrite);
                } else if task.phase.is_active() {
                    task.phase = crate::ToolPhase::Cancelling;
                }
            }
        }
        self.state.publish_only_change();
        Ok(CommandReceipt::Accepted)
    }

    /// Take the next queued tool request. The runner calls this only when no task is in flight.
    pub fn take_next_tool_request(&mut self) -> Option<crate::ToolRequest> {
        self.tool_requests.pop_front()
    }

    pub fn set_tool_phase(&mut self, id: u64, phase: crate::ToolPhase) {
        if let Some(task) = self.device_tools.task.as_mut() {
            if task.id == id {
                task.phase = phase;
            }
        }
        self.state.publish_only_change();
    }

    pub fn record_tool_progress(&mut self, id: u64, completed: usize, total: usize) {
        if let Some(task) = self.device_tools.task.as_mut() {
            if task.id == id {
                task.completed_items = completed;
                task.total_items = total;
            }
        }
        self.state.publish_only_change();
    }

    pub fn record_tool_capability(&mut self, row: crate::ToolCapabilityRow) {
        self.device_tools.record_capability(row);
        self.state.publish_only_change();
    }

    /// Apply one partial profile update.
    ///
    /// The closure receives only the fields the caller actually learned, so a read that failed
    /// leaves the previous value alone instead of overwriting it with a default. Only parsed
    /// values are ever stored.
    pub fn update_tool_profile(
        &mut self,
        context: &crate::ToolContext,
        update: impl FnOnce(&mut crate::ModuleProfile),
    ) {
        update(&mut self.device_tools.profile);
        self.device_tools.profile.observed_at = Some(self.clock.system_now());
        self.device_tools.profile.context = Some(context.clone());
        self.state.publish_only_change();
    }

    /// Whether any *other* serial work would conflict with starting a tool task right now.
    ///
    /// Re-checked at start time, not only when the request was queued: an SMS transaction or a
    /// repair can have begun while the tool request waited.
    #[must_use]
    pub fn tool_start_conflict(&self) -> bool {
        let now = self.clock.system_now();
        self.sms_active()
            || !self.sms_requests.is_empty()
            || self.operation_pending.is_some()
            || self
                .operation
                .as_ref()
                .is_some_and(|op| matches!(op.state, OperationState::Running { .. }))
            || self.prepared.as_ref().is_some_and(|stored| {
                matches!(
                    stored.summary.state,
                    PreparedActionState::AwaitingConfirmation
                ) && now <= stored.plan.expires_at
            })
    }

    /// Finish one tool task and publish its evidence.
    pub fn finish_tool_task(&mut self, receipt: crate::ToolReceipt) {
        let finished_at = self.clock.system_now();
        self.device_tools.history.push(crate::ToolHistoryEntry {
            id: receipt.id,
            operation: receipt.operation,
            outcome: receipt.outcome,
            elapsed: receipt.elapsed,
            finished_at,
            transcript: std::sync::Arc::clone(&receipt.transcript),
        });
        if self
            .device_tools
            .task
            .as_ref()
            .is_some_and(|task| task.id == receipt.id)
        {
            if let Some(task) = self.device_tools.task.as_mut() {
                task.phase = crate::ToolPhase::Finished;
                task.outcome = Some(receipt.outcome);
            }
        }
        self.state.publish_only_change();
    }

    /// Record a refusal that happened before any command was written.
    pub fn refuse_tool_task(&mut self, id: u64, outcome: crate::ToolOutcome) {
        self.device_tools.last_refusal = Some(outcome);
        if let Some(task) = self.device_tools.task.as_mut() {
            if task.id == id {
                task.phase = crate::ToolPhase::Finished;
                task.outcome = Some(outcome);
            }
        }
        self.state.publish_only_change();
    }

    /// Mark a batch's task row finished with its terminal outcome.
    pub fn finish_tool_batch_task(&mut self, id: u64, outcome: crate::ToolOutcome) {
        if let Some(task) = self.device_tools.task.as_mut() {
            if task.id == id {
                task.phase = crate::ToolPhase::Finished;
                task.outcome = Some(outcome);
            }
        }
        self.state.publish_only_change();
    }

    pub fn clear_tool_task(&mut self) {
        self.device_tools.task = None;
        self.state.publish_only_change();
    }

    pub fn clear_tool_history(&mut self) {
        self.device_tools.history.clear();
        self.state.publish_only_change();
    }

    /// Drop every conclusion tied to the previous device or SIM.
    fn invalidate_tool_context(&mut self) {
        self.tool_requests.clear();
        self.device_tools.invalidate_context();
        self.expert_plan = None;
    }

    pub fn advance_time(&mut self, by: Duration) {
        if let Some(clock) = &self.test_clock {
            clock.advance_wall(by);
        }
        self.expire_if_needed();
    }

    pub fn accept_evidence_change(&mut self) {
        self.state.bump_evidence_for_test();
        self.invalidate_prepared(ConfirmationInvalidationReason::SnapshotChanged);
    }

    /// Apply one backend observation event (a monitor DAG result or an epoch invalidation) to the
    /// controller.  A SIM identity change proven by the AT evidence advances `sim_epoch`; whenever
    /// the evidence revision or the SIM epoch moves, a prepared plan awaiting confirmation is
    /// invalidated so a swap or a ground-level change can never confirm against stale state.
    pub fn apply_backend_event(&mut self, event: BackendEvent) {
        let before = self.state.evidence_revision();
        let sim_epoch_before = self.state.sim_epoch();
        let now = self.clock.system_now();
        let invalidates_for_removal = matches!(
            &event,
            BackendEvent::EpochInvalidated {
                reason: EpochInvalidationReason::PhysicalRemoval,
                ..
            }
        );
        self.state = crate::reduce_state(&self.state, event, now);
        let evidence_changed = self.state.evidence_revision() != before;
        let sim_changed = self.state.sim_epoch() != sim_epoch_before;
        if evidence_changed || sim_changed {
            self.invalidate_prepared(if invalidates_for_removal {
                ConfirmationInvalidationReason::DeviceRemoved
            } else {
                ConfirmationInvalidationReason::SnapshotChanged
            });
        }
    }

    /// Record one module-feature verdict from the collection layer.  The verdict is scoped to the
    /// current device context; a firmware change clears the cached capability statuses (§8.1).  A
    /// no-op verdict (unchanged, or no device context anchored yet) does not republish.
    pub fn set_feature_status(&mut self, key: FeatureKey, status: FeatureStatus) {
        if self.state.set_feature_status(key, status) {
            self.state.publish_only_change();
        }
    }

    /// Store one listed SMS message; returns whether it was new. The runner feeds listed messages
    /// through this after the `SmsPort` returns, and the snapshot's `sms_inbox` follows.
    pub fn ingest_sms(&mut self, mut message: SmsMessage) -> bool {
        // The port layer cannot know the current SIM session; stamp it here so dedup and
        // display stay scoped to the SIM that is actually present.
        message.sim_epoch = self.state.sim_epoch();
        self.state.ingest_sms(message)
    }

    /// Mark one stored message read after the module-side read succeeded; republishes on change.
    ///
    /// The `(index, storage)` pair from the message the port returned identifies the stored copy:
    /// a read that resolved a different storage holder than the listing must not touch another
    /// holder's entry.
    pub fn mark_sms_read(&mut self, message: &SmsMessage) -> bool {
        self.state.mark_sms_read(message.index, &message.storage)
    }

    /// Remove the message with this index after the module confirmed the deletion; republishes on
    /// change. Runs only on the runner thread from the `SmsPort` success path — never at command
    /// time, so a failed or queued delete cannot hide a message that is still on the module.
    pub fn confirm_sms_delete(&mut self, index: u32) -> bool {
        self.state.remove_sms_by_index(index)
    }

    /// Record the latest module-side SMS probe status and `CPMS` capacity; republishes on change.
    /// The runner uses this for both successes (with the listed capacity) and failures, so the
    /// published inbox status reflects the last real transaction, not a fabricated one.
    pub fn set_sms_inbox_failure(&mut self, error: Option<crate::PortError>) {
        self.sms_inbox_failure =
            error.map(|error| (self.state.epoch(), self.state.sim_epoch(), error));
    }

    pub fn record_sms_probe(&mut self, status: FeatureStatus, capacity: Option<(u32, u32)>) {
        self.state.record_sms_probe(status, capacity);
    }

    /// Drain the queued module-side SMS operations, oldest first, so the runner's `SmsPort`
    /// implementation can execute them without the controller ever touching the module path.
    pub fn take_sms_requests(&mut self) -> Vec<SmsRequest> {
        self.sms_requests.drain(..).collect()
    }

    pub fn take_next_sms_request(&mut self) -> Option<SmsRequest> {
        self.sms_requests.pop_front()
    }
    pub fn sms_active(&self) -> bool {
        self.sms_send
            .as_ref()
            .is_some_and(crate::SmsSendSnapshot::is_active)
    }
    pub fn sms_send_context(&self) -> Option<(DeviceEpoch, u64)> {
        self.sms_send_context
    }
    fn reject_sms_busy(&mut self) -> Result<CommandReceipt, UiSendError> {
        self.report_feedback(failure(ErrorCode::Internal, "sms:busy"));
        Err(UiSendError::QueueFull)
    }

    /// The module's port is occupied by a device-tool task. The message is distinct from a busy
    /// SMS transaction so the UI can say which one is holding the port.
    fn reject_sms_tool_busy(&mut self) -> Result<CommandReceipt, UiSendError> {
        self.report_feedback(failure(ErrorCode::Internal, "tool:busy"));
        Err(UiSendError::QueueFull)
    }
    pub fn set_sms_refresh_pending(&mut self, pending: bool) {
        self.sms_refresh_pending = pending;
        self.state.publish_only_change();
    }
    pub fn update_sms_send(
        &mut self,
        phase: crate::SmsSendPhase,
        result: Option<crate::SmsSendResult>,
        failure: Option<crate::SmsFailureDetail>,
    ) {
        if let Some(send) = self.sms_send.as_mut() {
            if send.phase != phase || send.result != result || send.failure != failure {
                send.phase = phase;
                send.result = result;
                send.failure = failure;
                self.state.publish_only_change();
            }
        }
    }

    pub(crate) fn next_cycle(&self) -> crate::RefreshCycleId {
        crate::RefreshCycleId(self.state.current_cycle().0.saturating_add(1))
    }

    pub(crate) fn now(&self) -> SystemTime {
        self.clock.system_now()
    }

    pub fn invalidate_epoch(&mut self, next: DeviceEpoch, reason: EpochInvalidationReason) {
        let before_epoch = self.state.epoch();
        self.state = crate::reduce_state(
            &self.state,
            BackendEvent::EpochInvalidated {
                next_epoch: next,
                reason,
            },
            self.clock.system_now(),
        );
        if self.state.epoch() != before_epoch {
            self.invalidate_prepared(
                if matches!(reason, EpochInvalidationReason::PhysicalRemoval) {
                    ConfirmationInvalidationReason::DeviceRemoved
                } else {
                    ConfirmationInvalidationReason::EpochChanged
                },
            );
            // Capability evidence and a frozen expert command belong to one device and one SIM:
            // neither may survive a swap.
            self.invalidate_tool_context();
        }
    }

    /// Whether an explicit user interaction is in progress right now.
    ///
    /// True while a prepared plan is still `AwaitingConfirmation` **and** inside its own
    /// `PLAN_LIFETIME` window, or while an operation is still `Running`.
    ///
    /// The automatic monitoring cadence uses this to yield.  Every scan records fresh evidence and
    /// therefore bumps `evidence_revision`, and any revision bump invalidates a plan awaiting
    /// confirmation, so an unconditional timer would tear down a repair the user is about to
    /// confirm every `REFRESH_INTERVAL`.  Yielding does not weaken safety: `confirm_action` still
    /// re-validates expiry, evidence revision, epoch, target identity, and the before-state hash,
    /// and the executor re-enumerates the exact target before acting.
    ///
    /// The window is bounded by the plan's own `expires_at`, so a plan the user prepares and then
    /// abandons can never stop monitoring; and `Running` is transient because the background
    /// executor reports its terminal result through [`Self::poll_operation_completion`].  An
    /// explicit user-initiated refresh is deliberately *not* suppressed: refreshing
    /// mid-confirmation is the user saying the ground may have moved, and invalidating the plan
    /// is the intended response.
    #[must_use]
    pub fn interaction_in_flight(&self, now: SystemTime) -> bool {
        let awaiting_confirmation = self.prepared.as_ref().is_some_and(|stored| {
            matches!(
                stored.summary.state,
                PreparedActionState::AwaitingConfirmation
            ) && now <= stored.plan.expires_at
        });
        let operation_running = self
            .operation
            .as_ref()
            .is_some_and(|operation| matches!(operation.state, OperationState::Running { .. }));
        awaiting_confirmation || operation_running
    }

    #[must_use]
    pub fn executor_call_count(&self) -> usize {
        self.test_executor
            .as_ref()
            .map_or(0, |executor| executor.calls())
    }

    pub fn executor_returns(&self, result: Result<crate::ExecutionReceipt, PortError>) {
        if let Some(executor) = &self.test_executor {
            executor.set_result(result);
        }
    }

    fn invalidate_prepared(&mut self, reason: ConfirmationInvalidationReason) {
        if let Some(stored) = self.prepared.as_mut() {
            if matches!(
                stored.summary.state,
                PreparedActionState::AwaitingConfirmation
            ) {
                stored.summary.state = PreparedActionState::Invalidated { reason };
                self.state.publish_only_change();
            }
        }
    }

    fn expire_if_needed(&mut self) {
        let now = self.clock.system_now();
        let expired = self
            .prepared
            .as_ref()
            .is_some_and(|stored| now > stored.plan.expires_at);
        if expired {
            self.invalidate_prepared(ConfirmationInvalidationReason::Expired);
        }
        self.state = crate::reduce_state(&self.state, BackendEvent::ExpirationTick, now);
    }

    fn set_operation_phase(&mut self, phase: OperationPhase) {
        if let Some(operation) = self.operation.as_mut() {
            operation.state = OperationState::Running { phase };
            self.state.publish_only_change();
        }
    }

    fn finish_operation(&mut self, outcome: OperationOutcome, finished_at: SystemTime) {
        if let Some(operation) = self.operation.as_mut() {
            operation.state = OperationState::Finished {
                outcome,
                finished_at,
            };
            self.state.publish_only_change();
        }
    }

    /// Record a user-visible command rejection. The seq is monotonic so the UI can show exactly
    /// one toast per rejection even when several snapshots carry the same notice.
    fn report_feedback(&mut self, code: FailureCode) {
        self.feedback_seq = self.feedback_seq.saturating_add(1);
        self.feedback = Some(crate::UiFeedback {
            seq: self.feedback_seq,
            code,
        });
    }
}

fn map_execution(receipt: Result<crate::ExecutionReceipt, PortError>) -> OperationOutcome {
    match receipt {
        Ok(receipt) => match receipt.outcome {
            crate::ExecutionReceiptOutcome::Applied => receipt.after_state_hash.map_or(
                OperationOutcome::OutcomeUnknown {
                    code: ErrorCode::VerificationFailed,
                },
                |after_state_hash| OperationOutcome::Applied { after_state_hash },
            ),
            crate::ExecutionReceiptOutcome::Failed { code } => OperationOutcome::Failed {
                code,
                rollback: RollbackOutcome::NotAttempted,
            },
            crate::ExecutionReceiptOutcome::OutcomeUnknown { code } => {
                OperationOutcome::OutcomeUnknown { code }
            }
        },
        Err(error) if error.code.category == ErrorCode::Timeout => {
            OperationOutcome::OutcomeUnknown {
                code: ErrorCode::Timeout,
            }
        }
        Err(error) => OperationOutcome::Failed {
            code: error.code.category,
            rollback: RollbackOutcome::NotAttempted,
        },
    }
}

fn failure(category: ErrorCode, stable: &'static str) -> FailureCode {
    FailureCode::new(
        category,
        StableCode::try_from_static(stable).expect("static stable code"),
    )
}

/// Map a rejected prepare onto a stable, localizable failure code.
fn prepare_feedback(error: PrepareError) -> FailureCode {
    match error {
        PrepareError::Busy => failure(ErrorCode::Internal, "app:busy"),
        PrepareError::UnsupportedAction => {
            failure(ErrorCode::Unsupported, "app:unsupported_action")
        }
        PrepareError::MissingTarget => failure(ErrorCode::DeviceRemoved, "app:target_absent"),
        PrepareError::Prerequisite(code) => code,
        PrepareError::Safety(_) => failure(ErrorCode::Unsupported, "app:safety_rejected"),
    }
}

/// Map a rejected confirm/cancel/dismiss onto a stable, localizable failure code.
fn confirm_feedback(error: ConfirmError) -> FailureCode {
    use ConfirmationInvalidationReason as Reason;
    match error {
        ConfirmError::Invalidated(Reason::Expired) => {
            failure(ErrorCode::EvidenceExpired, "app:plan_expired")
        }
        ConfirmError::Invalidated(Reason::SnapshotChanged | Reason::BeforeStateChanged) => {
            failure(ErrorCode::EvidenceExpired, "app:before_state_changed")
        }
        ConfirmError::Invalidated(
            Reason::EpochChanged | Reason::TargetChanged | Reason::DeviceRemoved,
        ) => failure(ErrorCode::DeviceRemoved, "app:target_absent"),
        ConfirmError::Invalidated(Reason::Superseded)
        | ConfirmError::UnknownPlan
        | ConfirmError::AlreadyConsumed
        | ConfirmError::RevalidationFailed(_)
        | ConfirmError::ExecutionFailed => failure(ErrorCode::Internal, "app:confirm_rejected"),
        ConfirmError::Busy => failure(ErrorCode::Internal, "app:busy"),
    }
}

// `ControllerRunner` is implemented in monitor.rs so monitor scheduling and command handling
// share one bounded signal path without exposing the reducer internals to UI code.
