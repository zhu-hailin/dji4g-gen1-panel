//! A confirmed SMS owns one serial session from handshake through final acknowledgement.
use crate::{ActorError, AtSessionActor, DjiDevice, SmsSendResult};
use dji4g_at_protocol::{
    AtCommand, AtFinalCode, AtResponse, EncodedSubmit, ProtocolError, ProtocolErrorKind,
    build_ucs2_submit,
};
use dji4g_domain::{DeviceEpoch, ErrorCode, SmsFailureDetail, SmsSendPhase, SmsTransactionControl};
use std::{
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct SmsSubmitReceipt {
    pub result: SmsSendResult,
    pub failure: Option<SmsFailureDetail>,
}

pub fn sms_send_controlled(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    recipient: &str,
    text: &str,
    control: SmsTransactionControl,
) -> SmsSubmitReceipt {
    let submit = match build_ucs2_submit(recipient, text) {
        Ok(value) => value,
        Err(_) => return failed("sms:invalid_message", &control),
    };
    if control.is_cancelled() {
        return failed("sms:timeout", &control);
    }
    #[cfg(windows)]
    {
        let mut actor = match open_verified(device, epoch, &control) {
            Ok(value) => value,
            Err(error) => return actor_failure(error, &control),
        };
        let outcome = send_in_session(&actor, submit, &control);
        let receipt = classify(outcome, &control);
        // The actor retains its lease until its worker has really released the OS handle.
        // Waiting here keeps the application busy until cleanup is complete.
        close_session(&mut actor, &control);
        receipt
    }
    #[cfg(not(windows))]
    {
        let _ = (device, epoch, submit);
        failed("sms:unsupported", &control)
    }
}

fn close_session(actor: &mut AtSessionActor, control: &SmsTransactionControl) {
    if actor.close_and_wait(Duration::from_secs(2)).is_err() {
        control.mark_cleanup_pending();
        control.cancel();
        // Keep the app's send active while an unresponsive driver still owns the transport.
        // Every wait is bounded; the controller continues to run and displays cleanup_timeout.
        while actor.close_and_wait(Duration::from_millis(100)).is_err() {}
    }
}

#[cfg(windows)]
pub(crate) fn open_verified(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    control: &SmsTransactionControl,
) -> Result<AtSessionActor, ActorError> {
    let selection =
        crate::select_at_port_verified(device.com_candidates(), &[], 2).map_err(|_| {
            ActorError::Protocol(ProtocolError {
                code: ErrorCode::VerificationFailed,
                kind: ProtocolErrorKind::WrongPortData,
            })
        })?;
    let candidates = match selection {
        crate::AtPortSelection::Classified(port) => vec![port],
        crate::AtPortSelection::HandshakeCandidates(ports) => ports,
    };
    let mut last_error = ActorError::Closed;
    for selected in candidates {
        if control.is_cancelled() {
            return Err(timeout_error());
        }
        let mut actor = AtSessionActor::open_selected(epoch, &selected)?;
        match wait(&actor, actor.try_safe_handshake(), control) {
            Ok(identity) if !identity.is_empty() => return Ok(actor),
            Ok(_) => {
                last_error = ActorError::Protocol(ProtocolError {
                    code: ErrorCode::VerificationFailed,
                    kind: ProtocolErrorKind::WrongPortData,
                })
            }
            Err(error) => last_error = error,
        }
        close_session(&mut actor, control);
    }
    Err(last_error)
}

fn timeout_error() -> ActorError {
    ActorError::Protocol(ProtocolError {
        code: ErrorCode::Timeout,
        kind: ProtocolErrorKind::Timeout,
    })
}

fn wait<T>(
    actor: &AtSessionActor,
    pending: Result<Receiver<Result<T, ActorError>>, ActorError>,
    control: &SmsTransactionControl,
) -> Result<T, ActorError> {
    let receiver = pending?;
    let deadline = (Instant::now() + Duration::from_secs(5)).min(control.deadline());
    loop {
        if control.is_cancelled() || Instant::now() >= deadline {
            actor.invalidate_epoch();
            return Err(timeout_error());
        }
        match receiver.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50)),
        ) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(ActorError::Closed),
        }
    }
}

fn execute(
    actor: &AtSessionActor,
    command: AtCommand,
    control: &SmsTransactionControl,
) -> Result<AtResponse, ActorError> {
    if control.is_cancelled() {
        return Err(timeout_error());
    }
    wait(actor, actor.try_execute(command), control)
}

