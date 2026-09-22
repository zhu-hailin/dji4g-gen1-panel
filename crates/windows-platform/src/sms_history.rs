//! One bounded, same-session history read and optional reversible mem1 selection.
use crate::{ActorError, AtSessionActor, DjiDevice, PlatformError, SmsListing};
use dji4g_at_protocol::{AtCommand, AtFinalCode, AtResponse};
use dji4g_domain::{
    DeviceEpoch, SmsReadControl, SmsReadPhase, SmsReadReport, SmsReadStorage, SmsStorageId,
    SmsStorageRestoration,
};
use std::time::Duration;

/// Read only when the same caller-verified SIM fingerprint matches before and after CMGL.
/// Missing/unreadable/changed identity yields no records; storage restoration still runs.
pub fn sms_list_controlled(
    device: &DjiDevice,
    epoch: DeviceEpoch,
    expected_sim: Option<[u8; 8]>,
    storage: Option<SmsStorageId>,
    control: SmsReadControl,
) -> Result<SmsListing, PlatformError> {
    if expected_sim.is_none() {
        return Err(error("sms:sim_identity_required"));
    }
    if control.is_cancelled() || control.is_expired() {
        return Err(actor_failure(ActorError::Closed, &control));
    }
    #[cfg(windows)]
    {
        let selection = crate::select_at_port_verified(device.com_candidates(), &[], 2)
            .map_err(|_| error("sms:verification_failed"))?;
        let candidates = match selection {
            crate::AtPortSelection::Classified(port) => vec![port],
            crate::AtPortSelection::HandshakeCandidates(ports) => ports,
        };
        let mut last = Err(error("sms:verification_failed"));
        for selected in candidates {
            let mut actor =
                AtSessionActor::open_selected_delete(epoch, &selected, control.transport_control())
                    .map_err(|e| actor_failure(e, &control))?;
            let result = actor.execute_sms_history(expected_sim, storage.clone(), control.clone());
            let _ =
                actor.close_delete_and_wait(control.transport_control(), Duration::from_secs(3));
            if control.cleanup_pending()
                || control.is_cancelled()
                || control.is_expired()
                || result
                    .as_ref()
                    .err()
                    .is_none_or(|e| e.code != "sms:handshake_failed")
            {
                return result;
            }
            last = result;
        }
        last
    }
    #[cfg(not(windows))]
    {
        let _ = (device, epoch, expected_sim, storage);
        Err(error("sms:unsupported"))
    }
}
fn error(code: &'static str) -> PlatformError {
    PlatformError {
        code,
        os_code: None,
    }
}
pub(crate) fn actor_failure(error_value: ActorError, control: &SmsReadControl) -> PlatformError {
    if control.is_expired() {
        error("sms:timeout")
    } else if control.is_cancelled() {
        error("sms:cancelled")
    } else {
        crate::sms::map_actor_error(&error_value)
    }
}
#[derive(Clone, Debug)]
struct StorageSnapshot {
    names: [String; 3],
    capacity: (u32, u32),
}
fn snapshot(lines: &[String]) -> Option<StorageSnapshot> {
    let mut payloads = lines.iter().filter_map(|line| line.strip_prefix("+CPMS:"));
    let payload = payloads.next()?;
    if payloads.next().is_some() {
        return None;
    }
    let fields: Vec<_> = payload.split(',').map(str::trim).collect();
    if fields.len() != 9 {
        return None;
    }
    let mut names = Vec::new();
    for group in fields.chunks_exact(3) {
        let name = group[0].strip_prefix('"')?.strip_suffix('"')?;
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_uppercase()) {
            return None;
        }
        let used = group[1].parse::<u32>().ok()?;
        let total = group[2].parse::<u32>().ok()?;
        if used > total {
            return None;
        }
        names.push(name.to_owned());
    }
    Some(StorageSnapshot {
        names: names.try_into().ok()?,
        capacity: (fields[1].parse().ok()?, fields[2].parse().ok()?),
    })
}
fn supported(lines: &[String]) -> Option<Vec<SmsStorageId>> {
    if lines.len() != 1 {
        return None;
    }
    let payload = lines[0].strip_prefix("+CPMS:")?.trim();
    let first = payload.strip_prefix('(')?.split_once(')')?.0;
    let mut result = Vec::new();
    for token in first.split(',').map(str::trim) {
        let value = token.strip_prefix('"')?.strip_suffix('"')?;
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_uppercase()) {
            return None;
        }
        if !result.iter().any(|s: &SmsStorageId| s.0 == value) {
            result.push(SmsStorageId(value.into()));
        }
    }
    Some(result)
}

