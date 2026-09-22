//! User-triggered diagnostic export, assembled entirely by the UI from the current snapshot.
//!
//! The export is privacy-bounded by construction: only whitelisted fields are copied into the
//! two documents, the container/device-instance identifiers and the APN are replaced with fixed
//! markers, and both documents pass through [`crate::logging::redact_sensitive`] as a final
//! defense-in-depth pass. Carrier name, interface addresses, gateways, DNS servers, VID/PID,
//! and the AT COM port are core diagnostic payload and are deliberately included.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use dji4g_application::{
    ControllerSnapshot, DiagnosticCheckId, DiagnosticCheckState, UnexecutedReason,
};
use dji4g_domain::{
    AdapterState, AttachState, Availability, BoundDnsStatus, BoundPublicStatus, DefaultRouteOwner,
    FeatureStatus, Freshness, HotspotStatus, ProtocolCoverage, RegistrationState, SimState,
};
use serde::Serialize;

use crate::localization::{
    Language, LocalizedText, TextArgs, TextKey, adapter_state, attach_state, bound_dns_status,
    bound_public_status, default_route_owner, diagnostic_id, diagnostic_state, format_text_in,
    freshness_key, protocol_coverage, registration_state, sim_state, unexecuted_reason,
};
use crate::logging::redact_sensitive;

/// Fixed marker replacing the Windows container and device-instance identifiers. A short hash
/// was considered and rejected: even a prefix of a stable hardware identifier narrows the
/// search space, and support does not need it to correlate one exported report.
pub const REDACTED_DEVICE_ID: &str = "[REDACTED_DEVICE_ID]";
/// Fixed marker replacing the APN value.
pub const REDACTED_APN: &str = "[REDACTED_APN]";

pub const REPORT_FILE_NAME: &str = "diagnostics-report.txt";
pub const REPORT_JSON_FILE_NAME: &str = "diagnostics-report.json";

const HUMAN_TEMP_PREFIX: &str = ".diagnostics-report.txt.tmp";
const JSON_TEMP_PREFIX: &str = ".diagnostics-report.json.tmp";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticExport {
    pub human: String,
    pub json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportError {
    stable_code: &'static str,
    os_code: Option<u32>,
}

impl ExportError {
    /// The export directory is unknown because the Windows profile variables were unavailable
    /// when the panel started.
    pub const PATH_UNAVAILABLE: Self = Self {
        stable_code: "export:path_unavailable",
        os_code: None,
    };

    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        self.stable_code
    }

    #[must_use]
    pub const fn os_code(&self) -> Option<u32> {
        self.os_code
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.stable_code)
    }
}

impl std::error::Error for ExportError {}

/// Build both export documents from the snapshot the UI currently holds.
#[must_use]
pub fn build(snapshot: &ControllerSnapshot, now: SystemTime) -> DiagnosticExport {
    // The redaction pass is a backstop, not the primary boundary: the builders below already
    // emit markers instead of the raw values.
    DiagnosticExport {
        human: redact_sensitive(&build_human(snapshot, now)),
        json: redact_sensitive(&build_json(snapshot, now)),
    }
}

/// Write both export files into `exports_dir`, creating the directory when missing. Each file
/// is written through a temporary sibling and one platform atomic replace, so a failed export
/// can never leave a truncated report behind.
pub fn write_export(
    exports_dir: &Path,
    snapshot: &ControllerSnapshot,
    now: SystemTime,
) -> Result<(), ExportError> {
    let export = build(snapshot, now);
    fs::create_dir_all(exports_dir).map_err(|error| io_failure("export:write_failed", error))?;
    write_atomic(
        exports_dir,
        REPORT_FILE_NAME,
        HUMAN_TEMP_PREFIX,
        export.human.as_bytes(),
    )?;
    write_atomic(
        exports_dir,
        REPORT_JSON_FILE_NAME,
        JSON_TEMP_PREFIX,
        export.json.as_bytes(),
    )?;
    Ok(())
}

