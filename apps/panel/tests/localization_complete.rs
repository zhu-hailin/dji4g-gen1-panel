use std::collections::HashSet;

use dji4g_panel::localization::{
    Language, TextKey, available_languages, english_available, stable_code_text, template,
};

#[test]
fn catalog_keys_are_unique_and_have_simplified_chinese_text() {
    let unique = TextKey::ALL.iter().copied().collect::<HashSet<_>>();
    assert_eq!(unique.len(), TextKey::ALL.len());

    for key in TextKey::ALL {
        let value = template(Language::ZhCn, *key);
        assert!(!value.trim().is_empty(), "empty text for {key:?}");
        assert!(!value.contains("TODO"));
        assert!(!value.contains("TBD"));
        assert!(
            value
                .chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)),
            "text for {key:?} is not zh-CN: {value}"
        );
    }
}

#[test]
fn language_switch_stays_hidden_until_english_is_complete() {
    assert!(!english_available());
    assert_eq!(available_languages(), &[Language::ZhCn]);
}

#[test]
fn known_backend_code_namespaces_have_a_localized_fallback() {
    let codes = [
        "apn:empty",
        "at_protocol:wrong_port_data",
        "pdp_context_id:out_of_range",
        "net:adapter_identity_mismatch",
        "net:route_enumeration_failed",
        "pnp:property_invalid",
        "pnp:registry_value_invalid",
        "probe:bind_failed",
        "probe:connect_failed",
        "probe:dns_timeout",
        "probe:http_invalid",
        "probe:tls_timeout",
        "app:adapter_not_ready",
        "app:missing_before_state",
        "at:future",
        "operation:uac_cancelled",
        "route:not_observed",
        "serial_actor:closed",
        "export:write_failed",
        "export:path_unavailable",
        "privilege:helper_unsigned",
        "privilege:helper_unverified",
        "sms:pdu_mode_required",
        "sms:pdu_confirm_failed",
        "sms:invalid_message",
        "sms:send_failed",
        "sms:timeout",
        "sms:device_removed",
        "sms:unsupported",
        "sms:verification_failed",
        "sms:internal",
        "sms:send_unavailable",
    ];

    for code in codes {
        assert!(
            stable_code_text(code).is_some(),
            "unlocalized stable code: {code}"
        );
    }
    assert_eq!(stable_code_text("future_component:new_code"), None);
}

#[test]
fn sms_stable_codes_map_to_precise_operator_notes() {
    let cases = [
        ("sms:pdu_mode_required", "PDU"),
        ("sms:pdu_confirm_failed", "未能确认"),
        ("sms:invalid_message", "收件人"),
        ("sms:send_failed", "模块拒绝"),
        ("sms:timeout", "超时"),
        ("sms:device_removed", "断开"),
        ("sms:unsupported", "不支持"),
        ("sms:verification_failed", "格式"),
        ("sms:internal", "内部错误"),
    ];
    for (code, needle) in cases {
        let key =
            stable_code_text(code).unwrap_or_else(|| panic!("unlocalized stable code: {code}"));
        let text = template(Language::ZhCn, key);
        assert!(text.contains(needle), "{code} => {text:?} lacks {needle:?}");
    }
    // Unknown codes under the namespace keep a localized generic fallback.
    let fallback = stable_code_text("sms:future_code").expect("sms namespace fallback");
    assert_eq!(template(Language::ZhCn, fallback), "短信操作失败。");
}