fn verify_sim(
    execute: &mut impl FnMut(AtCommand, bool) -> Result<AtResponse, PlatformError>,
    expected: [u8; 8],
    control: &SmsReadControl,
) -> Result<(), PlatformError> {
    let response = execute(AtCommand::Iccid, false).map_err(|failure| {
        if control.is_cancelled() || control.is_expired() {
            failure
        } else {
            error("sms:sim_identity_unverified")
        }
    })?;
    if response.lines.len() != 1 {
        return Err(error("sms:sim_identity_unverified"));
    }
    let iccid = dji4g_at_protocol::parse_iccid_line(&response.lines[0])
        .ok_or(error("sms:sim_identity_unverified"))?;
    // Identical to SimIdentity.fingerprint: SHA-256 prefix, never raw ICCID in a result.
    let digest = dji4g_domain::sha256(iccid.as_bytes());
    if digest[..8] != expected {
        return Err(error("sms:sim_changed"));
    }
    Ok(())
}

pub(crate) fn list_in_session(
    epoch: DeviceEpoch,
    expected_sim: Option<[u8; 8]>,
    storage: Option<SmsStorageId>,
    control: &SmsReadControl,
    mut transact: impl FnMut(AtCommand, bool) -> Result<AtResponse, ActorError>,
) -> Result<SmsListing, PlatformError> {
    let expected_sim = expected_sim.ok_or(error("sms:sim_identity_required"))?;
    let requested = match storage.as_ref().map(|s| s.0.as_str()) {
        None => None,
        Some("SM") => Some(SmsReadStorage::Sim),
        Some("ME") => Some(SmsReadStorage::Device),
        _ => return Err(error("sms:unsupported_storage")),
    };
    let mut execute = |command: AtCommand, cleanup: bool| {
        if !cleanup && (control.is_cancelled() || control.is_expired()) {
            return Err(actor_failure(ActorError::Closed, control));
        }
        let response = transact(command.clone(), cleanup).map_err(|e| actor_failure(e, control))?;
        if response.epoch != epoch || response.command != command {
            return Err(error("sms:stale_epoch"));
        }
        if response.final_code != AtFinalCode::Ok {
            return Err(error("sms:module_rejected"));
        }
        Ok(response)
    };
    control.set_phase(SmsReadPhase::Verifying);
    let handshake =
        execute(AtCommand::Attention, false).and_then(|_| execute(AtCommand::Identity, false));
    match handshake {
        Ok(identity) if !identity.lines.is_empty() => {}
        Err(e) if control.is_cancelled() || control.is_expired() => return Err(e),
        _ => return Err(error("sms:handshake_failed")),
    }
    verify_sim(&mut execute, expected_sim, control)?;
    let mode = execute(AtCommand::SmsMessageFormat, false)?;
    if crate::sms::parse_cmgf_mode(&mode.lines) != Some(true) {
        execute(AtCommand::SmsSetPduMode, false)?;
        let confirmed = execute(AtCommand::SmsMessageFormat, false)?;
        if crate::sms::parse_cmgf_mode(&confirmed.lines) != Some(true) {
            return Err(error("sms:pdu_confirm_failed"));
        }
    }
    control.set_phase(SmsReadPhase::ReadingStorage);
    let original = snapshot(&execute(AtCommand::SmsStorageQuery, false)?.lines)
        .ok_or(error("sms:unknown_storage"))?;
    let supported_storages = supported(&execute(AtCommand::SmsStorageCapabilities, false)?.lines)
        .ok_or(error("sms:unknown_storage"))?;
    let target = requested
        .map(|s| s.as_str())
        .unwrap_or(&original.names[0])
        .to_owned();
    if requested.is_some() && !supported_storages.iter().any(|s| s.0 == target) {
        return Err(error("sms:unsupported_storage"));
    }
    let switching = target != original.names[0];
    let restore = if switching {
        Some(SmsReadStorage::from_token(&original.names[0]).ok_or(error("sms:unknown_storage"))?)
    } else {
        None
    };
    let outcome = (|| {
        let current = if switching {
            control.set_phase(SmsReadPhase::SwitchingStorage);
            // Mark uncertain before even a partial serial write. Restore on every later exit.
            control.set_restoration(SmsStorageRestoration::Unknown);
            execute(
                AtCommand::SmsSelectStorage {
                    storage: requested.unwrap(),
                },
                false,
            )?;
            let selected = snapshot(&execute(AtCommand::SmsStorageQuery, false)?.lines)
                .ok_or(error("sms:storage_mismatch"))?;
            if selected.names[0] != target || selected.names[1..] != original.names[1..] {
                return Err(error("sms:storage_mismatch"));
            }
            selected
        } else {
            original.clone()
        };
        control.set_phase(SmsReadPhase::Listing);
        let response = execute(AtCommand::SmsList, false)?;
        verify_sim(&mut execute, expected_sim, control)?;
        control.set_phase(SmsReadPhase::Decoding);
        let raw_records = response
            .lines
            .iter()
            .filter(|line| line.starts_with("+CMGL:"))
            .count();
        let records = crate::sms::pair_cmgl_records(&response.lines, &target);
        control.set_progress(raw_records);
        let report = SmsReadReport {
            storage: Some(SmsStorageId(target.clone())),
            capacity: Some(current.capacity),
            raw_records,
            decoded_records: records.len(),
            skipped_records: raw_records.saturating_sub(records.len()),
            supported_storages,
            restoration: SmsStorageRestoration::NotNeeded,
        };
        Ok(SmsListing {
            records,
            capacity: report.capacity,
            report,
        })
    })();
    if let Some(restore) = restore {
        control.set_phase(SmsReadPhase::RestoringStorage);
        // Independent cleanup budget is supplied by the actor, ignoring cancelled read budget.
        let restored = execute(AtCommand::SmsSelectStorage { storage: restore }, true)
            .and_then(|_| execute(AtCommand::SmsStorageQuery, true))
            .ok()
            .and_then(|r| snapshot(&r.lines))
            .is_some_and(|s| s.names == original.names);
        if !restored {
            control.set_restoration(SmsStorageRestoration::Unknown);
            return Err(error("sms:storage_restore_unknown"));
        }
        control.set_restoration(SmsStorageRestoration::Restored);
    }
    control.set_phase(SmsReadPhase::Complete);
    outcome.map(|mut listing| {
        listing.report.restoration = control.restoration();
        listing
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_sim() -> [u8; 8] {
        dji4g_domain::sha256(b"89860123456789012345")[..8]
            .try_into()
            .unwrap()
    }
    use dji4g_at_protocol::AtFinalCode;
    fn response(command: AtCommand, lines: &[&str]) -> AtResponse {
        AtResponse {
            epoch: DeviceEpoch(7),
            command,
            lines: lines.iter().map(|s| s.to_string()).collect(),
            final_code: AtFinalCode::Ok,
        }
    }
    fn execute(
        storage: Option<&str>,
        fail_list: bool,
        fail_restore: bool,
    ) -> (
        Result<SmsListing, PlatformError>,
        Vec<AtCommand>,
        SmsReadControl,
    ) {
        let control = SmsReadControl::new(Duration::from_secs(60));
        let mut writes = Vec::new();
        let mut holder = "MT";
        let mut listing_seen = false;
        let result = list_in_session(
            DeviceEpoch(7),
            Some(test_sim()),
            storage.map(|s| SmsStorageId(s.into())),
            &control,
            |command, cleanup| {
                writes.push(command.clone());
                let lines = match &command {
                    AtCommand::Identity => vec!["Quectel EC25".into()],
                    AtCommand::Iccid => vec!["+QCCID: 89860123456789012345".into()],
                    AtCommand::SmsMessageFormat => vec!["+CMGF: 0".into()],
                    AtCommand::SmsStorageCapabilities => {
                        vec!["+CPMS: (\"SM\",\"ME\",\"MT\"),(\"ME\"),(\"SM\")".into()]
                    }
                    AtCommand::SmsStorageQuery => vec![format!(
                        "+CPMS: \"{holder}\",2,500,\"ME\",9,100,\"SM\",1,50"
                    )],
                    AtCommand::SmsSelectStorage { storage } => {
                        assert_eq!(cleanup, listing_seen);
                        if cleanup && fail_restore {
                            return Err(ActorError::Closed);
                        }
                        holder = storage.as_str();
                        vec![]
                    }
                    AtCommand::SmsList => {
                        listing_seen = true;
                        if fail_list {
                            control.cancel();
                            return Err(ActorError::Closed);
                        }
                        vec![
                            "+CMGL: 1,1,,1".into(),
                            "FF".into(),
                            "+CMGL: 2,1,,1".into(),
                            "FF".into(),
                        ]
                    }
                    _ => vec![],
                };
                Ok(response(
                    command,
                    &lines.iter().map(String::as_str).collect::<Vec<_>>(),
                ))
            },
        );
        (result, writes, control)
    }
    #[test]
    fn history_reports_raw_slots_and_skipped_decoding() {
        let (result, writes, _) = execute(None, false, false);
        let result = result.unwrap();
        assert_eq!(result.report.storage, Some(SmsStorageId("MT".into())));
        assert_eq!(result.report.capacity, Some((2, 500)));
        assert_eq!(result.report.raw_records, 2);
        assert_eq!(result.report.skipped_records, 2);
        assert_eq!(result.report.decoded_records, 0);
        assert_eq!(
            writes
                .iter()
                .filter(|c| matches!(c, AtCommand::SmsList))
                .count(),
            1
        );
    }
    #[test]
    fn history_selected_storage_restores_exact_original_mt() {
        let (result, writes, control) = execute(Some("SM"), false, false);
        let result = result.unwrap();
        assert_eq!(result.report.storage, Some(SmsStorageId("SM".into())));
        assert_eq!(control.restoration(), SmsStorageRestoration::Restored);
        let selects: Vec<_> = writes
            .iter()
            .filter(|c| matches!(c, AtCommand::SmsSelectStorage { .. }))
            .collect();
        assert_eq!(
            selects,
            vec![
                &AtCommand::SmsSelectStorage {
                    storage: SmsReadStorage::Sim
                },
                &AtCommand::SmsSelectStorage {
                    storage: SmsReadStorage::Combined
                }
            ]
        );
        assert_eq!(writes.last(), Some(&AtCommand::SmsStorageQuery));
    }
    #[test]
    fn history_cancelled_listing_still_restores() {
        let (result, writes, control) = execute(Some("ME"), true, false);
        assert_eq!(result.unwrap_err().code, "sms:cancelled");
        assert_eq!(control.restoration(), SmsStorageRestoration::Restored);
        assert_eq!(writes.last(), Some(&AtCommand::SmsStorageQuery));
    }
    #[test]
    fn history_failed_restore_is_unknown() {
        let (result, _, control) = execute(Some("ME"), false, true);
        assert_eq!(result.unwrap_err().code, "sms:storage_restore_unknown");
        assert_eq!(control.restoration(), SmsStorageRestoration::Unknown);
    }

    #[test]
    fn history_switches_pdu_once_and_confirms_before_listing() {
        let control = SmsReadControl::new(Duration::from_secs(10));
        let mut commands = Vec::new();
        let mut enabled = false;
        let result = list_in_session(
            DeviceEpoch(7),
            Some(test_sim()),
            None,
            &control,
            |command, _| {
                commands.push(command.clone());
                let lines = match command {
                    AtCommand::Identity => vec!["Quectel EC25"],
                    AtCommand::Iccid => vec!["+QCCID: 89860123456789012345"],
                    AtCommand::SmsMessageFormat => {
                        vec![if enabled { "+CMGF: 0" } else { "+CMGF: 1" }]
                    }
                    AtCommand::SmsSetPduMode => {
                        enabled = true;
                        vec![]
                    }
                    AtCommand::SmsStorageQuery => {
                        vec!["+CPMS: \"SM\",0,50,\"ME\",0,50,\"SM\",0,50"]
                    }
                    AtCommand::SmsStorageCapabilities => {
                        vec!["+CPMS: (\"SM\",\"ME\"),(\"ME\"),(\"SM\")"]
                    }
                    _ => vec![],
                };
                Ok(response(command, &lines))
            },
        );
        assert!(result.is_ok());
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, AtCommand::SmsSetPduMode))
                .count(),
            1
        );
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, AtCommand::SmsMessageFormat))
                .count(),
            2
        );
    }
    #[test]
    fn history_rejects_mt_as_user_target() {
        let (result, writes, _) = execute(Some("MT"), false, false);
        assert_eq!(result.unwrap_err().code, "sms:unsupported_storage");
        assert!(
            !writes
                .iter()
                .any(|c| matches!(c, AtCommand::SmsSelectStorage { .. } | AtCommand::SmsList))
        );
    }

    #[test]
    fn history_explicit_same_holder_must_still_be_supported() {
        let control = SmsReadControl::new(Duration::from_secs(1));
        let mut listed = false;
        let result = list_in_session(
            DeviceEpoch(7),
            Some(test_sim()),
            Some(SmsStorageId("SM".into())),
            &control,
            |command, _| {
                let lines = match command {
                    AtCommand::Identity => vec!["Quectel EC25"],
                    AtCommand::Iccid => vec!["+QCCID: 89860123456789012345"],
                    AtCommand::SmsMessageFormat => vec!["+CMGF: 0"],
                    AtCommand::SmsStorageQuery => {
                        vec!["+CPMS: \"SM\",0,50,\"ME\",0,50,\"SM\",0,50"]
                    }
                    AtCommand::SmsStorageCapabilities => vec!["+CPMS: (\"ME\"),(\"ME\"),(\"SM\")"],
                    AtCommand::SmsList => {
                        listed = true;
                        vec![]
                    }
                    _ => vec![],
                };
                Ok(response(command, &lines))
            },
        );
        assert_eq!(result.unwrap_err().code, "sms:unsupported_storage");
        assert!(!listed);
    }
    #[test]
    fn history_unsupported_capability_query_must_not_continue_after_transport_failure() {
        let control = SmsReadControl::new(Duration::from_secs(1));
        let mut listed = false;
        let result = list_in_session(
            DeviceEpoch(7),
            Some(test_sim()),
            None,
            &control,
            |command, _| {
                let lines = match command {
                    AtCommand::Identity => vec!["Quectel EC25"],
                    AtCommand::Iccid => vec!["+QCCID: 89860123456789012345"],
                    AtCommand::SmsMessageFormat => vec!["+CMGF: 0"],
                    AtCommand::SmsStorageQuery => {
                        vec!["+CPMS: \"SM\",0,50,\"ME\",0,50,\"SM\",0,50"]
                    }
                    AtCommand::SmsStorageCapabilities => return Err(ActorError::Closed),
                    AtCommand::SmsList => {
                        listed = true;
                        vec![]
                    }
                    _ => vec![],
                };
                Ok(response(command, &lines))
            },
        );
        assert!(result.is_err());
        assert!(!listed);
    }

    #[test]
    fn history_sim_missing_changed_or_unreadable_never_returns_messages() {
        for (expected, before, after, want_code, want_lists) in [
            (
                None,
                Some("89860123456789012345"),
                Some("89860123456789012345"),
                "sms:sim_identity_required",
                0,
            ),
            (
                Some(test_sim()),
                Some("89860123456789012346"),
                Some("89860123456789012346"),
                "sms:sim_changed",
                0,
            ),
            (
                Some(test_sim()),
                Some("89860123456789012345"),
                Some("89860123456789012346"),
                "sms:sim_changed",
                1,
            ),
            (
                Some(test_sim()),
                None,
                Some("89860123456789012345"),
                "sms:sim_identity_unverified",
                0,
            ),
            (
                Some(test_sim()),
                Some("89860123456789012345"),
                None,
                "sms:sim_identity_unverified",
                1,
            ),
        ] {
            let control = SmsReadControl::new(Duration::from_secs(1));
            let mut iccid_count = 0;
            let mut list_count = 0;
            let mut writes = 0;
            let result = list_in_session(DeviceEpoch(7), expected, None, &control, |command, _| {
                writes += 1;
                let lines = match command {
                    AtCommand::Identity => vec!["Quectel EC25".to_owned()],
                    AtCommand::Iccid => {
                        iccid_count += 1;
                        let iccid = if iccid_count == 1 { before } else { after };
                        let Some(iccid) = iccid else {
                            return Err(ActorError::Closed);
                        };
                        vec![format!("+QCCID: {iccid}")]
                    }
                    AtCommand::SmsMessageFormat => vec!["+CMGF: 0".to_owned()],
                    AtCommand::SmsStorageQuery => {
                        vec!["+CPMS: \"SM\",1,50,\"ME\",0,50,\"SM\",1,50".to_owned()]
                    }
                    AtCommand::SmsStorageCapabilities => {
                        vec!["+CPMS: (\"SM\",\"ME\"),(\"ME\"),(\"SM\")".to_owned()]
                    }
                    AtCommand::SmsList => {
                        list_count += 1;
                        vec![
                            "+CMGL: 1,1,,24".to_owned(),
                            "00040B912120550521F300004210203040502305E8329BFD06".to_owned(),
                        ]
                    }
                    _ => vec![],
                };
                Ok(response(
                    command,
                    &lines.iter().map(String::as_str).collect::<Vec<_>>(),
                ))
            });
            assert_eq!(result.unwrap_err().code, want_code);
            assert_eq!(list_count, want_lists);
            if expected.is_none() {
                assert_eq!(writes, 0);
            }
        }
    }
}