fn write_atomic(
    directory: &Path,
    file_name: &str,
    temp_prefix: &str,
    bytes: &[u8],
) -> Result<(), ExportError> {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!("{temp_prefix}.{}.{}", std::process::id(), counter));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_failure("export:write_failed", error))?;
    let write_result = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|error| io_failure("export:write_failed", error));
    // Close the handle before the replace: an open handle can make ReplaceFileW fail even when
    // every byte is already flushed and synced (same constraint as the config store).
    drop(file);
    let replace_result = write_result.and_then(|()| {
        dji4g_windows_platform::atomic_replace_file(&temporary, &directory.join(file_name)).map_err(
            |error| ExportError {
                stable_code: "export:write_failed",
                os_code: error.os_code,
            },
        )
    });
    if replace_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    replace_result
}

fn io_failure(code: &'static str, error: io::Error) -> ExportError {
    ExportError {
        stable_code: code,
        os_code: error
            .raw_os_error()
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn build_human(snapshot: &ControllerSnapshot, now: SystemTime) -> String {
    // The export ships in the only released language; `available_languages()` is closed on
    // zh-CN by design.
    let language = Language::ZhCn;
    let app = snapshot.app.as_ref();
    let availability = crate::ui::availability_vm(app, &snapshot.diagnostics, now, language);
    let text = |key: TextKey| LocalizedText::new(language, key).text;
    let not_available = || text(TextKey::ValueNotAvailable);

    let mut report = String::new();
    report.push_str("DJI 一代 4G 面板 · 诊断信息导出\n");
    report.push_str(&format!("生成时间（UTC）：{}\n", utc_timestamp(now)));
    report.push_str(&format!(
        "隐私说明：{}\n",
        text(TextKey::DiagnosticsExportRedactionNotice)
    ));

    report.push_str("\n【概览】\n");
    report.push_str(&format!("当前判定：{}\n", availability.title.text));
    report.push_str(&format!("{}\n", availability.reason.text));
    report.push_str(&format!("证据状态：{}\n", availability.freshness.text));
    report.push_str(&format!(
        "{}：{}\n",
        text(TextKey::FieldHotspot),
        crate::ui::hotspot_vm(app.hotspot, language).status.text
    ));

    report.push_str("\n【设备】\n");
    match app.device.as_ref() {
        Some(device) => {
            report.push_str(&format!(
                "{}：VID {:04X}、PID {:04X}\n",
                text(TextKey::FieldUsbIdentity),
                device.identity.vid,
                device.identity.pid
            ));
            report.push_str(&format!("容器 / 设备实例标识：{REDACTED_DEVICE_ID}\n"));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldProblemCode),
                device
                    .problem_code
                    .map_or_else(&not_available, |code| code.to_string())
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldAtPort),
                device.at_port.clone().unwrap_or_else(&not_available)
            ));
        }
        None => report.push_str(&format!(
            "{}：{}\n",
            text(TextKey::FieldUsbIdentity),
            not_available()
        )),
    }

    report.push_str("\n【蜂窝网络】\n");
    match app.cellular.as_ref() {
        Some(cellular) => {
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldSimState),
                LocalizedText::new(language, sim_state(cellular.sim)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldRegistration),
                LocalizedText::new(language, registration_state(cellular.registration)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldAttachState),
                LocalizedText::new(language, attach_state(cellular.attached)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldCarrier),
                cellular
                    .carrier
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(&not_available)
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldRadioAccessTechnology),
                cellular
                    .radio_access_technology
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(&not_available)
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldSignal),
                cellular
                    .signal_rssi_dbm
                    .map_or_else(&not_available, |rssi| format!("{rssi} dBm"))
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldApn),
                cellular
                    .apn
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map_or_else(&not_available, |_| REDACTED_APN.to_owned())
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldPdpAddress),
                cellular.pdp_address.clone().unwrap_or_else(&not_available)
            ));
        }
        None => report.push_str(&format!(
            "{}：{}\n",
            text(TextKey::FieldCarrier),
            not_available()
        )),
    }

    report.push_str("\n【Windows 网络】\n");
    match app.network.as_ref() {
        Some(network) => {
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldWindowsAddresses),
                joined_or_unavailable(&network.addresses, &text)
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldGateway),
                joined_or_unavailable(&network.gateways, &text)
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldDnsServers),
                joined_or_unavailable(&network.dns_servers, &text)
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldAdapter),
                LocalizedText::new(language, adapter_state(network.adapter_state)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldBoundPublicProbe),
                LocalizedText::new(language, bound_public_status(network.bound_public)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldBoundDnsProbe),
                LocalizedText::new(language, bound_dns_status(network.bound_dns)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldProtocolCoverage),
                LocalizedText::new(language, protocol_coverage(network.protocol_coverage)).text
            ));
            report.push_str(&format!(
                "{}：{}\n",
                text(TextKey::FieldDefaultRoute),
                LocalizedText::new(language, default_route_owner(network.system_default_route))
                    .text
            ));
        }
        None => report.push_str(&format!(
            "{}：{}\n",
            text(TextKey::FieldAdapter),
            not_available()
        )),
    }

    report.push_str("\n【短信】\n");
    if let Some(send) = &snapshot.sms_send {
        report.push_str(&format!(
            "发送请求：{}；阶段：{:?}；结果：{:?}\n",
            send.request_id, send.phase, send.result
        ));
        if let Some(detail) = &send.failure {
            report.push_str(&format!(
                "发送错误：{}；CMS：{:?}；CME：{:?}；系统错误：{:?}；可能已提交：{}\n",
                detail.code,
                detail.cms_code,
                detail.cme_code,
                detail.os_code,
                detail.submission_possible
            ));
        }
    }
    // Aggregate counts only: message bodies and senders live in the application store and are
    // deliberately absent from both documents (§6.2, §8.4).
    report.push_str(&format!(
        "{}：{}\n",
        text(TextKey::FieldSmsStatus),
        crate::ui::sms::sms_vm(snapshot, &[], language).status.text
    ));
    report.push_str(&format!(
        "{}：{}\n",
        text(TextKey::FieldSmsMessageCount),
        snapshot.sms_inbox.message_count
    ));
    report.push_str(&format!(
        "{}：{}\n",
        text(TextKey::FieldSmsUnreadCount),
        snapshot.sms_inbox.unread_count
    ));
    report.push_str(&format!(
        "{}：{}\n",
        text(TextKey::FieldSmsCapacity),
        snapshot
            .sms_inbox
            .capacity
            .map_or_else(&not_available, |(used, total)| {
                format_text_in(
                    language,
                    TextKey::SmsCapacityUsed,
                    &TextArgs::used_total(used, total),
                )
                .text
            })
    ));
    if snapshot.sms_inbox.has_incomplete {
        report.push_str(&format!("{}\n", text(TextKey::SmsIncompleteWarning)));
    }

    report.push_str("\n【连接证据】\n");
    for id in DiagnosticCheckId::ORDERED {
        let check = snapshot.diagnostics.get(id);
        report.push_str(&format!(
            "{}（{}）：{} · {}\n",
            LocalizedText::new(language, diagnostic_id(check.id)).text,
            check_id_name(check.id),
            check_state_text(&check.state, language),
            LocalizedText::new(language, freshness_key(check.freshness(now))).text
        ));
    }
    report
}