fn send_in_session(
    actor: &AtSessionActor,
    submit: EncodedSubmit,
    control: &SmsTransactionControl,
) -> Result<AtResponse, ActorError> {
    let mode = execute(actor, AtCommand::SmsMessageFormat, control)?;
    if crate::sms::parse_cmgf_mode(&mode.lines) != Some(true) {
        execute(actor, AtCommand::SmsSetPduMode, control)?;
        let mode = execute(actor, AtCommand::SmsMessageFormat, control)?;
        if crate::sms::parse_cmgf_mode(&mode.lines) != Some(true) {
            return Err(ActorError::Protocol(ProtocolError {
                code: ErrorCode::VerificationFailed,
                kind: ProtocolErrorKind::UnexpectedData,
            }));
        }
    }
    control.set_phase(SmsSendPhase::Submitting);
    actor.execute_prompt_controlled(
        AtCommand::SmsSend {
            tpdu_octets: submit.tpdu_octets,
        },
        submit.expose_for_confirmed_send().as_bytes().to_vec(),
        control.clone(),
    )
}

fn failed(code: &str, control: &SmsTransactionControl) -> SmsSubmitReceipt {
    SmsSubmitReceipt {
        result: if control.submission_possible() {
            SmsSendResult::OutcomeUnknown
        } else {
            SmsSendResult::Failed
        },
        failure: Some(SmsFailureDetail::new(
            control.phase(),
            code,
            control.submission_possible(),
        )),
    }
}

fn classify(
    outcome: Result<AtResponse, ActorError>,
    control: &SmsTransactionControl,
) -> SmsSubmitReceipt {
    match outcome {
        Ok(response)
            if response.final_code == AtFinalCode::Ok
                && response
                    .lines
                    .iter()
                    .any(|line| crate::sms::is_cmgs_reference(line)) =>
        {
            SmsSubmitReceipt {
                result: SmsSendResult::Submitted,
                failure: None,
            }
        }
        Ok(_) => failed("sms:missing_reference", control),
        Err(error) => actor_failure(error, control),
    }
}

