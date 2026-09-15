use dji4g_at_protocol::{
    AtCommand, AtFinalCode, AtResponse, PdpContextId, PdpContextState, PdpType, parse_pdp_contexts,
    parse_pdp_contexts_with_activity,
};
use dji4g_domain::DeviceEpoch;

fn response(command: AtCommand, lines: &[&str]) -> AtResponse {
    AtResponse {
        epoch: DeviceEpoch(7),
        command,
        lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        final_code: AtFinalCode::Ok,
    }
}

#[test]
fn complete_cgdcont_and_cgact_parse_is_typed_and_marks_inactive_context() {
    let contexts = response(
        AtCommand::PdpContexts,
        &[
            r#"+CGDCONT: 1,"IP","internet.example","10.0.0.2""#,
            r#"+CGDCONT: 3,"IPV4V6","backup.example""#,
        ],
    );
    let activity = response(AtCommand::PdpActivation, &["+CGACT: 1,0", "+CGACT: 3,1"]);

    let parsed = parse_pdp_contexts_with_activity(&contexts, &activity).expect("valid fixture");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].cid(), PdpContextId::try_from(1).unwrap());
    assert_eq!(parsed[0].pdp_type(), PdpType::Ip);
    assert_eq!(parsed[0].apn().as_str(), "internet.example");
    assert_eq!(parsed[0].state(), PdpContextState::Inactive);
    assert_eq!(parsed[1].pdp_type(), PdpType::Ipv4v6);
    assert_eq!(parsed[1].state(), PdpContextState::Active);
}

#[test]
fn incomplete_or_duplicate_contexts_are_rejected_without_partial_fallback() {
    let contexts = response(
        AtCommand::PdpContexts,
        &[
            r#"+CGDCONT: 1,"IP","internet.example""#,
            "+CGDCONT: 1,garbage",
        ],
    );
    let activity = response(AtCommand::PdpActivation, &["+CGACT: 1,0"]);

    assert!(parse_pdp_contexts_with_activity(&contexts, &activity).is_err());
    assert!(parse_pdp_contexts(&contexts).is_err());
}