fn joined_or_unavailable(values: &[String], text: &dyn Fn(TextKey) -> String) -> String {
    if values.is_empty() {
        text(TextKey::ValueNotAvailable)
    } else {
        values.join("、")
    }
}

fn check_state_text(state: &DiagnosticCheckState, language: Language) -> String {
    let label = LocalizedText::new(language, diagnostic_state(state)).text;
    match state {
        DiagnosticCheckState::Unexecuted { reason } => format!(
            "{label}（{}）",
            LocalizedText::new(language, unexecuted_reason(*reason)).text
        ),
        DiagnosticCheckState::Failed { code } | DiagnosticCheckState::Unavailable { code } => {
            format!("{label}（代码 {}）", code.stable().as_str())
        }
        DiagnosticCheckState::Running { .. }
        | DiagnosticCheckState::Passed
        | DiagnosticCheckState::Expired => label,
    }
}

/// Stable ASCII name of each check, mirrored into both documents. The closed match keeps the
/// exported identifiers pinned to the same closed set as `DiagnosticCheckId::ORDERED`.
#[must_use]
pub const fn check_id_name(id: DiagnosticCheckId) -> &'static str {
    match id {
        DiagnosticCheckId::UsbDevice => "usb_device",
        DiagnosticCheckId::AtControl => "at_control",
        DiagnosticCheckId::Cellular => "cellular",
        DiagnosticCheckId::WindowsAdapter => "windows_adapter",
        DiagnosticCheckId::BoundGateway => "bound_gateway",
        DiagnosticCheckId::BoundPublic => "bound_public",
        DiagnosticCheckId::BoundDns => "bound_dns",
        DiagnosticCheckId::SystemRoute => "system_route",
        DiagnosticCheckId::Hotspot => "hotspot",
    }
}

