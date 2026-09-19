//! SMS command layer for the first-generation DJI module (research document §6.1–§6.3).
//!
//! The inbox, storage capacity and single-message deletion are covered here. Reading
//! (`CMGL`/`CMGR`) may itself mark messages as read, so every transaction here is a single
//! `RetryPolicy::Never` attempt and a failure is surfaced instead of retried.
//!
//! Sending is a single confirmed `CMGS` transaction: PDU-mode preflight, PDU construction, then
//! one prompt transaction (`AtCommand::SmsSend`, wire `AT+CMGS=<tpdu_octets>`) that waits for `>`
//! before writing the PDU hexadecimal body plus the Ctrl-Z terminator and never resends it. The
//! AT sequences are based on the research document and the Quectel command reference;
//! 实机未验证 (not validated on real hardware).
//!
//! Storage holder resolution: the standard PDU-mode `+CMGL`/`+CMGR` headers carry no storage
//! holder. `sms_list` therefore reports the preferred holder from the `+CPMS?` first group,
//! and `sms_read` resolves it best-effort with the same read-only query so its record can be
//! paired with a listing. A failed holder query never fails the read itself.

use std::{sync::mpsc::RecvTimeoutError, time::Duration};

use dji4g_at_protocol::{
    AtCommand, AtFinalCode, AtResponse, DecodedSms, EncodedSubmit, ProtocolErrorKind,
    build_ucs2_submit, decode_deliver_pdu,
};
use dji4g_domain::DeviceEpoch;

use crate::{ActorError, AtSessionActor, DjiDevice, PlatformError};

/// Bound for one queued AT transaction and for the safe handshake.
const SMS_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// One stored message together with the module-reported storage metadata.
#[derive(Clone, Debug)]
pub struct SmsRecord {
    pub index: u32,
    pub storage: String,
    /// Read/status class 0..=4 per 3GPP TS 27.005 PDU mode.
    pub stat: u8,
    pub decoded: DecodedSms,
}

/// One inbox listing: decoded records plus the `+CPMS?` first-group capacity.
#[derive(Clone, Debug, Default)]
pub struct SmsListing {
    pub records: Vec<SmsRecord>,
    /// `(used, total)` slots reported by `+CPMS?`; `None` when that line is absent/malformed.
    pub capacity: Option<(u32, u32)>,
}

/// Terminal result of one confirmed send attempt (research document §6.3).
///
/// `Submitted` means the module accepted the PDU (`+CMGS` message reference plus final `OK`); it
/// never means the peer received the message. `OutcomeUnknown` means the transaction ended
/// without a deterministic result, so the PDU may already have left the module: a send is
/// attempted at most once and is never retried automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmsSendResult {
    Submitted,
    Failed,
    OutcomeUnknown,
}

/// Query the current message format (`AT+CMGF?`).
///
/// Returns `Some(true)` for PDU mode, `Some(false)` for text mode, and `None` when the response
/// cannot be confirmed. A protocol/transport failure is an error, never a guess.
pub fn sms_query_pdu_mode(
    device: &DjiDevice,
    epoch: DeviceEpoch,
) -> Result<Option<bool>, PlatformError> {
    with_session(device, epoch, query_pdu_mode)
}

/// Switch the module to PDU mode (`AT+CMGF=0`; per 3GPP TS 27.005 §3.2.2, 0 is PDU and 1 is
/// text). Session setting; user consent is a UI concern.
pub fn sms_set_pdu_mode(device: &DjiDevice, epoch: DeviceEpoch) -> Result<(), PlatformError> {
    with_session(device, epoch, set_pdu_mode)
}

/// List stored messages in PDU mode (`AT+CMGL=4`).
///
/// Requires the module to already be in PDU mode; otherwise the call fails closed with
/// `sms:pdu_mode_required`. Records whose PDU fails to decode are skipped individually without
/// failing the whole listing.
pub fn sms_list(device: &DjiDevice, epoch: DeviceEpoch) -> Result<SmsListing, PlatformError> {
    with_session(device, epoch, list)
}

/// Read one stored message (`AT+CMGR=<index>`). Reading may mark it as read (research §6.2).
pub fn sms_read(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    index: u32,
) -> Result<SmsRecord, PlatformError> {
    with_session(device, epoch, |actor| read(actor, index))
}

/// Delete one stored message (`AT+CMGD=<index>`). The final `OK` is required for success; a
/// failed delete is never retried.
pub fn sms_delete(device: &DjiDevice, epoch: DeviceEpoch, index: u32) -> Result<(), PlatformError> {
    with_session(device, epoch, |actor| delete(actor, index))
}

