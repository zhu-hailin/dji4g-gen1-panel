use dji4g_at_protocol::{AtCommand, Effect, RetryPolicy, Sensitivity};

#[test]
fn sms_and_temperature_commands_have_exact_wire_bytes() {
    let cases = [
        (AtCommand::SmsMessageFormat, b"AT+CMGF?\r".as_slice()),
        (AtCommand::SmsSetPduMode, b"AT+CMGF=0\r".as_slice()),
        (AtCommand::SmsStorageQuery, b"AT+CPMS?\r".as_slice()),
        (AtCommand::SmsList, b"AT+CMGL=4\r".as_slice()),
        (AtCommand::SmsRead { index: 3 }, b"AT+CMGR=3\r".as_slice()),
        (
            AtCommand::SmsRead { index: 9999 },
            b"AT+CMGR=9999\r".as_slice(),
        ),
        (AtCommand::SmsDelete { index: 5 }, b"AT+CMGD=5\r".as_slice()),
        (AtCommand::Temperature, b"AT+QTEMP\r".as_slice()),
    ];

    for (command, expected) in cases {
        let encoded = command.encode();
        assert_eq!(encoded.as_bytes(), expected, "command: {command:?}");
        assert_eq!(
            encoded.as_bytes().iter().filter(|&&b| b == b'\r').count(),
            1
        );
        assert!(!encoded.as_bytes().contains(&b'\n'));
    }
}

#[test]
fn sms_command_effects_and_sensitivities_match_the_contract() {
    assert_eq!(AtCommand::SmsMessageFormat.effect(), Effect::PureRead);
    assert_eq!(
        AtCommand::SmsMessageFormat.sensitivity(),
        Sensitivity::Public
    );
    assert_eq!(AtCommand::SmsSetPduMode.effect(), Effect::SessionSetting);
    assert_eq!(
        AtCommand::SmsSetPduMode.sensitivity(),
        Sensitivity::MessageContent
    );
    assert_eq!(AtCommand::SmsStorageQuery.effect(), Effect::PureRead);
    assert_eq!(
        AtCommand::SmsStorageQuery.sensitivity(),
        Sensitivity::Public
    );
    assert_eq!(AtCommand::SmsList.effect(), Effect::PureRead);
    assert_eq!(
        AtCommand::SmsList.sensitivity(),
        Sensitivity::MessageContent
    );
    assert_eq!(
        AtCommand::SmsRead { index: 1 }.effect(),
        Effect::ReadMayMarkRead
    );
    assert_eq!(
        AtCommand::SmsRead { index: 1 }.sensitivity(),
        Sensitivity::MessageContent
    );
    assert_eq!(
        AtCommand::SmsDelete { index: 1 }.effect(),
        Effect::SessionSetting
    );
    assert_eq!(
        AtCommand::SmsDelete { index: 1 }.sensitivity(),
        Sensitivity::MessageContent
    );
    assert_eq!(AtCommand::Temperature.effect(), Effect::PureRead);
    assert_eq!(AtCommand::Temperature.sensitivity(), Sensitivity::Public);
}

#[test]
fn sms_retry_policies_never_retry_state_changing_transactions() {
    let never = [
        AtCommand::SmsSetPduMode,
        AtCommand::SmsList,
        AtCommand::SmsRead { index: 2 },
        AtCommand::SmsDelete { index: 2 },
    ];
    for command in never {
        assert_eq!(command.retry_policy(), RetryPolicy::Never, "{command:?}");
    }

    assert!(AtCommand::SmsSetPduMode.is_write());
    assert!(AtCommand::SmsDelete { index: 2 }.is_write());
    assert!(!AtCommand::SmsList.is_write());
    assert!(!AtCommand::SmsRead { index: 2 }.is_write());
    assert!(!AtCommand::SmsMessageFormat.is_write());
    assert!(!AtCommand::SmsStorageQuery.is_write());
    assert!(!AtCommand::Temperature.is_write());

    assert_eq!(
        AtCommand::SmsMessageFormat.retry_policy(),
        RetryPolicy::OnceAfterQuietPeriod
    );
    assert_eq!(
        AtCommand::SmsStorageQuery.retry_policy(),
        RetryPolicy::OnceAfterQuietPeriod
    );
}

#[test]
fn sms_read_debug_exposes_only_the_storage_index() {
    let debug = format!("{:?}", AtCommand::SmsRead { index: 42 });
    assert!(debug.contains("SmsRead"));
    assert!(debug.contains("42"));
    let delete = format!("{:?}", AtCommand::SmsDelete { index: 7 });
    assert!(delete.contains("SmsDelete"));
    assert!(delete.contains("7"));
}