const fn unexecuted_reason_name(reason: UnexecutedReason) -> &'static str {
    match reason {
        UnexecutedReason::DisabledBySetting => "disabled_by_setting",
        UnexecutedReason::NotScheduled => "not_scheduled",
        UnexecutedReason::Superseded => "superseded",
    }
}

const fn check_state_name(state: &DiagnosticCheckState) -> &'static str {
    match state {
        DiagnosticCheckState::Unexecuted { .. } => "unexecuted",
        DiagnosticCheckState::Running { .. } => "running",
        DiagnosticCheckState::Passed => "passed",
        DiagnosticCheckState::Failed { .. } => "failed",
        DiagnosticCheckState::Unavailable { .. } => "unavailable",
        DiagnosticCheckState::Expired => "expired",
    }
}

/// Stable machine name for the inbox probe classification; the export must stay parseable without
/// depending on debug/serde spellings.
const fn feature_status_name(status: FeatureStatus) -> &'static str {
    match status {
        FeatureStatus::NotProbed => "not_probed",
        FeatureStatus::Supported => "supported",
        FeatureStatus::Empty => "empty",
        FeatureStatus::UnsupportedConfirmed => "unsupported_confirmed",
        FeatureStatus::TemporarilyUnavailable => "temporarily_unavailable",
        FeatureStatus::FormatMismatch => "format_mismatch",
        FeatureStatus::TransportFailure => "transport_failure",
    }
}

#[derive(Serialize)]
struct ExportDocumentV1 {
    schema_version: u32,
    generated_at_utc: String,
    availability: Availability,
    hotspot: HotspotStatus,
    device: Option<DeviceExportV1>,
    cellular: Option<CellularExportV1>,
    network: Option<NetworkExportV1>,
    /// Aggregate inbox counts only; message content never enters the export.
    sms: SmsExportV1,
    checks: Vec<CheckExportV1>,
}

#[derive(Serialize)]
struct SmsExportV1 {
    send: Option<dji4g_application::SmsSendSnapshot>,
    inbox_error: Option<String>,
    inbox_os_code: Option<u32>,
    message_count: usize,
    unread_count: usize,
    capacity: Option<(u32, u32)>,
    status: &'static str,
    has_incomplete: bool,
    evicted: u32,
}

#[derive(Serialize)]
struct DeviceExportV1 {
    vid: u16,
    pid: u16,
    // The container and device-instance identifiers are stable hardware identifiers outside the
    // export whitelist; the marker documents their presence without revealing them.
    device_identity: &'static str,
    problem_code: Option<u32>,
    at_port: Option<String>,
}

#[derive(Serialize)]
struct CellularExportV1 {
    sim: SimState,
    registration: RegistrationState,
    attached: AttachState,
    carrier: Option<String>,
    radio_access_technology: Option<String>,
    signal_rssi_dbm: Option<i16>,
    // Presence is kept (it is diagnostically useful), only the value is withheld.
    apn: Option<&'static str>,
    pdp_address: Option<String>,
}

#[derive(Serialize)]
struct NetworkExportV1 {
    addresses: Vec<String>,
    gateways: Vec<String>,
    dns_servers: Vec<String>,
    adapter_state: AdapterState,
    bound_public: BoundPublicStatus,
    bound_dns: BoundDnsStatus,
    protocol_coverage: ProtocolCoverage,
    system_default_route: DefaultRouteOwner,
    down_bytes_per_sec: Option<u64>,
    up_bytes_per_sec: Option<u64>,
}

#[derive(Serialize)]
struct CheckExportV1 {
    id: &'static str,
    state: CheckStateExportV1,
    freshness: Freshness,
}

#[derive(Serialize)]
struct CheckStateExportV1 {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unexecuted_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_code: Option<String>,
}

