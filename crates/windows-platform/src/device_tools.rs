//! One device-tool transaction against a verified AT session.
//!
//! This is the platform half of the tool path: open a session on the port the device inventory
//! proves belongs to the module (the same bounded, handshake-verified selection the SMS path
//! uses), write the validated line once, wait for one final response, and always give the worker
//! back before returning.
//!
//! The platform crate deliberately has no opinion about *what* an outcome means to the product —
//! it reports whether the module answered, whether bytes were written, and why waiting stopped.
//! The application layer turns that into a tool outcome.

use std::time::Duration;

use dji4g_at_protocol::{ToolParseError, ToolResponse, ToolWireRequest};
use dji4g_domain::{DeviceEpoch, ToolTransactionControl};

use crate::pnp::{DjiDevice, PlatformError};
use crate::{ActorError, AtSessionActor};

/// How long a tool session is given to release the port once its transaction has ended. A tool
/// transaction never keeps a worker behind: if this deadline passes, the port stays leased by that
/// worker and the next attempt fails honestly instead of racing it.
pub const TOOL_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// How often the waiting loop re-checks the caller's control handle. The actor's own read
/// interval is shorter, so this only bounds how quickly a cancel is noticed between reads.
const CONTROL_POLL: Duration = Duration::from_millis(50);

/// Why a transaction ended without a final response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnansweredReason {
    /// The caller cancelled (or a queued request was cancelled before it ran).
    Cancelled,
    /// The absolute deadline passed.
    Deadline,
    /// The module stopped answering or the port failed.
    Transport,
    /// The session was already retired (device removed, or a previous transaction failed).
    SessionUnavailable,
}

/// What the module actually did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExchangeOutcome {
    /// A final code arrived, including `ERROR`/`+CME ERROR`: the module answered.
    Answered(ToolResponse),
    /// The response was not one text command/response exchange. The session was retired.
    Malformed(ToolParseError),
    /// Nothing usable came back.
    Unanswered {
        /// Whether the command reached the port. A partial write counts as written.
        wrote: bool,
        reason: UnansweredReason,
    },
}

/// The result of one tool exchange, before the application classifies it.
#[derive(Clone, Debug)]
pub struct ToolExchange {
    pub outcome: ToolExchangeOutcome,
}

/// Run one tool transaction on a freshly verified session.
///
/// The session is opened per transaction, so a tool task can never inherit a stale port name, a
/// stale identity, or another task's pending input. A failure to open, or a session that cannot be
/// given back inside [`TOOL_CLOSE_TIMEOUT`], is a `PlatformError` with a stable code — the caller
/// treats it as a transport failure and never as a module verdict.
#[cfg(windows)]
pub fn tool_exchange(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    request: ToolWireRequest,
    control: ToolTransactionControl,
) -> Result<ToolExchange, PlatformError> {
    // The verification control is separate: it bounds the handshake, while `control` bounds the
    // command itself and is the handle the user's cancel reaches.
    let handshake_control = dji4g_domain::SmsTransactionControl::new(TOOL_CLOSE_TIMEOUT * 2);
    if control.is_cancelled() {
        return Ok(ToolExchange {
            outcome: ToolExchangeOutcome::Unanswered {
                wrote: false,
                reason: UnansweredReason::Cancelled,
            },
        });
    }
    let mut actor = crate::sms_transaction::open_verified(device, epoch, &handshake_control)
        .map_err(|error| map_actor_error(&error))?;

    let receiver = match actor.try_execute_tool(request, control.clone()) {
        Ok(receiver) => receiver,
        Err(error) => {
            close_session(&mut actor);
            return Err(map_actor_error(&error));
        }
    };

    // Wait for the worker without ever blocking the caller's thread past a control poll: the user
    // must be able to cancel a tool command, and the UI must keep repainting while it runs.
    let result = loop {
        if control.is_cancelled() {
            // The worker may be inside a blocking read that cannot observe the flag; cancellation
            // targets that worker, and ownership of the port stays with it until it exits.
            actor.invalidate_epoch();
            match receiver.recv_timeout(TOOL_CLOSE_TIMEOUT) {
                Ok(result) => break result,
                Err(_) => break Err(ActorError::CloseTimeout),
            }
        }
        if control.is_expired() {
            actor.invalidate_epoch();
            match receiver.recv_timeout(TOOL_CLOSE_TIMEOUT) {
                Ok(result) => break result,
                Err(_) => break Err(ActorError::CloseTimeout),
            }
        }
        match receiver.recv_timeout(CONTROL_POLL) {
            Ok(result) => break result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err(ActorError::Closed),
        }
    };

    // Hand the worker back before reporting anything: a late `OK` must never reach the next task.
    let close_result = actor.close_and_wait(TOOL_CLOSE_TIMEOUT);
    if let Err(error) = close_result {
        return Err(map_actor_error(&error));
    }

    Ok(classify_exchange(result, control.write_attempted()))
}