/// Send one UCS-2 SMS to an explicit international recipient.
///
/// The UI owns the single per-send confirmation and only calls this for an exact confirmed
/// recipient and body. Preconditions fail closed: a module that is not confirmed in PDU mode
/// yields `sms:pdu_mode_required` (the mode switch itself is a UI consent concern), and a message
/// the PDU builder rejects yields `sms:invalid_message` (recipient, body, size and BMP are all
/// validated by [`build_ucs2_submit`]). The submit itself is one exclusive prompt transaction
/// that is never retried. 实机未验证.
pub fn sms_send(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    recipient: &str,
    text: &str,
) -> Result<SmsSendResult, PlatformError> {
    with_session(device, epoch, |actor| send(actor, recipient, text))
}

fn with_session<T>(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    run: impl FnOnce(&AtSessionActor) -> Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    let actor = open_session(device, epoch)?;
    run(&actor)
}

#[cfg(windows)]
fn open_session(device: &DjiDevice, epoch: DeviceEpoch) -> Result<AtSessionActor, PlatformError> {
    // Use the same bounded, identity-verified selection as sending and monitoring.
    // Multiple interface candidates must be verified rather than guessed or rejected outright.
    let control = dji4g_domain::SmsTransactionControl::new(Duration::from_secs(10));
    crate::sms_transaction::open_verified(device, epoch, &control)
        .map_err(|error| map_actor_error(&error))
}

#[cfg(not(windows))]
fn open_session(_device: &DjiDevice, _epoch: DeviceEpoch) -> Result<AtSessionActor, PlatformError> {
    Err(platform_error("sms:unsupported"))
}