fn build_json(snapshot: &ControllerSnapshot, now: SystemTime) -> String {
    let app = snapshot.app.as_ref();
    let document = ExportDocumentV1 {
        schema_version: 1,
        generated_at_utc: utc_timestamp(now),
        availability: app.availability,
        hotspot: app.hotspot,
        device: app.device.as_ref().map(|device| DeviceExportV1 {
            vid: device.identity.vid,
            pid: device.identity.pid,
            device_identity: REDACTED_DEVICE_ID,
            problem_code: device.problem_code,
            at_port: device.at_port.clone(),
        }),
        cellular: app.cellular.as_ref().map(|cellular| CellularExportV1 {
            sim: cellular.sim,
            registration: cellular.registration,
            attached: cellular.attached,
            carrier: cellular.carrier.clone(),
            radio_access_technology: cellular.radio_access_technology.clone(),
            signal_rssi_dbm: cellular.signal_rssi_dbm,
            apn: cellular
                .apn
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|_| REDACTED_APN),
            pdp_address: cellular.pdp_address.clone(),
        }),
        network: app.network.as_ref().map(|network| NetworkExportV1 {
            addresses: network.addresses.clone(),
            gateways: network.gateways.clone(),
            dns_servers: network.dns_servers.clone(),
            adapter_state: network.adapter_state,
            bound_public: network.bound_public,
            bound_dns: network.bound_dns,
            protocol_coverage: network.protocol_coverage,
            system_default_route: network.system_default_route,
            down_bytes_per_sec: network.down_bytes_per_sec,
            up_bytes_per_sec: network.up_bytes_per_sec,
        }),
        sms: SmsExportV1 {
            send: snapshot.sms_send.clone(),
            inbox_error: snapshot
                .sms_inbox_failure
                .as_ref()
                .map(|e| e.code.stable().as_str().to_owned()),
            inbox_os_code: snapshot.sms_inbox_failure.as_ref().and_then(|e| e.os_code),
            message_count: snapshot.sms_inbox.message_count,
            unread_count: snapshot.sms_inbox.unread_count,
            capacity: snapshot.sms_inbox.capacity,
            status: feature_status_name(snapshot.sms_inbox.status),
            has_incomplete: snapshot.sms_inbox.has_incomplete,
            evicted: snapshot.sms_inbox.evicted,
        },
        checks: DiagnosticCheckId::ORDERED
            .iter()
            .map(|id| {
                let check = snapshot.diagnostics.get(*id);
                CheckExportV1 {
                    id: check_id_name(check.id),
                    state: CheckStateExportV1 {
                        status: check_state_name(&check.state),
                        unexecuted_reason: match &check.state {
                            DiagnosticCheckState::Unexecuted { reason } => {
                                Some(unexecuted_reason_name(*reason))
                            }
                            _ => None,
                        },
                        stable_code: match &check.state {
                            DiagnosticCheckState::Failed { code }
                            | DiagnosticCheckState::Unavailable { code } => {
                                Some(code.stable().as_str().to_owned())
                            }
                            _ => None,
                        },
                    },
                    freshness: check.freshness(now),
                }
            })
            .collect(),
    };
    // These whitelist structs contain no maps or floats, so serialization cannot fail; the
    // expectation documents that invariant instead of inventing a third stable code.
    serde_json::to_string_pretty(&document).expect("export document serializes")
}