fn actor_failure(error: ActorError, control: &SmsTransactionControl) -> SmsSubmitReceipt {
    let mut receipt = failed("sms:send_failed", control);
    let detail = receipt.failure.as_mut().expect("failure detail");
    match error {
        ActorError::LeaseBusy => detail.code = "sms:port_busy".into(),
        ActorError::OsIo { raw_os_error, .. } => {
            detail.code = "sms:port_open_failed".into();
            detail.os_code = raw_os_error;
        }
        ActorError::CloseTimeout => detail.code = "sms:cleanup_timeout".into(),
        ActorError::FinalCode(AtFinalCode::CmsError(code)) => {
            detail.code = "sms:module_rejected".into();
            detail.cms_code = code.trim().parse().ok();
            receipt.result = SmsSendResult::Failed;
        }
        ActorError::FinalCode(AtFinalCode::CmeError(code)) => {
            detail.code = "sms:module_rejected".into();
            detail.cme_code = code.trim().parse().ok();
            receipt.result = SmsSendResult::Failed;
        }
        ActorError::FinalCode(AtFinalCode::Error) => {
            detail.code = "sms:module_rejected".into();
            receipt.result = SmsSendResult::Failed;
        }
        ActorError::Protocol(error) => {
            detail.code = match error.kind {
                ProtocolErrorKind::Timeout => "sms:timeout",
                ProtocolErrorKind::DeviceRemoved => "sms:device_removed",
                ProtocolErrorKind::UnexpectedData if !control.submission_possible() => {
                    "sms:pdu_confirm_failed"
                }
                _ => "sms:verification_failed",
            }
            .into()
        }
        ActorError::Io(std::io::ErrorKind::Interrupted) if control.is_cancelled() => {
            detail.code = "sms:timeout".into()
        }
        ActorError::Io(_) => detail.code = "sms:transport_failed".into(),
        ActorError::Closed => detail.code = "sms:device_removed".into(),
        ActorError::QueueFull => detail.code = "sms:port_busy".into(),
        ActorError::FinalCode(_) => detail.code = "sms:unexpected_final_code".into(),
    }
    receipt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::{SerialIo, SerialIoCancellation};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    struct PassiveCancel;
    impl SerialIoCancellation for PassiveCancel {
        fn cancel(&self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct Fixture {
        reads: VecDeque<Vec<u8>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl SerialIo for Fixture {
        fn cancellation_handle(&self) -> std::io::Result<Arc<dyn SerialIoCancellation>> {
            Ok(Arc::new(PassiveCancel))
        }
        fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
        fn read_chunk(&mut self) -> std::io::Result<Vec<u8>> {
            self.reads
                .pop_front()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::TimedOut))
        }
    }
    #[test]
    fn production_transaction_switches_confirms_and_submits_once() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let actor = AtSessionActor::spawn(
            DeviceEpoch(1),
            Box::new(Fixture {
                writes: writes.clone(),
                reads: [
                    "+CMGF: 1\r\nOK\r\n",
                    "OK\r\n",
                    "+CMGF: 0\r\nOK\r\n",
                    ">",
                    "+CMGS: 7\r\nOK\r\n",
                ]
                .into_iter()
                .map(|s| s.as_bytes().to_vec())
                .collect(),
            }),
        );
        let control = SmsTransactionControl::new(Duration::from_secs(1));
        let response = send_in_session(
            &actor,
            build_ucs2_submit("+8613800138000", "测试").unwrap(),
            &control,
        );
        assert_eq!(
            classify(response, &control).result,
            SmsSendResult::Submitted
        );
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 5);
        assert_eq!(writes[0], b"AT+CMGF?\r");
        assert_eq!(writes[1], b"AT+CMGF=0\r");
        assert_eq!(writes[2], b"AT+CMGF?\r");
        assert_eq!(
            writes
                .last()
                .unwrap()
                .iter()
                .filter(|byte| **byte == 0x1a)
                .count(),
            1
        );
    }
    #[test]
    fn unconfirmed_mode_never_writes_cmgs_or_body() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let actor = AtSessionActor::spawn(
            DeviceEpoch(1),
            Box::new(Fixture {
                writes: writes.clone(),
                reads: ["+CMGF: 1\r\nOK\r\n", "OK\r\n", "+CMGF: 1\r\nOK\r\n"]
                    .into_iter()
                    .map(|s| s.as_bytes().to_vec())
                    .collect(),
            }),
        );
        let control = SmsTransactionControl::new(Duration::from_secs(1));
        let response = send_in_session(
            &actor,
            build_ucs2_submit("+8613800138000", "测试").unwrap(),
            &control,
        );
        let receipt = classify(response, &control);
        assert_eq!(receipt.result, SmsSendResult::Failed);
        assert_eq!(receipt.failure.unwrap().code, "sms:pdu_confirm_failed");
        assert!(!control.submission_possible());
        assert_eq!(writes.lock().unwrap().len(), 3);
    }
    #[test]
    fn cms_code_survives_without_message_content() {
        let control = SmsTransactionControl::new(Duration::from_secs(1));
        control.mark_submission_possible();
        control.set_phase(SmsSendPhase::WaitingForResult);
        let receipt = actor_failure(
            ActorError::FinalCode(AtFinalCode::CmsError("500".into())),
            &control,
        );
        assert_eq!(receipt.result, SmsSendResult::Failed);
        let detail = receipt.failure.unwrap();
        assert_eq!(detail.cms_code, Some(500));
        assert_eq!(detail.stage, SmsSendPhase::WaitingForResult);
    }
    #[test]
    fn timeout_before_body_is_definite_and_after_body_unknown() {
        let control = SmsTransactionControl::new(Duration::from_secs(1));
        assert_eq!(
            actor_failure(timeout_error(), &control).result,
            SmsSendResult::Failed
        );
        control.mark_submission_possible();
        assert_eq!(
            actor_failure(timeout_error(), &control).result,
            SmsSendResult::OutcomeUnknown
        );
    }
    #[test]
    fn os_error_is_preserved_and_is_not_assumed_to_be_busy() {
        let receipt = actor_failure(
            ActorError::OsIo {
                kind: std::io::ErrorKind::PermissionDenied,
                raw_os_error: Some(5),
            },
            &SmsTransactionControl::new(Duration::from_secs(1)),
        );
        let detail = receipt.failure.unwrap();
        assert_eq!(detail.code, "sms:port_open_failed");
        assert_eq!(detail.os_code, Some(5));
    }
}
