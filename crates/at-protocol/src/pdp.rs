use std::fmt;

use crate::{
    Apn, AtCommand, AtFinalCode, AtResponse, PdpContext, PdpContextId, PdpContextState, PdpType,
};

/// Complete, typed parsing errors for the two responses needed by APN repair.
///
/// This error intentionally contains no response text.  Raw modem lines can include APN and
/// identity material and must not become part of an error or audit record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdpParseError {
    WrongCommand,
    WrongEpoch,
    FinalCode,
    EmptyResponse,
    MalformedContext,
    DuplicateContext,
    UnsupportedPdpType,
    InvalidApn,
    MissingActivity,
    DuplicateActivity,
    UnknownActivity,
}

impl fmt::Display for PdpParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongCommand => "at:pdp_wrong_command",
            Self::WrongEpoch => "at:pdp_epoch_mismatch",
            Self::FinalCode => "at:pdp_final_error",
            Self::EmptyResponse => "at:pdp_empty_response",
            Self::MalformedContext => "at:pdp_malformed_context",
            Self::DuplicateContext => "at:pdp_duplicate_context",
            Self::UnsupportedPdpType => "at:pdp_unsupported_type",
            Self::InvalidApn => "at:pdp_invalid_apn",
            Self::MissingActivity => "at:pdp_missing_activity",
            Self::DuplicateActivity => "at:pdp_duplicate_activity",
            Self::UnknownActivity => "at:pdp_unknown_activity",
        })
    }
}

impl std::error::Error for PdpParseError {}

/// Parse a complete successful `AT+CGDCONT?` response.
pub fn parse_pdp_contexts(response: &AtResponse) -> Result<Vec<PdpContext>, PdpParseError> {
    if response.command != AtCommand::PdpContexts {
        return Err(PdpParseError::WrongCommand);
    }
    if response.final_code != AtFinalCode::Ok {
        return Err(PdpParseError::FinalCode);
    }
    if response.lines.is_empty() {
        return Err(PdpParseError::EmptyResponse);
    }

    let mut contexts = Vec::with_capacity(response.lines.len());
    for line in &response.lines {
        let context = parse_context_line(line)?;
        if contexts
            .iter()
            .any(|item: &PdpContext| item.cid() == context.cid())
        {
            return Err(PdpParseError::DuplicateContext);
        }
        contexts.push(context);
    }
    contexts.sort_by_key(|context| context.cid());
    Ok(contexts)
}

/// Parse complete `+CGDCONT` and `+CGACT` responses and attach activation state.
pub fn parse_pdp_contexts_with_activity(
    contexts_response: &AtResponse,
    activity_response: &AtResponse,
) -> Result<Vec<PdpContext>, PdpParseError> {
    if contexts_response.epoch != activity_response.epoch {
        return Err(PdpParseError::WrongEpoch);
    }
    let contexts = parse_pdp_contexts(contexts_response)?;
    if activity_response.command != AtCommand::PdpActivation {
        return Err(PdpParseError::WrongCommand);
    }
    if activity_response.final_code != AtFinalCode::Ok {
        return Err(PdpParseError::FinalCode);
    }
    if activity_response.lines.is_empty() {
        return Err(PdpParseError::EmptyResponse);
    }

    let mut activity = Vec::with_capacity(activity_response.lines.len());
    for line in &activity_response.lines {
        let (cid, state) = parse_activity_line(line)?;
        if activity
            .iter()
            .any(|(item, _): &(PdpContextId, _)| *item == cid)
        {
            return Err(PdpParseError::DuplicateActivity);
        }
        if !contexts.iter().any(|context| context.cid() == cid) {
            return Err(PdpParseError::UnknownActivity);
        }
        activity.push((cid, state));
    }
    if activity.len() != contexts.len()
        || contexts
            .iter()
            .any(|context| !activity.iter().any(|(cid, _)| *cid == context.cid()))
    {
        return Err(PdpParseError::MissingActivity);
    }

    contexts
        .into_iter()
        .map(|context| {
            let state = activity
                .iter()
                .find(|(cid, _)| *cid == context.cid())
                .map_or(PdpContextState::Inactive, |(_, state)| *state);
            Ok(context.with_state(state))
        })
        .collect()
}

fn parse_context_line(line: &str) -> Result<PdpContext, PdpParseError> {
    let payload = line
        .strip_prefix("+CGDCONT:")
        .ok_or(PdpParseError::MalformedContext)?
        .trim();
    let fields = split_csv(payload).ok_or(PdpParseError::MalformedContext)?;
    if fields.len() < 3 || fields[3..].iter().any(|field| field.is_empty()) {
        return Err(PdpParseError::MalformedContext);
    }
    let cid = fields[0]
        .parse::<u8>()
        .ok()
        .and_then(|value| PdpContextId::try_from(value).ok())
        .ok_or(PdpParseError::MalformedContext)?;
    let pdp_type = match unquote(fields[1]).ok_or(PdpParseError::MalformedContext)? {
        "IP" => PdpType::Ip,
        "IPV6" => PdpType::Ipv6,
        "IPV4V6" => PdpType::Ipv4v6,
        _ => return Err(PdpParseError::UnsupportedPdpType),
    };
    let apn = Apn::try_from(unquote(fields[2]).ok_or(PdpParseError::MalformedContext)?)
        .map_err(|_| PdpParseError::InvalidApn)?;
    Ok(PdpContext::new(
        cid,
        pdp_type,
        apn,
        PdpContextState::Inactive,
    ))
}

fn parse_activity_line(line: &str) -> Result<(PdpContextId, PdpContextState), PdpParseError> {
    let payload = line
        .strip_prefix("+CGACT:")
        .ok_or(PdpParseError::MalformedContext)?
        .trim();
    let fields = split_csv(payload).ok_or(PdpParseError::MalformedContext)?;
    if fields.len() != 2 {
        return Err(PdpParseError::MalformedContext);
    }
    let cid = fields[0]
        .parse::<u8>()
        .ok()
        .and_then(|value| PdpContextId::try_from(value).ok())
        .ok_or(PdpParseError::MalformedContext)?;
    let state = match fields[1] {
        "0" => PdpContextState::Inactive,
        "1" => PdpContextState::Active,
        _ => return Err(PdpParseError::MalformedContext),
    };
    Ok((cid, state))
}

fn split_csv(payload: &str) -> Option<Vec<&str>> {
    let mut fields = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, byte) in payload.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                fields.push(payload.get(start..index)?.trim());
                start = index + 1;
            }
            byte if byte.is_ascii_control() => return None,
            _ => {}
        }
    }
    if quoted {
        return None;
    }
    fields.push(payload.get(start..)?.trim());
    Some(fields)
}

fn unquote(value: &str) -> Option<&str> {
    (value.len() >= 2 && value.starts_with('"') && value.ends_with('"'))
        .then(|| value.get(1..value.len() - 1))
        .flatten()
        .filter(|value| !value.contains('"'))
}
