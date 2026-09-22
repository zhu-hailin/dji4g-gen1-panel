//! Verified one-fragment deletion. No automatic retry and no mode/storage mutation.
use crate::{ActorError, AtSessionActor, DjiDevice};
use dji4g_at_protocol::{AtCommand, AtFinalCode, AtResponse, decode_deliver_pdu};
use dji4g_domain::{
    DeviceEpoch, SmsDeleteControl, SmsDeleteItemResult, SmsDeleteReceipt, SmsFragmentKey,
    SmsMessage, SmsStatus,
};
use std::time::Duration;

pub fn sms_delete_checked(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    expected: &SmsFragmentKey,
    control: SmsDeleteControl,
) -> SmsDeleteReceipt {
    if epoch.0 != expected.device_epoch {
        return refused("sms:stale_epoch");
    }
    if control.is_cancelled() || control.is_expired() {
        return actor_failure(ActorError::Closed, &control);
    }
    #[cfg(windows)]
    {
        let selection = match crate::select_at_port_verified(device.com_candidates(), &[], 2) {
            Ok(value) => value,
            Err(_) => return refused("sms:verification_failed"),
        };
        let candidates = match selection {
            crate::AtPortSelection::Classified(port) => vec![port],
            crate::AtPortSelection::HandshakeCandidates(ports) => ports,
        };
        // Each candidate receives only a typed handshake until its identity is verified.
        // The first verified session performs all remaining checks and exactly one delete.
        let mut last = refused("sms:verification_failed");
        for selected in candidates {
            let mut actor = match AtSessionActor::open_selected_delete(epoch, &selected, &control) {
                Ok(value) => value,
                Err(error) => return actor_failure(error, &control),
            };
            let receipt = actor.execute_checked_delete(expected.clone(), control.clone());
            let _ = actor.close_delete_and_wait(&control, Duration::from_secs(3));
            if control.cleanup_pending()
                || control.delete_attempted()
                || control.is_cancelled()
                || control.is_expired()
                || receipt.code.as_deref() != Some("sms:handshake_failed")
            {
                return receipt;
            }
            last = receipt;
        }
        last
    }
    #[cfg(not(windows))]
    {
        let _ = device;
        refused("sms:unsupported")
    }
}

pub(crate) fn delete_in_session(
    epoch: DeviceEpoch,
    expected: &SmsFragmentKey,
    control: &SmsDeleteControl,
    mut transact: impl FnMut(AtCommand) -> Result<AtResponse, ActorError>,
) -> SmsDeleteReceipt {
    let mut execute = |command| {
        let response = transact(command)?;
        if response.final_code == AtFinalCode::Ok {
            Ok(response)
        } else {
            Err(ActorError::FinalCode(response.final_code))
        }
    };
    if expected.device_epoch != epoch.0 {
        return refused("sms:stale_epoch");
    }
    if expected.storage.0.trim().is_empty() {
        return refused("sms:unknown_storage");
    }
    let handshake = execute(AtCommand::Attention).and_then(|_| execute(AtCommand::Identity));
    match handshake {
        Ok(identity) if !identity.lines.is_empty() => {}
        Err(error) if control.is_cancelled() || control.is_expired() => {
            return actor_failure(error, control);
        }
        _ => return refused("sms:handshake_failed"),
    }
    let outcome = (|| {
        let mode = execute(AtCommand::SmsMessageFormat)?;
        if mode
            .lines
            .iter()
            .filter(|line| line.starts_with("+CMGF:"))
            .count()
            != 1
            || crate::sms::parse_cmgf_mode(&mode.lines) != Some(true)
        {
            return Ok(refused("sms:pdu_mode_required"));
        }
        let storage = execute(AtCommand::SmsStorageQuery)?;
        let holders: Vec<_> = storage
            .lines
            .iter()
            .filter_map(|line| line.strip_prefix("+CPMS:"))
            .collect();
        // CPMS mem1 is the holder used by both CMGR and CMGD; never infer it from mem2/mem3.
        if holders.len() != 1 {
            return Ok(refused("sms:unknown_storage"));
        }
        let fields: Vec<_> = holders[0].split(',').map(str::trim).collect();
        let holder = fields
            .first()
            .and_then(|value| value.strip_prefix('"'))
            .and_then(|value| value.strip_suffix('"'));
        if holder.is_none_or(str::is_empty)
            || fields.get(1).and_then(|v| v.parse::<u32>().ok()).is_none()
            || fields.get(2).and_then(|v| v.parse::<u32>().ok()).is_none()
        {
            return Ok(refused("sms:unknown_storage"));
        }
        if holder != Some(expected.storage.0.as_str()) {
            return Ok(refused("sms:storage_mismatch"));
        }
        let read = execute(AtCommand::SmsRead {
            index: expected.index,
        })?;
        let Some(message) = decode_read(&read.lines, expected) else {
            return Ok(refused("sms:verification_failed"));
        };
        if message.payload_fingerprint() != expected.payload_fingerprint {
            return Ok(refused("sms:payload_mismatch"));
        }
        execute(AtCommand::SmsDelete {
            index: expected.index,
        })?;
        Ok(SmsDeleteReceipt {
            result: SmsDeleteItemResult::Deleted,
            code: None,
        })
    })();
    outcome.unwrap_or_else(|error| actor_failure(error, control))
}

fn decode_read(lines: &[String], expected: &SmsFragmentKey) -> Option<SmsMessage> {
    let mut headers = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("+CMGR:"));
    let (position, header) = headers.next()?;
    if headers.next().is_some() {
        return None;
    }
    let status = header
        .strip_prefix("+CMGR:")?
        .split(',')
        .next()?
        .trim()
        .parse::<u8>()
        .ok()?;
    if status > 4 {
        return None;
    }
    let decoded = decode_deliver_pdu(lines.get(position + 1)?.trim()).ok()?;
    if decoded.encoding == dji4g_domain::SmsEncoding::Other {
        return None;
    }
    let mut message = SmsMessage::new(
        expected.index,
        expected.storage.clone(),
        expected.device_epoch,
        expected.sim_epoch,
        decoded.sender,
        decoded.body,
        decoded.encoding,
        SmsStatus::Received,
    );
    message.service_centre_timestamp = decoded.timestamp;
    message.multipart = decoded.multipart;
    Some(message)
}

fn refused(code: &str) -> SmsDeleteReceipt {
    SmsDeleteReceipt {
        result: SmsDeleteItemResult::NotAttempted,
        code: Some(code.into()),
    }
}

pub(crate) fn actor_failure(error: ActorError, control: &SmsDeleteControl) -> SmsDeleteReceipt {
    let definitive = matches!(
        error,
        ActorError::FinalCode(
            AtFinalCode::Error | AtFinalCode::CmsError(_) | AtFinalCode::CmeError(_)
        )
    );
    let code = if definitive {
        "sms:module_rejected"
    } else if control.is_expired() {
        "sms:timeout"
    } else if control.is_cancelled() {
        "sms:cancelled"
    } else {
        error.code()
    };
    SmsDeleteReceipt {
        result: if definitive {
            SmsDeleteItemResult::Failed
        } else if control.delete_attempted() {
            SmsDeleteItemResult::OutcomeUnknown
        } else {
            SmsDeleteItemResult::NotAttempted
        },
        code: Some(code.into()),
    }
}
