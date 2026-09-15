use std::fs;

use dji4g_panel::logging::{LoggingConfig, RollingLog, redact_sensitive};

#[test]
fn sms_state_events_reach_the_real_rolling_log() {
    let root = std::env::temp_dir().join(format!(
        "dji4g-sms-log-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let guard = dji4g_panel::logging::init_logging(LoggingConfig {
        directory: root,
        ..LoggingConfig::default()
    })
    .unwrap();
    let mut failure = dji4g_application::SmsFailureDetail::new(
        dji4g_application::SmsSendPhase::WaitingForResult,
        "sms:module_rejected",
        true,
    );
    failure.cms_code = Some(500);
    dji4g_panel::logging::record_sms_state(&dji4g_application::SmsSendSnapshot {
        request_id: 42,
        phase: dji4g_application::SmsSendPhase::Finished,
        result: Some(dji4g_application::SmsSendResult::Failed),
        failure: Some(failure),
    });
    let content = fs::read_to_string(guard.active_path()).unwrap();
    assert!(content.contains("sms_state"));
    assert!(content.contains("\"request_id\":42"));
    assert!(content.contains("\"cms_code\":500"));
    assert!(!content.contains("recipient"));
    assert!(!content.contains("body"));
}

#[test]
fn sensitive_values_are_redacted_before_log_bytes_are_written() {
    let input = r#"imei=867530900123456 apn=internet iccid=8986001234567890123 phone=13800138000 ssid=home body=AT+CGSN\r\n secret"#;
    let output = redact_sensitive(input);
    assert!(!output.contains("867530900123456"));
    assert!(!output.contains("internet"));
    assert!(!output.contains("8986001234567890123"));
    assert!(!output.contains("13800138000"));
    assert!(!output.contains("AT+CGSN"));
}

#[test]
fn rolling_log_obeys_file_count_and_total_byte_caps() {
    let root = std::env::temp_dir().join(format!("dji4g-panel-log-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let config = LoggingConfig {
        directory: root.clone(),
        max_file_bytes: 32,
        max_files: 3,
        max_total_bytes: 64,
        level: dji4g_application::LogLevel::Info,
    };
    let mut log = RollingLog::new(config).expect("create log");
    for _ in 0..20 {
        log.append("event=ok imei=867530900123456\n")
            .expect("append");
    }
    let mut total = 0_u64;
    let mut count = 0;
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("dji4g-panel")
        {
            count += 1;
            total += entry.metadata().unwrap().len();
        }
    }
    assert!(count <= 3);
    assert!(total <= 64);
    let content = fs::read_to_string(log.active_path()).unwrap();
    assert!(!content.contains("867530900123456"));
}