#[cfg(not(windows))]
pub fn tool_exchange(
    _device: &DjiDevice,
    _epoch: DeviceEpoch,
    _request: ToolWireRequest,
    _control: ToolTransactionControl,
) -> Result<ToolExchange, PlatformError> {
    Err(platform_error("device_tools:unsupported"))
}

fn classify_exchange(result: Result<ToolResponse, ActorError>, wrote: bool) -> ToolExchange {
    let outcome = match result {
        Ok(response) => ToolExchangeOutcome::Answered(response),
        Err(ActorError::Tool(error)) => ToolExchangeOutcome::Malformed(error),
        Err(ActorError::Io(std::io::ErrorKind::Interrupted)) => ToolExchangeOutcome::Unanswered {
            wrote,
            reason: UnansweredReason::Cancelled,
        },
        Err(ActorError::Io(std::io::ErrorKind::TimedOut)) => ToolExchangeOutcome::Unanswered {
            wrote,
            reason: UnansweredReason::Deadline,
        },
        Err(ActorError::CloseTimeout | ActorError::Closed | ActorError::LeaseBusy) => {
            ToolExchangeOutcome::Unanswered {
                wrote,
                reason: UnansweredReason::SessionUnavailable,
            }
        }
        Err(_) => ToolExchangeOutcome::Unanswered {
            wrote,
            reason: UnansweredReason::Transport,
        },
    };
    ToolExchange { outcome }
}

const fn platform_error(code: &'static str) -> PlatformError {
    PlatformError {
        code,
        os_code: None,
    }
}

fn close_session(actor: &mut AtSessionActor) {
    let _ = actor.close_and_wait(TOOL_CLOSE_TIMEOUT);
}

fn map_actor_error(error: &ActorError) -> PlatformError {
    platform_error(match error {
        ActorError::LeaseBusy => "device_tools:port_busy",
        ActorError::CloseTimeout => "device_tools:close_timeout",
        ActorError::OsIo { .. } => "device_tools:port_open_failed",
        ActorError::QueueFull => "device_tools:port_busy",
        ActorError::Closed => "device_tools:session_closed",
        ActorError::Io(std::io::ErrorKind::PermissionDenied) => "device_tools:permission_denied",
        ActorError::Io(std::io::ErrorKind::NotFound) => "device_tools:device_removed",
        ActorError::Io(_) => "device_tools:transport_failed",
        ActorError::Protocol(error) => match error.kind {
            dji4g_at_protocol::ProtocolErrorKind::DeviceRemoved => "device_tools:device_removed",
            dji4g_at_protocol::ProtocolErrorKind::Timeout => "device_tools:timeout",
            _ => "device_tools:verification_failed",
        },
        ActorError::FinalCode(_) => "device_tools:unexpected_final_code",
        ActorError::Tool(error) => error.code(),
    })
}

#[cfg(test)]
#[path = "device_tools_tests.rs"]
mod tests;