#[cfg(test)]
fn handshake(actor: &AtSessionActor) -> Result<Vec<String>, PlatformError> {
    let receiver = actor
        .try_safe_handshake()
        .map_err(|error| map_actor_error(&error))?;
    match receiver.recv_timeout(SMS_OPERATION_TIMEOUT) {
        Ok(result) => result.map_err(|error| map_actor_error(&error)),
        Err(RecvTimeoutError::Timeout) => {
            actor.invalidate_epoch();
            Err(platform_error("sms:timeout"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(platform_error("sms:device_removed")),
    }
}

fn execute_timed(actor: &AtSessionActor, command: AtCommand) -> Result<AtResponse, PlatformError> {
    let receiver = actor
        .try_execute(command)
        .map_err(|error| map_actor_error(&error))?;
    match receiver.recv_timeout(SMS_OPERATION_TIMEOUT) {
        Ok(result) => result.map_err(|error| map_actor_error(&error)),
        Err(RecvTimeoutError::Timeout) => {
            actor.invalidate_epoch();
            Err(platform_error("sms:timeout"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(platform_error("sms:device_removed")),
    }
}

fn query_pdu_mode(actor: &AtSessionActor) -> Result<Option<bool>, PlatformError> {
    let response = execute_timed(actor, AtCommand::SmsMessageFormat)?;
    Ok(parse_cmgf_mode(&response.lines))
}

fn set_pdu_mode(actor: &AtSessionActor) -> Result<(), PlatformError> {
    let response = execute_timed(actor, AtCommand::SmsSetPduMode)?;
    ensure_final_ok(&response)
}

fn list(actor: &AtSessionActor) -> Result<SmsListing, PlatformError> {
    if query_pdu_mode(actor)? != Some(true) {
        return Err(platform_error("sms:pdu_mode_required"));
    }
    let cpms = execute_timed(actor, AtCommand::SmsStorageQuery)?;
    let (storage, capacity) = parse_cpms(&cpms.lines);
    let response = execute_timed(actor, AtCommand::SmsList)?;
    Ok(SmsListing {
        records: pair_cmgl_records(&response.lines, &storage),
        capacity,
    })
}

fn read(actor: &AtSessionActor, index: u32) -> Result<SmsRecord, PlatformError> {
    let storage = storage_holder(actor).unwrap_or_default();
    let response = execute_timed(actor, AtCommand::SmsRead { index })?;
    parse_cmgr_record(&response.lines, index, &storage)
}

fn delete(actor: &AtSessionActor, index: u32) -> Result<(), PlatformError> {
    let response = execute_timed(actor, AtCommand::SmsDelete { index })?;
    ensure_final_ok(&response)
}

/// Resolve the send preconditions without any write beyond the read-only mode query: confirmed
/// PDU mode, then one PDU the codec accepts.
fn prepare_submit(
    actor: &AtSessionActor,
    recipient: &str,
    text: &str,
) -> Result<EncodedSubmit, PlatformError> {
    let submit =
        build_ucs2_submit(recipient, text).map_err(|_| platform_error("sms:invalid_message"))?;
    if query_pdu_mode(actor)? != Some(true) {
        set_pdu_mode(actor)?;
        if query_pdu_mode(actor)? != Some(true) {
            return Err(platform_error("sms:pdu_confirm_failed"));
        }
    }
    Ok(submit)
}

fn send(
    actor: &AtSessionActor,
    recipient: &str,
    text: &str,
) -> Result<SmsSendResult, PlatformError> {
    let submit = prepare_submit(actor, recipient, text)?;
    let body = submit.expose_for_confirmed_send().as_bytes().to_vec();
    let Some(command) = send_command(submit.tpdu_octets) else {
        return Err(platform_error("sms:send_unavailable"));
    };
    classify_send_outcome(actor.execute_prompt(command, body))
}

/// The typed submission command: `AT+CMGS=<tpdu_octets>`, whose PDU body travels on the prompt
/// path (research §6.3). The command is modeled by `dji4g-at-protocol` now, so this is a direct
/// typed construction — no raw AT string ever reaches the actor.
fn send_command(tpdu_octets: usize) -> Option<AtCommand> {
    Some(AtCommand::SmsSend { tpdu_octets })
}

/// Map one completed prompt transaction onto the terminal send outcome (research §6.3).
///
/// Only a `+CMGS` message reference followed by the final `OK` proves submission; it still never
/// proves delivery. `ERROR` and `+CMS ERROR` prove the module rejected the submit. Timeouts,
/// removal and write failures leave the message reference unproven (`OutcomeUnknown`), and that
/// outcome is never retried.
fn classify_send_outcome(
    outcome: Result<AtResponse, ActorError>,
) -> Result<SmsSendResult, PlatformError> {
    match outcome {
        Ok(response) if response.lines.iter().any(|line| is_cmgs_reference(line)) => {
            Ok(SmsSendResult::Submitted)
        }
        // A final OK without the message reference proves neither submission nor its failure.
        Ok(_) => Ok(SmsSendResult::OutcomeUnknown),
        Err(ActorError::FinalCode(AtFinalCode::Error | AtFinalCode::CmsError(_))) => {
            Ok(SmsSendResult::Failed)
        }
        // CME/NO CARRIER and any future final code cannot be interpreted as a definite rejection
        // of this submit.
        Err(ActorError::FinalCode(_)) => Err(platform_error("sms:send_failed")),
        Err(ActorError::Protocol(error))
            if matches!(
                error.kind,
                ProtocolErrorKind::Timeout | ProtocolErrorKind::DeviceRemoved
            ) =>
        {
            Ok(SmsSendResult::OutcomeUnknown)
        }
        // Bad line data or oversized responses after the body write: the PDU may still have been
        // accepted, but the response cannot be interpreted.
        Err(ActorError::Protocol(_)) => Err(platform_error("sms:send_failed")),
        // A write failure (the body write included) leaves the submit state unproven.
        Err(ActorError::Io(_) | ActorError::OsIo { .. } | ActorError::CloseTimeout) => {
            Ok(SmsSendResult::OutcomeUnknown)
        }
        // The transaction never started, so this is not a module verdict.
        Err(ActorError::QueueFull | ActorError::Closed | ActorError::LeaseBusy) => {
            Err(platform_error("sms:send_failed"))
        }
        // The SMS path never issues a tool transaction; the arm keeps the mapping total.
        Err(ActorError::Tool(_)) => Err(platform_error("sms:send_failed")),
    }
}

pub(crate) fn is_cmgs_reference(line: &str) -> bool {
    line.trim()
        .strip_prefix("+CMGS:")
        .is_some_and(|value| value.trim().parse::<u32>().is_ok())
}

pub(crate) fn parse_cmgf_mode(lines: &[String]) -> Option<bool> {
    lines.iter().find_map(|line| {
        let value = line.strip_prefix("+CMGF:")?.trim().parse::<u8>().ok()?;
        match value {
            // 3GPP TS 27.005 §3.2.2: 0 is PDU mode, 1 is text mode.
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    })
}

fn parse_cpms(lines: &[String]) -> (String, Option<(u32, u32)>) {
    let Some(payload) = lines
        .iter()
        .find_map(|line| line.strip_prefix("+CPMS:").map(str::trim))
    else {
        return (String::new(), None);
    };
    let fields: Vec<&str> = payload.split(',').map(str::trim).collect();
    let storage = fields
        .first()
        .and_then(|value| unquote(value))
        .unwrap_or_default()
        .to_owned();
    let capacity = match (
        fields.get(1).and_then(|value| value.parse::<u32>().ok()),
        fields.get(2).and_then(|value| value.parse::<u32>().ok()),
    ) {
        (Some(used), Some(total)) => Some((used, total)),
        _ => None,
    };
    (storage, capacity)
}

fn unquote(value: &str) -> Option<&str> {
    value
        .strip_prefix('"')?
        .strip_suffix('"')
        .filter(|inner| !inner.is_empty())
}

fn parse_cmgl_header(line: &str) -> Option<(u32, u8)> {
    let payload = line.strip_prefix("+CMGL:")?.trim();
    let mut fields = payload.split(',').map(str::trim);
    let index = fields.next()?.parse::<u32>().ok()?;
    let stat = fields.next()?.parse::<u8>().ok()?;
    (stat <= 4).then_some((index, stat))
}

fn parse_cmgr_header(line: &str) -> Option<u8> {
    let payload = line.strip_prefix("+CMGR:")?.trim();
    let stat = payload.split(',').next()?.trim().parse::<u8>().ok()?;
    (stat <= 4).then_some(stat)
}

fn pair_cmgl_records(lines: &[String], storage: &str) -> Vec<SmsRecord> {
    let mut records = Vec::new();
    let mut lines = lines.iter();
    while let Some(line) = lines.next() {
        let Some((index, stat)) = parse_cmgl_header(line) else {
            continue;
        };
        let Some(pdu) = lines.next() else {
            break;
        };
        let Ok(decoded) = decode_deliver_pdu(pdu.trim()) else {
            continue;
        };
        records.push(SmsRecord {
            index,
            storage: storage.to_owned(),
            stat,
            decoded,
        });
    }
    records
}

fn parse_cmgr_record(
    lines: &[String],
    index: u32,
    storage: &str,
) -> Result<SmsRecord, PlatformError> {
    let mut lines = lines.iter();
    while let Some(line) = lines.next() {
        let Some(stat) = parse_cmgr_header(line) else {
            continue;
        };
        let Some(pdu) = lines.next() else {
            return Err(platform_error("sms:verification_failed"));
        };
        let decoded = decode_deliver_pdu(pdu.trim())
            .map_err(|_| platform_error("sms:verification_failed"))?;
        return Ok(SmsRecord {
            index,
            storage: storage.to_owned(),
            stat,
            decoded,
        });
    }
    Err(platform_error("sms:verification_failed"))
}

fn storage_holder(actor: &AtSessionActor) -> Option<String> {
    let response = execute_timed(actor, AtCommand::SmsStorageQuery).ok()?;
    let (storage, _) = parse_cpms(&response.lines);
    (!storage.is_empty()).then_some(storage)
}

fn ensure_final_ok(response: &AtResponse) -> Result<(), PlatformError> {
    if response.final_code == AtFinalCode::Ok {
        Ok(())
    } else {
        Err(platform_error("sms:verification_failed"))
    }
}

const fn platform_error(code: &'static str) -> PlatformError {
    PlatformError {
        code,
        os_code: None,
    }
}

fn map_actor_error(error: &ActorError) -> PlatformError {
    let code = match error.protocol_kind() {
        Some(ProtocolErrorKind::Timeout) => "sms:timeout",
        Some(ProtocolErrorKind::DeviceRemoved) => "sms:device_removed",
        Some(
            ProtocolErrorKind::WrongPortData
            | ProtocolErrorKind::LineTooLong
            | ProtocolErrorKind::ResponseTooLarge
            | ProtocolErrorKind::UnexpectedData,
        ) => "sms:verification_failed",
        None => match error {
            ActorError::LeaseBusy => "sms:port_busy",
            ActorError::CloseTimeout => "sms:cleanup_timeout",
            ActorError::OsIo { .. } => "sms:port_open_failed",
            ActorError::Io(std::io::ErrorKind::PermissionDenied) => "sms:permission_denied",
            ActorError::Io(std::io::ErrorKind::NotFound) => "sms:device_removed",
            ActorError::FinalCode(AtFinalCode::Ok) => "sms:internal",
            ActorError::FinalCode(_) => "sms:verification_failed",
            ActorError::Protocol(_) => "sms:verification_failed",
            // SMS never issues a tool transaction; the arm keeps the mapping total.
            ActorError::Tool(_) => "sms:internal",
            ActorError::QueueFull | ActorError::Closed | ActorError::Io(_) => "sms:internal",
        },
    };
    let mut mapped = platform_error(code);
    if let ActorError::OsIo { raw_os_error, .. } = error {
        mapped.os_code = raw_os_error.map(|value| value as u32);
    }
    mapped
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io,
        sync::{Arc, Mutex},
    };

    use dji4g_at_protocol::ProtocolError;
    use dji4g_domain::{DeviceEpoch, ErrorCode};

    use super::*;
    use crate::serial::{SerialIo, SerialIoCancellation};

    // SMSC 00, DELIVER|MMS, sender +12025550123, DCS 00, SCTS 24-01-02 03:04:05 +32, "hello".
    const GSM7_HELLO: &str = "00040B912120550521F300004210203040502305E8329BFD06";
    // SMSC 00, sender +8613800138000, DCS 08 (UCS-2), UDL 02, body U+4E2D.
    const UCS2_ZHONG: &str = "00040D91683108108300F0000842102030405023024E2D";
    const CPMS: &str = "+CPMS: \"SM\",3,20,\"SM\",3,20,\"SM\",3,20\r\nOK\r\n";

    struct PassiveCancellation;

    impl SerialIoCancellation for PassiveCancellation {
        fn cancel(&self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeState {
        writes: Vec<Vec<u8>>,
        write_attempts: usize,
        fail_write_at: Option<usize>,
        reads: VecDeque<io::Result<Vec<u8>>>,
        quiet_when_empty: bool,
    }

    struct FakeSerial(Arc<Mutex<FakeState>>);

    impl SerialIo for FakeSerial {
        fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
            Ok(Arc::new(PassiveCancellation))
        }

        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            let mut state = self.0.lock().unwrap();
            let attempt = state.write_attempts;
            state.write_attempts += 1;
            if state.fail_write_at == Some(attempt) {
                return Err(io::Error::other("test write failure"));
            }
            state.writes.push(bytes.to_vec());
            Ok(())
        }

        fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
            if let Some(result) = self.0.lock().unwrap().reads.pop_front() {
                return result;
            }
            assert!(
                self.0.lock().unwrap().quiet_when_empty,
                "FakeSerial has no queued read"
            );
            Err(io::Error::new(io::ErrorKind::TimedOut, "quiet"))
        }
    }

    fn actor_with_reads<'a>(
        reads: impl IntoIterator<Item = &'a str>,
    ) -> (AtSessionActor, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            reads: reads
                .into_iter()
                .map(|text| Ok(text.as_bytes().to_vec()))
                .collect(),
            ..FakeState::default()
        }));
        let actor = AtSessionActor::spawn(DeviceEpoch(7), Box::new(FakeSerial(Arc::clone(&state))));
        (actor, state)
    }

    fn quiet_actor() -> (AtSessionActor, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            quiet_when_empty: true,
            ..FakeState::default()
        }));
        let actor = AtSessionActor::spawn(DeviceEpoch(7), Box::new(FakeSerial(Arc::clone(&state))));
        (actor, state)
    }

    /// Serve the queued reads first, then behave like a device that went silent mid-transaction.
    fn actor_with_reads_then_quiet<'a>(
        reads: impl IntoIterator<Item = &'a str>,
    ) -> (AtSessionActor, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            reads: reads
                .into_iter()
                .map(|text| Ok(text.as_bytes().to_vec()))
                .collect(),
            quiet_when_empty: true,
            ..FakeState::default()
        }));
        let actor = AtSessionActor::spawn(DeviceEpoch(7), Box::new(FakeSerial(Arc::clone(&state))));
        (actor, state)
    }

    #[test]
    fn session_handshake_sends_only_at_then_ati() {
        let (actor, state) = actor_with_reads(["OK\r\n", "Quectel EC200A\r\nOK\r\n"]);

        let identity = handshake(&actor).unwrap();

        assert_eq!(identity, ["Quectel EC200A"]);
        assert_eq!(
            state.lock().unwrap().writes,
            vec![b"AT\r".to_vec(), b"ATI\r".to_vec()]
        );
    }

    #[test]
    fn pdu_mode_query_parses_both_modes() {
        let (actor, state) = actor_with_reads(["+CMGF: 0\r\nOK\r\n"]);
        assert_eq!(query_pdu_mode(&actor).unwrap(), Some(true));
        assert_eq!(state.lock().unwrap().writes, vec![b"AT+CMGF?\r".to_vec()]);

        let (actor, _) = actor_with_reads(["+CMGF: 1\r\nOK\r\n"]);
        assert_eq!(query_pdu_mode(&actor).unwrap(), Some(false));
    }

    #[test]
    fn set_pdu_mode_requires_the_final_ok_and_never_retries() {
        let (actor, state) = actor_with_reads(["OK\r\n"]);
        set_pdu_mode(&actor).unwrap();
        assert_eq!(state.lock().unwrap().writes, vec![b"AT+CMGF=0\r".to_vec()]);

        let (actor, state) = actor_with_reads(["ERROR\r\n"]);
        assert_eq!(
            set_pdu_mode(&actor).unwrap_err().code,
            "sms:verification_failed"
        );
        assert_eq!(state.lock().unwrap().write_attempts, 1);
    }

    #[test]
    fn list_requires_pdu_mode_and_sends_nothing_else() {
        let (actor, state) = actor_with_reads(["+CMGF: 1\r\nOK\r\n"]);

        let error = list(&actor).unwrap_err();

        assert_eq!(error.code, "sms:pdu_mode_required");
        assert_eq!(state.lock().unwrap().writes, vec![b"AT+CMGF?\r".to_vec()]);
    }

    #[test]
    fn list_reads_capacity_and_decodes_records_skipping_bad_pdus() {
        let cmgl = format!("+CMGL: 1,1,,24\r\n{GSM7_HELLO}\r\n+CMGL: 2,0,,4\r\n0000\r\nOK\r\n");
        let (actor, state) = actor_with_reads(["+CMGF: 0\r\nOK\r\n", CPMS, cmgl.as_str()]);

        let listing = list(&actor).unwrap();

        assert_eq!(listing.capacity, Some((3, 20)));
        assert_eq!(listing.records.len(), 1);
        assert_eq!(listing.records[0].index, 1);
        assert_eq!(listing.records[0].stat, 1);
        assert_eq!(listing.records[0].storage, "SM");
        assert_eq!(listing.records[0].decoded.body, "hello");
        assert_eq!(
            state.lock().unwrap().writes,
            vec![
                b"AT+CMGF?\r".to_vec(),
                b"AT+CPMS?\r".to_vec(),
                b"AT+CMGL=4\r".to_vec(),
            ]
        );
    }

    #[test]
    fn read_resolves_storage_and_decodes_the_single_record() {
        let cmgr = format!("+CMGR: 1,,24\r\n{GSM7_HELLO}\r\nOK\r\n");
        let (actor, state) = actor_with_reads([CPMS, cmgr.as_str()]);

        let record = read(&actor, 1).unwrap();

        assert_eq!(record.index, 1);
        assert_eq!(record.stat, 1);
        assert_eq!(record.storage, "SM");
        assert_eq!(record.decoded.body, "hello");
        assert_eq!(
            state.lock().unwrap().writes,
            vec![b"AT+CPMS?\r".to_vec(), b"AT+CMGR=1\r".to_vec()]
        );
    }

    #[test]
    fn delete_requires_the_final_ok_and_never_retries() {
        let (actor, state) = actor_with_reads(["ERROR\r\n"]);

        let error = delete(&actor, 5).unwrap_err();

        assert_eq!(error.code, "sms:verification_failed");
        assert_eq!(state.lock().unwrap().write_attempts, 1);
        assert_eq!(state.lock().unwrap().writes, vec![b"AT+CMGD=5\r".to_vec()]);

        let (actor, _) = actor_with_reads(["OK\r\n"]);
        delete(&actor, 5).unwrap();
    }

    #[test]
    fn silent_device_is_mapped_to_timeout_without_resending() {
        let (actor, state) = quiet_actor();

        let error = delete(&actor, 5).unwrap_err();

        assert_eq!(error.code, "sms:timeout");
        assert_eq!(state.lock().unwrap().write_attempts, 1);
    }

    #[test]
    fn removed_device_is_mapped_to_device_removed() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        state
            .lock()
            .unwrap()
            .reads
            .push_back(Err(io::Error::new(io::ErrorKind::NotConnected, "removed")));
        let actor = AtSessionActor::spawn(DeviceEpoch(7), Box::new(FakeSerial(Arc::clone(&state))));

        let error = query_pdu_mode(&actor).unwrap_err();

        assert_eq!(error.code, "sms:device_removed");
    }

    #[test]
    fn cmgf_parsing_accepts_only_zero_and_one() {
        assert_eq!(parse_cmgf_mode(&["+CMGF: 1".to_owned()]), Some(false));
        assert_eq!(parse_cmgf_mode(&["+CMGF: 0".to_owned()]), Some(true));
        assert_eq!(parse_cmgf_mode(&["+CMGF:".to_owned()]), None);
        assert_eq!(parse_cmgf_mode(&["OK".to_owned()]), None);
        assert_eq!(parse_cmgf_mode(&[]), None);
    }

    #[test]
    fn cpms_parsing_takes_the_first_group_and_survives_bad_capacity() {
        let (storage, capacity) =
            parse_cpms(&["+CPMS: \"SM\",3,20,\"ME\",1,10,\"MT\",0,5".to_owned()]);
        assert_eq!(storage, "SM");
        assert_eq!(capacity, Some((3, 20)));

        let (storage, capacity) = parse_cpms(&["+CPMS: \"ME\",x,10".to_owned()]);
        assert_eq!(storage, "ME");
        assert_eq!(capacity, None);

        assert_eq!(parse_cpms(&[]), (String::new(), None));
    }

    #[test]
    fn cmgl_pairing_keeps_headers_aligned_when_a_pdu_is_skipped() {
        let lines = [
            "+CMGL: 1,1,,24".to_owned(),
            GSM7_HELLO.to_owned(),
            "+CMGL: 2,0,,4".to_owned(),
            "0000".to_owned(),
            "+CMGL: 3,2,,22".to_owned(),
            UCS2_ZHONG.to_owned(),
        ];

        let records = pair_cmgl_records(&lines, "SM");

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].index, 1);
        assert_eq!(records[0].storage, "SM");
        assert_eq!(records[0].decoded.body, "hello");
        assert_eq!(records[1].index, 3);
        assert_eq!(records[1].stat, 2);
        assert_eq!(records[1].decoded.body, "中");
    }

    #[test]
    fn cmgr_parsing_rejects_missing_or_undecodable_payloads() {
        let undecodable = ["+CMGR: 1,,24".to_owned(), "0000".to_owned()];
        assert_eq!(
            parse_cmgr_record(&undecodable, 1, "SM").unwrap_err().code,
            "sms:verification_failed"
        );
        assert_eq!(
            parse_cmgr_record(&[], 1, "SM").unwrap_err().code,
            "sms:verification_failed"
        );
    }

    // --- Send preflight (research §6.3) ---

    #[test]
    fn confirmed_send_switches_mode_inside_the_same_actor() {
        let (actor, state) = actor_with_reads([
            "+CMGF: 1\r\nOK\r\n",
            "OK\r\n",
            "+CMGF: 0\r\nOK\r\n",
            "\r\n>",
            "\r\n+CMGS: 12\r\nOK\r\n",
        ]);
        assert_eq!(
            send(&actor, "+8613800138000", "中").unwrap(),
            SmsSendResult::Submitted
        );
        let writes = &state.lock().unwrap().writes;
        assert_eq!(
            &writes[..3],
            &[
                b"AT+CMGF?\r".to_vec(),
                b"AT+CMGF=0\r".to_vec(),
                b"AT+CMGF?\r".to_vec()
            ]
        );
    }

    #[test]
    fn send_refuses_when_mode_switch_is_not_confirmed() {
        let (actor, state) =
            actor_with_reads(["+CMGF: 1\r\nOK\r\n", "OK\r\n", "+CMGF: 1\r\nOK\r\n"]);

        let error = send(&actor, "+12025550123", "中").unwrap_err();

        assert_eq!(error.code, "sms:pdu_confirm_failed");
        assert_eq!(
            state.lock().unwrap().writes,
            vec![
                b"AT+CMGF?\r".to_vec(),
                b"AT+CMGF=0\r".to_vec(),
                b"AT+CMGF?\r".to_vec()
            ]
        );
    }

    #[test]
    fn send_rejects_unencodable_messages_before_any_submit_writes() {
        let cases: &[(&str, &str)] = &[
            ("12025550123", "中"),  // not an explicit international recipient
            ("+12025550123", ""),   // empty body
            ("+12025550123", "😀"), // outside the BMP
        ];
        for (recipient, text) in cases {
            let (actor, state) = actor_with_reads(["+CMGF: 0\r\nOK\r\n"]);

            let error = send(&actor, recipient, text).unwrap_err();

            assert_eq!(error.code, "sms:invalid_message", "{recipient:?}");
            assert!(state.lock().unwrap().writes.is_empty());
        }

        let overlong = "中".repeat(71);
        let (actor, _) = actor_with_reads(["+CMGF: 0\r\nOK\r\n"]);
        assert_eq!(
            send(&actor, "+12025550123", &overlong).unwrap_err().code,
            "sms:invalid_message"
        );
    }

    #[test]
    fn send_uses_the_typed_cmgs_prompt_transaction() {
        let (actor, state) =
            actor_with_reads(["+CMGF: 0\r\nOK\r\n", "\r\n>", "\r\n+CMGS: 12\r\nOK\r\n"]);

        let outcome = send(&actor, "+12025550123", "中");

        assert_eq!(outcome.unwrap(), SmsSendResult::Submitted);
        let submit = build_ucs2_submit("+12025550123", "中").unwrap();
        let mut expected_body = submit.expose_for_confirmed_send().as_bytes().to_vec();
        expected_body.push(0x1A);
        assert_eq!(
            state.lock().unwrap().writes,
            vec![
                b"AT+CMGF?\r".to_vec(),
                format!("AT+CMGS={}\r", submit.tpdu_octets).into_bytes(),
                expected_body,
            ]
        );
    }

    // --- Send transaction outcomes (research §6.3) ---

    fn ok_response(lines: &[&str]) -> AtResponse {
        AtResponse {
            epoch: DeviceEpoch(7),
            command: AtCommand::Identity,
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            final_code: AtFinalCode::Ok,
        }
    }

    fn protocol_error(kind: ProtocolErrorKind) -> ActorError {
        ActorError::Protocol(ProtocolError {
            code: ErrorCode::VerificationFailed,
            kind,
        })
    }

    #[test]
    fn submitted_needs_the_cmgs_reference_with_the_final_ok() {
        assert_eq!(
            classify_send_outcome(Ok(ok_response(&["+CMGS: 12"]))).unwrap(),
            SmsSendResult::Submitted
        );
        // A final OK without the message reference proves neither submission nor its failure.
        assert_eq!(
            classify_send_outcome(Ok(ok_response(&[]))).unwrap(),
            SmsSendResult::OutcomeUnknown
        );
        // A malformed reference is not a reference.
        assert_eq!(
            classify_send_outcome(Ok(ok_response(&["+CMGS: n/a"]))).unwrap(),
            SmsSendResult::OutcomeUnknown
        );
    }

    #[test]
    fn definitive_rejections_are_failed() {
        for error in [
            ActorError::FinalCode(AtFinalCode::Error),
            ActorError::FinalCode(AtFinalCode::CmsError("500".to_owned())),
        ] {
            assert_eq!(
                classify_send_outcome(Err(error)).unwrap(),
                SmsSendResult::Failed
            );
        }
    }

    #[test]
    fn uninterpretable_failures_are_send_failed() {
        for error in [
            ActorError::FinalCode(AtFinalCode::CmeError("3".to_owned())),
            ActorError::FinalCode(AtFinalCode::NoCarrier),
            ActorError::QueueFull,
            ActorError::Closed,
            protocol_error(ProtocolErrorKind::WrongPortData),
            protocol_error(ProtocolErrorKind::LineTooLong),
            protocol_error(ProtocolErrorKind::UnexpectedData),
        ] {
            assert_eq!(
                classify_send_outcome(Err(error)).unwrap_err().code,
                "sms:send_failed"
            );
        }
    }

    #[test]
    fn transport_loss_and_write_failures_leave_the_outcome_unknown() {
        for error in [
            protocol_error(ProtocolErrorKind::Timeout),
            protocol_error(ProtocolErrorKind::DeviceRemoved),
            ActorError::Io(io::ErrorKind::Other),
            ActorError::Io(io::ErrorKind::BrokenPipe),
        ] {
            assert_eq!(
                classify_send_outcome(Err(error)).unwrap(),
                SmsSendResult::OutcomeUnknown
            );
        }
    }

    #[test]
    fn prompt_transaction_with_cmgs_reference_is_submitted() {
        let (actor, state) = actor_with_reads(["\r\n>", "\r\n+CMGS: 12\r\nOK\r\n"]);
        let submit = build_ucs2_submit("+12025550123", "中").unwrap();
        let body = submit.expose_for_confirmed_send().as_bytes().to_vec();

        // The prompt transaction itself is driven with a stand-in typed command; the real
        // `AT+CMGS` path is exercised by `send_uses_the_typed_cmgs_prompt_transaction`. The wire
        // bytes differ, the transaction state machine does not.
        let outcome =
            classify_send_outcome(actor.execute_prompt(AtCommand::Identity, body.clone()));

        assert_eq!(outcome.unwrap(), SmsSendResult::Submitted);
        let mut expected = body;
        expected.push(0x1A);
        assert_eq!(
            state.lock().unwrap().writes,
            vec![b"ATI\r".to_vec(), expected]
        );
    }

    #[test]
    fn module_rejection_after_the_body_is_failed() {
        for reply in ["\r\nERROR\r\n", "\r\n+CMS ERROR: 500\r\n"] {
            let (actor, state) = actor_with_reads(["\r\n>", reply]);

            let outcome =
                classify_send_outcome(actor.execute_prompt(AtCommand::Identity, b"41".to_vec()));

            assert_eq!(outcome.unwrap(), SmsSendResult::Failed, "{reply:?}");
            assert_eq!(state.lock().unwrap().write_attempts, 2);
        }
    }

    #[test]
    fn prompt_timeout_writes_no_body_and_is_outcome_unknown() {
        let (actor, state) = quiet_actor();

        let outcome =
            classify_send_outcome(actor.execute_prompt(AtCommand::Identity, b"41".to_vec()));

        assert_eq!(outcome.unwrap(), SmsSendResult::OutcomeUnknown);
        assert_eq!(state.lock().unwrap().write_attempts, 1);
    }

    #[test]
    fn result_timeout_after_the_body_is_outcome_unknown_and_never_resends() {
        let (actor, state) = actor_with_reads_then_quiet(["\r\n>"]);

        let outcome =
            classify_send_outcome(actor.execute_prompt(AtCommand::Identity, b"41".to_vec()));

        assert_eq!(outcome.unwrap(), SmsSendResult::OutcomeUnknown);
        let state = state.lock().unwrap();
        assert_eq!(state.write_attempts, 2);
        assert_eq!(
            state.writes,
            vec![b"ATI\r".to_vec(), vec![b'4', b'1', 0x1A]]
        );
    }

    #[test]
    fn failed_body_write_is_outcome_unknown() {
        let state = Arc::new(Mutex::new(FakeState {
            reads: [Ok(b"\r\n>".to_vec())].into_iter().collect(),
            fail_write_at: Some(1),
            ..FakeState::default()
        }));
        let actor = AtSessionActor::spawn(DeviceEpoch(7), Box::new(FakeSerial(Arc::clone(&state))));

        let outcome =
            classify_send_outcome(actor.execute_prompt(AtCommand::Identity, b"41".to_vec()));

        assert_eq!(outcome.unwrap(), SmsSendResult::OutcomeUnknown);
        let state = state.lock().unwrap();
        assert_eq!(state.write_attempts, 2);
        assert_eq!(state.writes, vec![b"ATI\r".to_vec()]);
    }
    #[test]
    fn inbox_os_error_keeps_native_code() {
        let error = map_actor_error(&ActorError::OsIo {
            kind: std::io::ErrorKind::PermissionDenied,
            raw_os_error: Some(5),
        });
        assert_eq!(error.code, "sms:port_open_failed");
        assert_eq!(error.os_code, Some(5));
    }
}