/// UTC ISO-8601 stamp without pulling a date-time dependency into the panel boundary.
fn utc_timestamp(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

/// Inverse of Howard Hinnant's `days_from_civil`; valid for every second since the epoch that
/// `SystemTime` can represent.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_pattern = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_pattern + 2) / 5 + 1) as u32;
    let month = if month_pattern < 10 {
        month_pattern + 3
    } else {
        month_pattern - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    use dji4g_application::{
        CommandStateSnapshot, DeviceEpoch, DiagnosticSet, ErrorCode, PortError, SettingsSnapshot,
    };
    use dji4g_domain::{
        AppSnapshot, CellularSnapshot, DeviceSnapshot, NetworkSnapshot, NumberLookup, PhoneNumber,
        SimIdentity, StableDeviceIdentity,
    };

    const CONTAINER_ID: &str = "container-9f3a1c7e5b2d";
    const DEVICE_INSTANCE_ID: &str = r"USB\VID_2CA3&PID_4006\5&1a2b3c4d&0&2";
    const APN: &str = "internet.carrier-apn.example";
    const DEVICE_ADAPTER_ID: &str = "device-adapter-internal-id";
    const NETWORK_ADAPTER_ID: &str = "network-adapter-internal-id";
    const NOW_SECS: u64 = 1_700_000_000;

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(NOW_SECS)
    }

    fn fixture() -> ControllerSnapshot {
        ControllerSnapshot {
            publication_revision: 7,
            app: Arc::new(AppSnapshot {
                revision: 7,
                observed_at: UNIX_EPOCH + Duration::from_secs(1_699_999_970),
                freshness: Freshness::Fresh,
                availability: Availability::Available,
                hotspot: HotspotStatus::Off,
                device: Some(DeviceSnapshot {
                    epoch: DeviceEpoch(3),
                    identity: StableDeviceIdentity {
                        container_id: CONTAINER_ID.to_owned(),
                        device_instance_id: DEVICE_INSTANCE_ID.to_owned(),
                        vid: 0x2CA3,
                        pid: 0x4006,
                    },
                    problem_code: Some(28),
                    at_port: Some("COM3".to_owned()),
                    adapter_id: Some(DEVICE_ADAPTER_ID.to_owned()),
                }),
                cellular: Some(CellularSnapshot {
                    sim: SimState::Ready,
                    registration: RegistrationState::RegisteredHome,
                    attached: AttachState::Attached,
                    carrier: Some("中国移动".to_owned()),
                    radio_access_technology: Some("LTE".to_owned()),
                    signal_rssi_dbm: Some(-71),
                    apn: Some(APN.to_owned()),
                    pdp_address: Some("10.11.12.13".to_owned()),
                    firmware: Some("EC200A".to_owned()),
                    pdp_state: Some("active".to_owned()),
                    serving_cell: None,
                    sim_identity: None,
                    numbers: None,
                    temperature_celsius: None,
                    temperature_status: dji4g_domain::FeatureStatus::NotProbed,
                }),
                network: Some(NetworkSnapshot {
                    adapter_id: NETWORK_ADAPTER_ID.to_owned(),
                    addresses: vec!["192.168.42.11".to_owned()],
                    gateways: vec!["192.168.42.1".to_owned()],
                    dns_servers: vec!["192.168.42.1".to_owned()],
                    adapter_state: AdapterState::UsableAddressAndRoute,
                    bound_public: BoundPublicStatus::Succeeded,
                    bound_dns: BoundDnsStatus::Succeeded,
                    protocol_coverage: ProtocolCoverage::AllRequiredFamilies,
                    system_default_route: DefaultRouteOwner::TargetAdapter,
                    down_bytes_per_sec: None,
                    up_bytes_per_sec: None,
                }),
                active_operation: None,
                issues: Vec::new(),
            }),
            diagnostics: DiagnosticSet::new(DeviceEpoch(3)),
            prepared_action: None,
            operation: None,
            settings: SettingsSnapshot::default(),
            command_state: CommandStateSnapshot::default(),
            action_readiness: Vec::new(),
            feedback: None,
            sim_epoch: 0,
            feature_status: None,
            adapter_metrics: None,
            timeline: Default::default(),
            sms_inbox: Default::default(),
            sms_messages: Vec::new(),
            sms_delete: None,
            serial_work_busy: false,
            sms_send: None,
            sms_refresh_pending: false,
            sms_inbox_failure: None,
            device_tools: Default::default(),
        }
    }

    #[test]
    fn report_contains_all_nine_check_labels_ids_and_default_states() {
        let export = build(&fixture(), now());
        for id in DiagnosticCheckId::ORDERED {
            let label = LocalizedText::new(Language::ZhCn, diagnostic_id(id)).text;
            assert!(
                export.human.contains(&label),
                "human report is missing the label for {id:?}"
            );
            assert!(
                export.human.contains(check_id_name(id)),
                "human report is missing the id for {id:?}"
            );
            assert!(
                export.json.contains(check_id_name(id)),
                "json report is missing the id for {id:?}"
            );
        }
        // The fixture set has never executed a check, so all nine rows carry that state (the
        // state label is repeated inside the unexecuted reason, hence the line-based count).
        assert_eq!(
            export
                .human
                .lines()
                .filter(|line| line.contains("：未执行（"))
                .count(),
            9
        );
        let parsed: serde_json::Value = serde_json::from_str(&export.json).expect("json parses");
        assert_eq!(parsed["checks"].as_array().expect("checks array").len(), 9);
    }

    #[test]
    fn export_includes_the_allowed_core_payload() {
        let export = build(&fixture(), now());
        for document in [&export.human, &export.json] {
            assert!(document.contains("中国移动"), "carrier must be included");
            assert!(
                document.contains("192.168.42.11"),
                "interface addresses must be included"
            );
            assert!(
                document.contains("192.168.42.1"),
                "gateway and DNS values must be included"
            );
            assert!(document.contains("COM3"), "AT port must be included");
            assert!(
                document.contains("10.11.12.13"),
                "PDP address must be included"
            );
        }
        // The human report prints the USB identity in hex; the JSON document keeps the raw
        // numbers.
        assert!(export.human.contains("VID 2CA3、PID 4006"));
        let parsed: serde_json::Value = serde_json::from_str(&export.json).expect("json parses");
        assert_eq!(parsed["device"]["vid"], 0x2CA3);
        assert_eq!(parsed["device"]["pid"], 0x4006);
    }

    #[test]
    fn export_never_contains_sensitive_identifier_values() {
        let export = build(&fixture(), now());
        for document in [&export.human, &export.json] {
            assert!(!document.contains(CONTAINER_ID));
            assert!(!document.contains(DEVICE_INSTANCE_ID));
            assert!(!document.contains(APN));
            // Whitelist discipline: internal adapter identifiers never enter the export either.
            assert!(!document.contains(DEVICE_ADAPTER_ID));
            assert!(!document.contains(NETWORK_ADAPTER_ID));
            assert!(document.contains(REDACTED_DEVICE_ID));
            assert!(document.contains(REDACTED_APN));
        }
    }

    #[test]
    fn export_never_carries_phone_numbers_or_the_iccid_even_in_masked_form() {
        // The optional CNUM/ICCID probes feed the identity area but the export whitelist must
        // never grow to include them: neither the plaintext number, its mask, nor the masked
        // ICCID may appear in either document (research §8.4/§10).
        let mut snapshot = fixture();
        let mut cellular = snapshot
            .app
            .cellular
            .clone()
            .expect("fixture has cellular evidence");
        cellular.numbers = Some(NumberLookup::Reported(vec![
            PhoneNumber::new("+8613800138000", 145),
            PhoneNumber::new("+8613900138000", 129),
        ]));
        cellular.sim_identity = Some(SimIdentity {
            iccid_masked: "8986…2345".to_owned(),
            fingerprint: [1; 8],
        });
        snapshot.app = Arc::new({
            let mut app = (*snapshot.app).clone();
            app.cellular = Some(cellular);
            app
        });
        let export = build(&snapshot, now());
        for document in [&export.human, &export.json] {
            assert!(
                !document.contains("8613800138000"),
                "plaintext number leaked"
            );
            assert!(
                !document.contains("8613900138000"),
                "second plaintext number leaked"
            );
            assert!(
                !document.contains("****8000"),
                "masked number must not be exported"
            );
            assert!(
                !document.contains("8986"),
                "ICCID (masked or not) must not be exported"
            );
            assert!(!document.contains("89860123456789012345"));
        }
    }

    #[test]
    fn json_round_trips_through_serde_json() {
        let export = build(&fixture(), now());
        let parsed: serde_json::Value = serde_json::from_str(&export.json).expect("json parses");
        let reparsed = serde_json::to_string(&parsed).expect("re-serialize");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&reparsed).expect("round trip parses"),
            parsed
        );
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["device"]["vid"], 0x2CA3);
        assert_eq!(parsed["device"]["device_identity"], REDACTED_DEVICE_ID);
        assert_eq!(parsed["cellular"]["apn"], REDACTED_APN);
        assert_eq!(parsed["generated_at_utc"], "2023-11-14T22:13:20Z");
    }

    #[test]
    fn export_carries_sms_counts_only_and_never_message_content() {
        let mut snapshot = fixture();
        snapshot.sms_inbox = dji4g_domain::SmsInboxSummary {
            message_count: 2,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: dji4g_domain::FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 0,
        };
        let export = build(&snapshot, now());
        assert!(export.human.contains("消息数：2"));
        assert!(export.human.contains("未读数：1"));
        assert!(export.human.contains("已用 2 / 总数 30"));
        assert!(export.human.contains("存在未完整接收的长短信"));
        let parsed: serde_json::Value = serde_json::from_str(&export.json).expect("json parses");
        assert_eq!(parsed["sms"]["message_count"], 2);
        assert_eq!(parsed["sms"]["unread_count"], 1);
        assert_eq!(parsed["sms"]["capacity"][0], 2);
        assert_eq!(parsed["sms"]["capacity"][1], 30);
        assert_eq!(parsed["sms"]["status"], "supported");
        assert_eq!(parsed["sms"]["has_incomplete"], true);
        for document in [&export.human, &export.json] {
            assert!(
                !document.contains("\"sender\""),
                "the export must never gain a sender field"
            );
            assert!(
                !document.contains("\"body\""),
                "the export must never gain a body field"
            );
            assert!(!document.contains("短信正文"));
            assert!(!document.contains("发送方"));
        }
    }

    #[test]
    fn export_keeps_send_failure_evidence_without_any_payload() {
        let mut snapshot = fixture();
        let mut detail = dji4g_application::SmsFailureDetail::new(
            dji4g_application::SmsSendPhase::WaitingForResult,
            "sms:module_rejected",
            true,
        );
        detail.cms_code = Some(500);
        snapshot.sms_send = Some(dji4g_application::SmsSendSnapshot {
            request_id: 42,
            phase: dji4g_application::SmsSendPhase::Finished,
            result: Some(dji4g_application::SmsSendResult::Failed),
            failure: Some(detail),
        });
        let export = build(&snapshot, now());
        let parsed: serde_json::Value = serde_json::from_str(&export.json).unwrap();
        assert_eq!(parsed["sms"]["send"]["request_id"], 42);
        assert_eq!(parsed["sms"]["send"]["failure"]["cms_code"], 500);
        assert!(export.human.contains("sms:module_rejected"));
        assert!(!export.json.contains("recipient"));
        assert!(!export.json.contains("body"));
        assert!(!export.json.contains("pdu"));
    }

    #[test]
    fn sms_probe_classifications_have_stable_machine_names() {
        assert_eq!(feature_status_name(FeatureStatus::NotProbed), "not_probed");
        assert_eq!(feature_status_name(FeatureStatus::Supported), "supported");
        assert_eq!(feature_status_name(FeatureStatus::Empty), "empty");
        assert_eq!(
            feature_status_name(FeatureStatus::UnsupportedConfirmed),
            "unsupported_confirmed"
        );
        assert_eq!(
            feature_status_name(FeatureStatus::TemporarilyUnavailable),
            "temporarily_unavailable"
        );
        assert_eq!(
            feature_status_name(FeatureStatus::FormatMismatch),
            "format_mismatch"
        );
        assert_eq!(
            feature_status_name(FeatureStatus::TransportFailure),
            "transport_failure"
        );
    }

    #[test]
    fn check_states_map_to_stable_machine_names() {
        let failed = DiagnosticCheckState::Failed {
            code: PortError::new(ErrorCode::ProbeFailed, "probe:connect_failed").code,
        };
        let unavailable = DiagnosticCheckState::Unavailable {
            code: PortError::new(ErrorCode::DnsFailed, "probe:dns_failed").code,
        };
        assert_eq!(check_state_name(&failed), "failed");
        assert_eq!(check_state_name(&unavailable), "unavailable");
        assert_eq!(
            check_state_name(&DiagnosticCheckState::Unexecuted {
                reason: UnexecutedReason::DisabledBySetting
            }),
            "unexecuted"
        );
        assert_eq!(
            unexecuted_reason_name(UnexecutedReason::DisabledBySetting),
            "disabled_by_setting"
        );
        let DiagnosticCheckState::Failed { code } = &failed else {
            unreachable!("constructed as Failed above");
        };
        assert_eq!(code.stable().as_str(), "probe:connect_failed");
    }

    #[test]
    fn utc_timestamp_formats_known_instants() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_000_000_000)),
            "2001-09-09T01:46:40Z"
        );
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        // A clock set before the epoch has no meaningful stamp; the helper clamps instead of
        // panicking so an export can never fail on it.
        assert_eq!(
            utc_timestamp(UNIX_EPOCH - Duration::from_secs(1)),
            "1970-01-01T00:00:00Z"
        );
    }
}
