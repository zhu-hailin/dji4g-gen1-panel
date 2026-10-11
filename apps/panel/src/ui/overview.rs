//! Overview page: the read-only answer to 「现在能用吗」.
//!
//! The title, verdict banner and summary line span the full width; below them the cards pair up —
//! the measured rate trend beside the device's own record, then the Windows network facts — so a
//! desktop-width window shows the whole state without scrolling.
//!
//! Nothing on this page acts. It reports a value, a curve, a row of facts or a timeline entry, and
//! that is all: a check, a retry, a switch, a repair or an export belongs to the page that owns the
//! subject, and lives there. The one exception is the panel-wide 刷新 / 更多 pair in the header,
//! which is deliberately the overview's own.

use std::time::{Duration, Instant};

use dji4g_application::{AdapterMetrics, ControllerSnapshot};
use dji4g_domain::{NumberLookup, Timeline};
use eframe::egui::{self, RichText, Ui};

use super::{
    DisplayValue, carrier_display_name, clock_hms, field_label, format_age, format_mbps, info_grid,
    meta_text, render_rate_section, scale, section_frame, section_heading, speed_grade,
    two_columns, wrapped_label,
};
use crate::feature_probe::FeatureProbeView;
use crate::localization::{
    Language, LocalizedText, TextArgs, TextKey, bound_dns_status, default_route_owner,
    feature_status_note, format_text_in, registration_state, sim_state, timeline_detail_text,
};

/// One catalog string in the language this page was rendered with.
fn t(language: crate::localization::Language, key: crate::localization::TextKey) -> String {
    crate::localization::LocalizedText::new(language, key).text
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewVm {
    pub question: LocalizedText,
    pub device: DisplayValue,
    pub carrier: DisplayValue,
    pub radio_access_technology: DisplayValue,
    pub signal: DisplayValue,
    pub registration: LocalizedText,
    pub sim: LocalizedText,
    pub adapter_addresses: DisplayValue,
    pub addresses: Vec<String>,
    pub dns: LocalizedText,
    pub route: LocalizedText,
    pub down_rate: Option<u64>,
    pub up_rate: Option<u64>,
    pub firmware: DisplayValue,
    pub pdp: DisplayValue,
    pub serving_cell: DisplayValue,
    pub serving_cell_note: Option<LocalizedText>,
    pub serving_cell_raw: Option<String>,
    pub serving_cell_provisional: bool,
    pub temperature: DisplayValue,
    pub temperature_note: Option<LocalizedText>,
    /// The numeric reading behind [`Self::temperature`], for the trend section's delta.
    pub temperature_celsius: Option<i16>,
    /// How many channels the firmware reported and in which order — only for a correlated
    /// observation that reported more than one reading.
    pub temperature_sensors_note: Option<LocalizedText>,
    /// Raw `+QTEMP:` line of the correlated observation, shown on hover so a layout this build
    /// cannot read is still visible to the user instead of being hidden behind 「格式不匹配」.
    pub temperature_raw: Option<String>,
    pub adapter_metrics: AdapterMetricsVm,
    pub timeline: Vec<TimelineRowVm>,
    pub identity: IdentityVm,
}

/// Projection of the bound-adapter interface metrics (research §7.1). Counters are per-interface
/// totals and the link rates are negotiated interface speeds — never measured throughput — so the
/// view keeps a fixed note next to the link row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterMetricsVm {
    pub errors: DisplayValue,
    pub discards: DisplayValue,
    pub link_rate: DisplayValue,
    pub link_note: Option<LocalizedText>,
}

/// One observed transition as the overview renders it: wall-clock time plus the event's closed
/// detail phrase (falling back to the localized category when a producer has no detail).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineRowVm {
    pub time: Option<String>,
    pub text: LocalizedText,
}

/// How many newest timeline entries the overview shows; the store keeps the full bounded ring.
pub const TIMELINE_VISIBLE: usize = 10;

/// How many reported sensor values the 温度 note lists before it marks the list as cut.  The raw
/// `+QTEMP:` line stays available on hover, so a long list never has to be guessed at.
pub const MAX_LISTED_TEMPERATURE_VALUES: usize = 6;

/// View data for the 「身份信息」 area (research §4.1/§4.3): reported phone numbers and the SIM
/// ICCID, both defaulting to masked display with explicit user-action reveal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityVm {
    /// Masked form of every reported number, in report order.
    pub numbers: Vec<String>,
    /// Primary text of the 本机号码 row when there is nothing to show.
    pub number_absent: Option<LocalizedText>,
    /// Classified failure note (固件不支持/格式不匹配/本次超时/暂时不可用) for this row, only when
    /// the probe record provably belongs to the displayed snapshot.
    pub number_note: Option<LocalizedText>,
    /// Masked ICCID display form.
    pub iccid_masked: Option<String>,
    /// Plaintext ICCID of the correlated observation, handed out only so the [显示] click can
    /// reveal it.  UI-local; never serialized, logged or exported.
    pub iccid_reveal_full: Option<String>,
    pub iccid_note: Option<LocalizedText>,
}

#[must_use]
pub fn overview_vm(snapshot: &ControllerSnapshot, language: Language) -> OverviewVm {
    overview_vm_with_probes(snapshot, language, None)
}

#[must_use]
pub fn overview_vm_with_probes(
    snapshot: &ControllerSnapshot,
    language: Language,
    probes: Option<&FeatureProbeView>,
) -> OverviewVm {
    let app = snapshot.app.as_ref();
    let cellular = app.cellular.as_ref();
    let network = app.network.as_ref();
    let probes = probes.filter(|view| view.captured);
    let device = match app.device.as_ref() {
        // The 型号 row names the module the inventory actually proved, so a recognized generic
        // module is never labelled as a DJI one.
        Some(device) => device.identity.profile().map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::DeviceModelName)),
            |profile| {
                DisplayValue::new(t(language, crate::localization::device_model_name(profile)))
            },
        ),
        None => DisplayValue::new(t(
            language,
            crate::localization::TextKey::AvailabilityNotDetectedTitle,
        )),
    };
    // 网速分档与速率面板同源（`speed_grade`）：封闭阈值只在 ui/mod.rs 定义一次，文案走本地化
    // 词表。分档是展示词汇，不参与任何可用性判定。
    let (grade_key, _grade_tone) =
        speed_grade(network.and_then(|network| network.down_bytes_per_sec));
    let grade_text = LocalizedText::new(language, grade_key).text;
    let carrier = cellular
        .and_then(|value| value.carrier.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable)),
            |value| DisplayValue::new(carrier_display_name(value, language)),
        );
    let radio_access_technology = cellular
        .and_then(|value| value.radio_access_technology.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable)),
            DisplayValue::new,
        );
    let signal = cellular
        .and_then(|value| value.signal_rssi_dbm)
        .map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable)),
            |value| {
                let reading = value.to_string();
                DisplayValue::new(crate::localization::format_positional(
                    language,
                    crate::localization::TextKey::SignalWithGrade,
                    &[&reading, &grade_text],
                ))
            },
        );
    let registration = cellular.map_or_else(
        || LocalizedText::new(language, TextKey::ValueNotAvailable),
        |value| LocalizedText::new(language, registration_state(value.registration)),
    );
    let sim = cellular.map_or_else(
        || LocalizedText::new(language, TextKey::ValueNotAvailable),
        |value| LocalizedText::new(language, sim_state(value.sim)),
    );
    let firmware = cellular
        .and_then(|value| value.firmware.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable)),
            DisplayValue::new,
        );
    let pdp = cellular
        .and_then(|value| value.pdp_state.as_deref())
        .map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable)),
            |value| {
                DisplayValue::new(match value {
                    "active" => t(language, crate::localization::TextKey::PdpActive),
                    "inactive" => t(language, crate::localization::TextKey::PdpInactive),
                    other => other.to_owned(),
                })
            },
        );
    let serving_cell = cellular
        .and_then(|value| value.serving_cell.as_ref())
        .map_or_else(
            || DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable)),
            |cell| DisplayValue::new(serving_cell_text(cell, language)),
        );
    // The provisional-layout note is about parsing output, so it rides with the report itself.
    let serving_cell_provisional = cellular
        .and_then(|value| value.serving_cell.as_ref())
        .is_some();
    let serving_cell_note = probes
        .and_then(|view| feature_status_note(view.serving_cell_status))
        .map(|key| LocalizedText::new(language, key));
    let serving_cell_raw = probes.and_then(|view| view.serving_cell_raw.clone());
    let addresses = network
        .map(|network| network.addresses.clone())
        .unwrap_or_default();
    let adapter_addresses = if addresses.is_empty() {
        DisplayValue::new(t(language, crate::localization::TextKey::ValueNotAvailable))
    } else {
        DisplayValue::copyable(preview_values(&addresses, language))
    };
    let dns = network.map_or_else(
        || LocalizedText::new(language, TextKey::ValueNotAvailable),
        |network| LocalizedText::new(language, bound_dns_status(network.bound_dns)),
    );
    let route = network.map_or_else(
        || LocalizedText::new(language, TextKey::ValueNotAvailable),
        |network| LocalizedText::new(language, default_route_owner(network.system_default_route)),
    );
    let (temperature, temperature_note) = temperature_vm(cellular, language);
    let temperature_sensors_note = probes
        .map(|view| view.temperature_sensors.as_slice())
        .and_then(|readings| temperature_sensors_note(readings, language));
    OverviewVm {
        question: LocalizedText::new(language, TextKey::OverviewQuestion),
        device,
        carrier,
        radio_access_technology,
        signal,
        registration,
        sim,
        adapter_addresses,
        addresses,
        dns,
        route,
        down_rate: network.and_then(|network| network.down_bytes_per_sec),
        up_rate: network.and_then(|network| network.up_bytes_per_sec),
        firmware,
        pdp,
        serving_cell,
        serving_cell_note,
        serving_cell_raw,
        serving_cell_provisional,
        temperature,
        temperature_note,
        temperature_celsius: cellular.and_then(|value| value.temperature_celsius),
        temperature_sensors_note,
        temperature_raw: probes.and_then(|view| view.temperature_raw.clone()),
        adapter_metrics: adapter_metrics_vm(snapshot.adapter_metrics, language),
        timeline: timeline_rows(&snapshot.timeline, language),
        identity: identity_vm(cellular, probes, language),
    }
}

/// The 温度 row. A reading is shown verbatim with the fixed reminder that the sensor definition
/// belongs to the firmware; without a reading the row says exactly that, and keeps the probe's
/// classified failure note so 「未读取到」 and 「查询失败」 stay distinguishable.
#[must_use]
pub fn temperature_vm(
    cellular: Option<&dji4g_domain::CellularSnapshot>,
    language: Language,
) -> (DisplayValue, Option<LocalizedText>) {
    let Some(cellular) = cellular else {
        return (
            DisplayValue::new(LocalizedText::new(language, TextKey::ValueNotAvailable).text),
            None,
        );
    };
    match cellular.temperature_celsius {
        Some(celsius) => (
            DisplayValue::new(format!("{celsius} °C")),
            Some(LocalizedText::new(language, TextKey::TemperatureSensorNote)),
        ),
        None => {
            let note = feature_status_note(cellular.temperature_status)
                .map(|key| LocalizedText::new(language, key));
            (
                DisplayValue::new(LocalizedText::new(language, TextKey::TemperatureNotRead).text),
                note,
            )
        }
    }
}

/// How many sensor values the module reported with this reading, in report order.
///
/// The channel order and meaning belong to the firmware (§7.5), so the note says exactly what came
/// back and claims nothing about which sensor is which.  A single reading needs no list — the row
/// already shows it with the firmware-defined sensor note.
#[must_use]
pub fn temperature_sensors_note(
    readings: &[dji4g_at_protocol::SensorTemperature],
    language: Language,
) -> Option<LocalizedText> {
    if readings.len() < 2 {
        return None;
    }
    let values = readings
        .iter()
        .take(MAX_LISTED_TEMPERATURE_VALUES)
        .map(|reading| reading.celsius.to_string())
        .collect::<Vec<_>>()
        .join(" / ");
    // A cut list says so, instead of looking like the module reported exactly this many values.
    let values = if readings.len() > MAX_LISTED_TEMPERATURE_VALUES {
        format!("{values} / …")
    } else {
        values
    };
    Some(format_text_in(
        language,
        TextKey::TemperatureSensorsReported,
        &TextArgs {
            count: Some(readings.len()),
            detail: Some(values),
            ..TextArgs::default()
        },
    ))
}

/// The Windows-network interface metrics rows. `None` is an honest 未获取 on every row — never a
/// fabricated zero; the link-rate note only appears when a link rate is actually shown.
#[must_use]
pub fn adapter_metrics_vm(metrics: Option<AdapterMetrics>, language: Language) -> AdapterMetricsVm {
    let rx_tx = |rx: String, tx: String| {
        DisplayValue::new(
            format_text_in(language, TextKey::AdapterRxTx, &TextArgs::rx_tx(rx, tx)).text,
        )
    };
    let Some(metrics) = metrics else {
        let missing =
            DisplayValue::new(LocalizedText::new(language, TextKey::ValueNotAvailable).text);
        return AdapterMetricsVm {
            errors: missing.clone(),
            discards: missing.clone(),
            link_rate: missing,
            link_note: None,
        };
    };
    AdapterMetricsVm {
        errors: rx_tx(
            metrics.in_errors.to_string(),
            metrics.out_errors.to_string(),
        ),
        discards: rx_tx(
            metrics.in_discards.to_string(),
            metrics.out_discards.to_string(),
        ),
        link_rate: rx_tx(
            format_mbps(metrics.link_rx_bits_per_second),
            format_mbps(metrics.link_tx_bits_per_second),
        ),
        link_note: Some(LocalizedText::new(language, TextKey::AdapterLinkRateNote)),
    }
}

/// The newest [`TIMELINE_VISIBLE`] transitions, newest first. The row's wording is resolved here,
/// in the page's language, from the closed values the reducer recorded.
#[must_use]
pub fn timeline_rows(timeline: &Timeline, language: Language) -> Vec<TimelineRowVm> {
    timeline
        .events()
        .iter()
        .rev()
        .take(TIMELINE_VISIBLE)
        .map(|event| TimelineRowVm {
            time: clock_hms(event.at),
            text: timeline_detail_text(language, event.kind, event.detail),
        })
        .collect()
}

/// Build the 「身份信息」 rows from the snapshot (research §4.1/§4.3).
///
/// Numbers are masked unless an explicit user action reveals them (the reveal/copy handlers read
/// the plaintext straight from the snapshot, never through this view).  The classified probe
/// notes are only shown when the probe record provably belongs to the snapshot being rendered,
/// so a stale cycle can never label the current rows.
fn identity_vm(
    cellular: Option<&dji4g_domain::CellularSnapshot>,
    probes: Option<&FeatureProbeView>,
    language: Language,
) -> IdentityVm {
    // Only a probe record that provably belongs to the rendered snapshot may annotate it; an
    // uncorrelated record can never leak plaintext or label these rows.
    let probes = probes.filter(|view| view.captured);
    let (numbers, number_absent) = match cellular.and_then(|cell| cell.numbers.as_ref()) {
        Some(NumberLookup::Reported(reported)) => (
            reported.iter().map(|number| number.masked()).collect(),
            None,
        ),
        // Empty is a successful command with no records: the device simply has nothing to
        // report, so the row says exactly that instead of implying a fault.
        Some(NumberLookup::Empty) => (
            Vec::new(),
            Some(LocalizedText::new(
                language,
                TextKey::ValueNumberNotProvided,
            )),
        ),
        // Absent means the read did not complete this cycle; the honest wording stays separate
        // from 「no number on this SIM」 (research §4.1 possible case five).
        None => (
            Vec::new(),
            Some(LocalizedText::new(
                language,
                TextKey::ValuePhoneNumberNotRead,
            )),
        ),
    };
    let number_note = probes
        .and_then(|view| feature_status_note(view.numbers_status))
        .map(|key| LocalizedText::new(language, key));
    let iccid_masked = cellular
        .and_then(|cell| cell.sim_identity.as_ref())
        .map(|identity| identity.iccid_masked.clone());
    let iccid_reveal_full = probes
        .filter(|view| view.iccid_full.is_some() && iccid_masked.is_some())
        .and_then(|view| view.iccid_full.clone());
    let iccid_note = probes
        .and_then(|view| feature_status_note(view.iccid_status))
        .map(|key| LocalizedText::new(language, key));
    IdentityVm {
        numbers,
        number_absent,
        number_note,
        iccid_masked,
        iccid_reveal_full,
        iccid_note,
    }
}

/// One compact line for the LTE serving-cell report; absent fields are skipped so a searching
/// module degrades to 未驻留 instead of a row of empty brackets. SINR is kept raw with an
/// explicit 「单位待确认」 note until the firmware scaling is confirmed (research §5.2), and the
/// field layout itself is annotated as provisional at the row level (research §10).
fn serving_cell_text(cell: &dji4g_domain::ServingCell, language: Language) -> String {
    if cell.rat.is_none() {
        return match cell.state.as_deref() {
            Some("SEARCH") => {
                t(language, crate::localization::TextKey::ServingSearching).to_owned()
            }
            Some("LIMSRV") => t(
                language,
                crate::localization::TextKey::ServingLimitedService,
            )
            .to_owned(),
            Some("NOCELL") => t(language, crate::localization::TextKey::ServingNoCell).to_owned(),
            _ => t(language, crate::localization::TextKey::ServingNotCamped).to_owned(),
        };
    }
    let mut parts = vec![cell.rat.clone().unwrap_or_default()];
    if let Some(phrase) = cell
        .state
        .as_deref()
        .and_then(|state| serving_state_phrase(state, language))
    {
        parts.push(phrase);
    }
    if let (Some(mcc), Some(mnc)) = (cell.mcc.as_deref(), cell.mnc.as_deref()) {
        if !mcc.is_empty() && !mnc.is_empty() {
            parts.push(format!("PLMN {mcc}-{mnc}"));
        }
    }
    if let Some(cell_id) = cell.cell_id {
        parts.push(format!("Cell {cell_id:08X}"));
    }
    if let Some(pci) = cell.pci {
        parts.push(format!("PCI {pci}"));
    }
    if let Some(band) = cell.band {
        parts.push(format!("B{band}"));
    }
    if let Some(earfcn) = cell.earfcn {
        parts.push(format!("EARFCN {earfcn}"));
    }
    if let Some(tac) = cell.tac {
        parts.push(format!("TAC {tac:04X}"));
    }
    if let Some(rsrp) = cell.rsrp_dbm {
        parts.push(format!("RSRP {rsrp} dBm"));
    }
    if let Some(rsrq) = cell.rsrq_db {
        parts.push(format!("RSRQ {rsrq} dB"));
    }
    if let Some(sinr) = cell.sinr_raw {
        let sinr = sinr.to_string();
        parts.push(crate::localization::format_positional(
            language,
            crate::localization::TextKey::ServingSinr,
            &[&sinr],
        ));
    }
    parts.join(" · ")
}

/// User-facing phrase for a serving-cell state code. `CONNECT` carries no phrase (it is the
/// ordinary in-call/active state and the RAT line already implies it); unknown codes are passed
/// through verbatim rather than guessed at.
fn serving_state_phrase(state: &str, language: Language) -> Option<String> {
    match state {
        "CONNECT" => None,
        "NOCONN" => Some(t(language, crate::localization::TextKey::ServingCampedIdle).to_owned()),
        "SEARCH" => Some(t(language, crate::localization::TextKey::ServingSearching).to_owned()),
        "LIMSRV" => Some(
            t(
                language,
                crate::localization::TextKey::ServingLimitedService,
            )
            .to_owned(),
        ),
        "NOCELL" => Some(t(language, crate::localization::TextKey::ServingNoCell).to_owned()),
        other => Some(other.to_owned()),
    }
}

fn preview_values(values: &[String], language: Language) -> String {
    if values.is_empty() {
        return t(language, crate::localization::TextKey::ValueNotAvailable);
    }
    let preview = values
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if values.len() > 2 {
        let more = (values.len() - 2).to_string();
        crate::localization::format_positional(
            language,
            crate::localization::TextKey::ValuePreviewMore,
            &[&preview, &more],
        )
    } else {
        preview
    }
}

/// The read-only verdict at the top of the overview: what the panel concluded, and one quiet line
/// of the numbers behind it.
///
/// It carries no button. Every check, retry, switch and repair lives on the page that owns it — the
/// overview states the result and nothing else — so the only thing this can return is nothing at
/// all.
pub(crate) fn render_summary(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    probes: Option<&FeatureProbeView>,
) {
    let vm = overview_vm_with_probes(snapshot, language, probes);
    let availability = super::availability_vm(
        &snapshot.app,
        &snapshot.diagnostics,
        std::time::SystemTime::now(),
        language,
    );
    super::components::status_banner(
        ui,
        &availability.title.text,
        &availability.reason.text,
        availability.tone,
    );
    // A quiet summary line; live throughput already has its own chart immediately below.
    wrapped_label(
        ui,
        meta_text(crate::localization::format_positional(
            language,
            crate::localization::TextKey::OverviewSummaryLine,
            &[&vm.carrier.text, &vm.signal.text, &vm.temperature.text],
        )),
    );
}

#[cfg(test)]
pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    rate_history: &crate::ui::RateHistory,
    temperature_history: &crate::ui::TemperatureHistory,
    probes: Option<&FeatureProbeView>,
) {
    render_with_sim_action(
        ui,
        snapshot,
        language,
        rate_history,
        temperature_history,
        probes,
        |_| {},
    );
}

/// The caller owns actions and navigation; the overview only places the SIM entry.
pub(crate) fn render_with_sim_action(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    rate_history: &crate::ui::RateHistory,
    temperature_history: &crate::ui::TemperatureHistory,
    probes: Option<&FeatureProbeView>,
    mut sim_action: impl FnMut(&mut Ui),
) {
    let mut vm = overview_vm_with_probes(snapshot, language, probes);
    if !snapshot.rates_sampled_at.is_some_and(|time| {
        std::time::SystemTime::now()
            .duration_since(time)
            .is_ok_and(|age| age <= std::time::Duration::from_secs(3))
    }) {
        vm.down_rate = None;
        vm.up_rate = None;
    }
    render_cards(
        ui,
        &vm,
        language,
        rate_history,
        temperature_history,
        snapshot.app.observed_at,
        |ui| {
            render_device_card(
                ui,
                &vm,
                snapshot.app.cellular.as_ref(),
                language,
                &mut sim_action,
            )
        },
    );
}

fn render_cards(
    ui: &mut Ui,
    vm: &OverviewVm,
    language: Language,
    rate_history: &crate::ui::RateHistory,
    temperature_history: &crate::ui::TemperatureHistory,
    observed_at: std::time::SystemTime,
    render_device: impl FnOnce(&mut Ui),
) {
    if ui.available_width() >= super::TWO_COLUMN_MIN_WIDTH {
        two_columns(
            ui,
            |ui| {
                render_trend_card(
                    ui,
                    vm,
                    language,
                    rate_history,
                    temperature_history,
                    observed_at,
                );
                render_network_card(ui, vm, language);
            },
            |ui| {
                render_device(ui);
            },
        );
    } else {
        render_trend_card(
            ui,
            vm,
            language,
            rate_history,
            temperature_history,
            observed_at,
        );
        render_device(ui);
        render_network_card(ui, vm, language);
    }
    render_timeline_card(ui, vm, language);
}

fn render_trend_card(
    ui: &mut Ui,
    vm: &OverviewVm,
    language: Language,
    rate_history: &crate::ui::RateHistory,
    temperature_history: &crate::ui::TemperatureHistory,
    observed_at: std::time::SystemTime,
) {
    let response = ui
        .scope(|ui| {
            section_frame(ui, |ui| {
                let tab_id = egui::Id::new("overview-chart-tab");
                let mut tab = ui.data(|data| data.get_temp::<u8>(tab_id)).unwrap_or(0);
                super::components::segmented_control(
                    ui,
                    tab_id.with("control"),
                    &mut tab,
                    &[
                        super::components::TabItem::new(
                            0,
                            t(language, crate::localization::TextKey::OverviewTabRate),
                        ),
                        super::components::TabItem::new(
                            1,
                            t(
                                language,
                                crate::localization::TextKey::TemperatureSectionHeading,
                            ),
                        ),
                    ],
                );
                ui.data_mut(|data| data.insert_temp(tab_id, tab));
                if tab == 0 {
                    render_rate_section(ui, rate_history, vm.down_rate, vm.up_rate, language);
                } else {
                    render_temperature_section(ui, vm, temperature_history, observed_at, language);
                }
            });
        })
        .response;
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new("overview-trend-rect"), response.rect));
    let _ = response;
}

fn render_device_card(
    ui: &mut Ui,
    vm: &OverviewVm,
    cellular: Option<&dji4g_domain::CellularSnapshot>,
    language: Language,
    sim_action: &mut dyn FnMut(&mut Ui),
) {
    let response = ui
        .scope(|ui| {
            section_frame(ui, |ui| {
                ui.label(section_heading(t(
                    language,
                    crate::localization::TextKey::OverviewDeviceHeading,
                )));
                info_grid(ui, "overview-device-details", |ui| {
                    for (label, value) in [
                        (
                            t(language, crate::localization::TextKey::FieldCarrier),
                            &vm.carrier.text,
                        ),
                        (
                            t(
                                language,
                                crate::localization::TextKey::FieldRadioAccessTechnology,
                            ),
                            &vm.radio_access_technology.text,
                        ),
                        (
                            t(
                                language,
                                crate::localization::TextKey::FieldRegistrationShort,
                            ),
                            &vm.registration.text,
                        ),
                    ] {
                        ui.label(field_label(label));
                        wrapped_label(ui, value);
                        ui.end_row();
                    }
                    ui.label(field_label("SIM"));
                    ui.horizontal_wrapped(|ui| {
                        wrapped_label(ui, &vm.sim.text);
                        sim_action(ui);
                    });
                    ui.end_row();
                    for (label, value) in [
                        (
                            t(language, crate::localization::TextKey::FieldModelShort),
                            &vm.device.text,
                        ),
                        (
                            t(language, crate::localization::TextKey::FieldFirmware),
                            &vm.firmware.text,
                        ),
                        (
                            t(language, crate::localization::TextKey::FieldPdpState),
                            &vm.pdp.text,
                        ),
                    ] {
                        ui.label(field_label(label));
                        wrapped_label(ui, value);
                        ui.end_row();
                    }
                    ui.label(field_label(t(
                        language,
                        crate::localization::TextKey::FieldServingCell,
                    )));
                    render_serving_cell_value(ui, vm, language);
                    ui.end_row();
                    ui.label(field_label(t(
                        language,
                        crate::localization::TextKey::FieldTemperature,
                    )));
                    render_temperature_value(ui, vm);
                    ui.end_row();
                });
                if cellular.is_some() {
                    render_identity_area(ui, &vm.identity, cellular, language);
                }
            });
        })
        .response;
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new("overview-device-rect"), response.rect));
    let _ = response;
}

fn render_network_card(ui: &mut Ui, vm: &OverviewVm, language: Language) {
    let response = ui
        .scope(|ui| {
            section_frame(ui, |ui| {
                ui.label(section_heading(t(
                    language,
                    crate::localization::TextKey::OverviewNetworkHeading,
                )));
                info_grid(ui, "overview-network-grid", |ui| {
                    ui.label(field_label(t(
                        language,
                        crate::localization::TextKey::FieldDefaultRouteShort,
                    )));
                    wrapped_label(ui, vm.route.text.clone());
                    ui.end_row();
                    ui.label(field_label("DNS"));
                    wrapped_label(ui, vm.dns.text.clone());
                    ui.end_row();
                    ui.label(field_label(t(
                        language,
                        crate::localization::TextKey::FieldIpAddresses,
                    )));
                    render_addresses(ui, vm, language);
                    ui.end_row();
                    ui.label(field_label(
                        LocalizedText::new(language, TextKey::FieldAdapterErrors).text,
                    ));
                    wrapped_label(ui, vm.adapter_metrics.errors.text.clone());
                    ui.end_row();
                    ui.label(field_label(
                        LocalizedText::new(language, TextKey::FieldAdapterDiscards).text,
                    ));
                    wrapped_label(ui, vm.adapter_metrics.discards.text.clone());
                    ui.end_row();
                    ui.label(field_label(
                        LocalizedText::new(language, TextKey::FieldAdapterLinkRate).text,
                    ));
                    wrapped_label(ui, vm.adapter_metrics.link_rate.text.clone());
                    ui.end_row();
                });
                if let Some(note) = &vm.adapter_metrics.link_note {
                    wrapped_label(ui, meta_text(note.text.clone()));
                }
            });
        })
        .response;
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new("overview-network-rect"), response.rect));
    let _ = response;
}

fn render_timeline_card(ui: &mut Ui, vm: &OverviewVm, language: Language) {
    let response = ui
        .scope(|ui| {
            section_frame(ui, |ui| {
                ui.label(section_heading(
                    LocalizedText::new(language, TextKey::TimelineHeading).text,
                ));
                ui.add_space(8.0);
                if vm.timeline.is_empty() {
                    wrapped_label(
                        ui,
                        meta_text(LocalizedText::new(language, TextKey::TimelineEmpty).text),
                    );
                    return;
                }
                for row in &vm.timeline {
                    ui.horizontal_wrapped(|ui| {
                        let time = row.time.clone().unwrap_or_else(|| "--:--:--".to_owned());
                        ui.label(meta_text(time));
                        wrapped_label(
                            ui,
                            RichText::new(row.text.text.clone())
                                .size(scale::BODY)
                                .color(scale::ink()),
                        );
                    });
                }
            });
        })
        .response;
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new("overview-timeline-rect"), response.rect));
    let _ = response;
}

/// The 温度 value cell: the reading (or the honest 未读取到) plus its firmware-scope note, the
/// reported channel count, and — on hover — the raw `+QTEMP:` line the reading came from.
fn render_temperature_value(ui: &mut Ui, vm: &OverviewVm) {
    ui.vertical(|ui| {
        let response = wrapped_label(ui, vm.temperature.text.clone());
        if let Some(raw) = &vm.temperature_raw {
            let _ = response.on_hover_text(raw.clone());
        }
        if let Some(note) = &vm.temperature_sensors_note {
            wrapped_label(ui, meta_text(note.text.clone()));
        }
        if let Some(note) = &vm.temperature_note {
            wrapped_label(ui, meta_text(note.text.clone()));
        }
    });
}

/// The 「较上次」 phrase for one measured change.  The magnitude is carried with its sign so a rise
/// and a fall can never be read as each other.
#[must_use]
pub fn temperature_delta_text(
    delta: crate::ui::TemperatureDelta,
    language: Language,
) -> LocalizedText {
    match delta {
        crate::ui::TemperatureDelta::Up(value) => format_text_in(
            language,
            TextKey::TemperatureDeltaUp,
            &TextArgs::detail(value.to_string()),
        ),
        crate::ui::TemperatureDelta::Down(value) => format_text_in(
            language,
            TextKey::TemperatureDeltaDown,
            &TextArgs::detail(value.to_string()),
        ),
        crate::ui::TemperatureDelta::Flat => {
            LocalizedText::new(language, TextKey::TemperatureDeltaFlat)
        }
    }
}

/// The 「模块温度」 section (§7.5): the current reading with its change since the previous cycle
/// and the age of that reading, over the trend line of the recent observations.
///
/// A cycle that reported nothing keeps the row's own wording (未读取到 with its classified note,
/// or 未获取 without cellular evidence) — the panel never draws a curve it did not measure.
fn render_temperature_section(
    ui: &mut Ui,
    vm: &OverviewVm,
    history: &crate::ui::TemperatureHistory,
    observed_at: std::time::SystemTime,
    language: Language,
) {
    let window = format_age(
        crate::ui::TEMPERATURE_SAMPLE_PERIOD * crate::ui::TEMPERATURE_HISTORY_CAPACITY as u32,
        language,
    )
    .text;
    let window_text = format_text_in(
        language,
        TextKey::TemperatureTrendWindow,
        &TextArgs::age(window),
    )
    .text;
    let sample_period = format_age(crate::ui::TEMPERATURE_SAMPLE_PERIOD, language).text;
    let width = ui.available_width();
    ui.set_min_width(width);
    ui.set_max_width(width);
    ui.horizontal(|ui| {
        ui.label(section_heading(
            LocalizedText::new(language, TextKey::TemperatureSectionHeading).text,
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            wrapped_label(
                ui,
                RichText::new(window_text)
                    .size(scale::RATE_AUX)
                    .color(scale::secondary()),
            );
        });
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        match vm.temperature_celsius {
            Some(celsius) => wrapped_label(
                ui,
                RichText::new(format!("{celsius} °C"))
                    .size(scale::RATE_NUMBER)
                    .color(scale::ink()),
            ),
            None => wrapped_label(
                ui,
                RichText::new(vm.temperature.text.clone())
                    .size(scale::BODY)
                    .color(scale::faint()),
            ),
        };
        if let Some(delta) = vm
            .temperature_celsius
            .and_then(|current| crate::ui::temperature_delta(current, history.previous_reading()))
        {
            wrapped_label(
                ui,
                RichText::new(temperature_delta_text(delta, language).text)
                    .size(scale::RATE_AUX)
                    .color(scale::secondary()),
            );
        }
        if let Ok(age) = std::time::SystemTime::now().duration_since(observed_at) {
            let age_text = format_text_in(
                language,
                TextKey::ObservedAgo,
                &TextArgs::age(format_age(age, language).text),
            )
            .text;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                wrapped_label(
                    ui,
                    RichText::new(age_text)
                        .size(scale::RATE_AUX)
                        .color(scale::secondary()),
                );
            });
        }
    });
    ui.add_space(8.0);
    crate::ui::paint_temperature_chart(ui, history, language);
    ui.add_space(8.0);
    let trend_note = format_text_in(
        language,
        TextKey::TemperatureTrendNote,
        &TextArgs::age(sample_period),
    )
    .text;
    wrapped_label(ui, meta_text(trend_note));
    if let Some(note) = &vm.temperature_sensors_note {
        wrapped_label(ui, meta_text(note.text.clone()));
    }
    if let Some(note) = &vm.temperature_note {
        wrapped_label(ui, meta_text(note.text.clone()));
    }
    // An answer this build could not read is described by its own bytes: the raw `+QTEMP:` line
    // stays on screen below the classified note.  A parsed reading keeps it on the row's hover
    // instead, so device chatter never becomes page furniture.
    if vm.temperature_celsius.is_none() {
        if let Some(raw) = &vm.temperature_raw {
            wrapped_label(ui, meta_text(raw.clone()));
        }
    }
}

/// The 服务小区 value cell: the compact report line, the provisional-layout note (research §10),
/// a hover with the raw `+QENG` line when the correlated observation still holds it, and the
/// classified failure note when the probe did not complete.
fn render_serving_cell_value(ui: &mut Ui, vm: &OverviewVm, language: Language) {
    ui.vertical(|ui| {
        let response = wrapped_label(ui, vm.serving_cell.text.clone());
        if let Some(raw) = &vm.serving_cell_raw {
            let _ = response.on_hover_text(raw.clone());
        }
        if vm.serving_cell_provisional {
            wrapped_label(
                ui,
                meta_text(LocalizedText::new(language, TextKey::ServingCellLayoutProvisional).text),
            );
        }
        if let Some(note) = &vm.serving_cell_note {
            wrapped_label(ui, meta_text(note.text.clone()));
        }
    });
}

/// The 「身份信息」 area (research §4.1/§4.3): 本机号码 with source/verification/capture rows and
/// the ICCID.  Numbers and the ICCID default to masked display; full values appear only for two
/// seconds after an explicit [显示] click, and copy never bypasses a user action.
fn render_identity_area(
    ui: &mut Ui,
    identity: &IdentityVm,
    cellular: Option<&dji4g_domain::CellularSnapshot>,
    language: Language,
) {
    ui.add_space(14.0);
    ui.label(section_heading(
        LocalizedText::new(language, TextKey::IdentityHeading).text,
    ));
    ui.add_space(8.0);
    info_grid(ui, "overview-identity-grid", |ui| {
        ui.label(field_label(
            LocalizedText::new(language, TextKey::FieldPhoneNumber).text,
        ));
        render_phone_number_value(ui, identity, cellular, language);
        ui.end_row();
        if !identity.numbers.is_empty() {
            // 来源/验证状态/采集时间: a device-reported number is not proof of an operator
            // account, and the capture is only meaningful inside the SIM session that produced
            // it (research §4.1).
            ui.label(field_label(
                LocalizedText::new(language, TextKey::FieldNumberSource).text,
            ));
            wrapped_label(
                ui,
                LocalizedText::new(language, TextKey::ValueNumberSourceSimReport).text,
            );
            ui.end_row();
            ui.label(field_label(
                LocalizedText::new(language, TextKey::FieldVerificationState).text,
            ));
            wrapped_label(
                ui,
                LocalizedText::new(language, TextKey::ValueVerificationNotCarrierChecked).text,
            );
            ui.end_row();
            ui.label(field_label(
                LocalizedText::new(language, TextKey::FieldCaptureTime).text,
            ));
            wrapped_label(
                ui,
                LocalizedText::new(language, TextKey::ValueCaptureTimeSimSession).text,
            );
            ui.end_row();
        }
        ui.label(field_label(
            LocalizedText::new(language, TextKey::FieldIccid).text,
        ));
        render_iccid_value(ui, identity, language);
        ui.end_row();
    });
}

/// How long a full-value reveal stays on screen before the mask returns.
const REVEAL_WINDOW: Duration = Duration::from_secs(2);

fn reveal_active(ui: &Ui, id: &str) -> bool {
    ui.data(|data| {
        data.get_temp::<Instant>(egui::Id::new(id))
            .is_some_and(|at| at.elapsed() < REVEAL_WINDOW)
    })
}

fn toggle_reveal(ui: &Ui, id: &str) {
    if reveal_active(ui, id) {
        ui.data_mut(|data| data.remove::<Instant>(egui::Id::new(id)));
    } else {
        ui.data_mut(|data| data.insert_temp(egui::Id::new(id), Instant::now()));
    }
}

fn just_copied(ui: &Ui, id: &str) -> bool {
    ui.data(|data| {
        data.get_temp::<Instant>(egui::Id::new(id))
            .is_some_and(|at| at.elapsed() < REVEAL_WINDOW)
    })
}

fn mark_copied(ui: &Ui, id: &str) {
    ui.data_mut(|data| data.insert_temp(egui::Id::new(id), Instant::now()));
}

/// The plaintext numbers of the reported snapshot entry, in report order.  Only handed to the
/// UI on an explicit user click; never part of the view model or any serialized document.
fn full_numbers(cellular: Option<&dji4g_domain::CellularSnapshot>) -> Option<Vec<String>> {
    match cellular.and_then(|cell| cell.numbers.as_ref()) {
        Some(NumberLookup::Reported(reported)) => Some(
            reported
                .iter()
                .map(|number| number.expose_after_user_action().to_owned())
                .collect(),
        ),
        _ => None,
    }
}

fn render_phone_number_value(
    ui: &mut Ui,
    identity: &IdentityVm,
    cellular: Option<&dji4g_domain::CellularSnapshot>,
    language: Language,
) {
    let reveal_id = "overview-identity-phone-reveal";
    let copy_id = "overview-identity-phone-copied";
    let reported = !identity.numbers.is_empty();
    let plain = full_numbers(cellular);
    let revealed = reveal_active(ui, reveal_id) && reported;
    ui.vertical(|ui| {
        if reported {
            let numbers: &[String] = if revealed {
                plain.as_deref().unwrap_or(&identity.numbers)
            } else {
                &identity.numbers
            };
            wrapped_label(
                ui,
                RichText::new(numbers.join("\n"))
                    .monospace()
                    .size(scale::RATE_AUX)
                    .color(scale::ink()),
            );
        } else if let Some(absent) = &identity.number_absent {
            wrapped_label(ui, absent.text.clone());
        }
        if let Some(note) = &identity.number_note {
            wrapped_label(ui, meta_text(note.text.clone()));
        }
        if !reported {
            return;
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if super::components::plain_button(ui, TextKey::ButtonShow.to_string(language))
                .clicked()
            {
                toggle_reveal(ui, reveal_id);
            }
            let copied_label = if just_copied(ui, copy_id) {
                TextKey::ButtonCopied
            } else {
                TextKey::ButtonCopy
            };
            if super::components::plain_button(ui, copied_label.to_string(language)).clicked() {
                if let Some(numbers) = &plain {
                    ui.ctx().copy_text(numbers.join("\n"));
                    mark_copied(ui, copy_id);
                }
            }
        });
    });
}

fn render_iccid_value(ui: &mut Ui, identity: &IdentityVm, language: Language) {
    let reveal_id = "overview-identity-iccid-reveal";
    ui.vertical(|ui| {
        match &identity.iccid_masked {
            Some(mask) => {
                let revealed = reveal_active(ui, reveal_id);
                let shown = if revealed {
                    identity
                        .iccid_reveal_full
                        .as_deref()
                        .unwrap_or(mask.as_str())
                } else {
                    mask.as_str()
                };
                wrapped_label(
                    ui,
                    RichText::new(shown)
                        .monospace()
                        .size(scale::RATE_AUX)
                        .color(scale::ink()),
                );
                if identity.iccid_reveal_full.is_some()
                    && super::components::plain_button(ui, TextKey::ButtonShow.to_string(language))
                        .clicked()
                {
                    toggle_reveal(ui, reveal_id);
                }
            }
            None => {
                wrapped_label(
                    ui,
                    LocalizedText::new(language, TextKey::ValueIccidNotRead).text,
                );
            }
        }
        if let Some(note) = &identity.iccid_note {
            wrapped_label(ui, meta_text(note.text.clone()));
        }
    });
}

/// IP addresses as monospace lines (like the reference's `.address`), a note for the hidden
/// remainder, and a copy button that copies every address.
fn render_addresses(ui: &mut Ui, vm: &OverviewVm, language: Language) -> egui::Response {
    let copy_id = egui::Id::new("overview-copy-address");
    let copied_at = ui.data(|data| data.get_temp::<Instant>(copy_id));
    let just_copied = copied_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2));
    let copy_label = TextKey::ButtonCopyAddress.to_string(language);
    let copied_label = TextKey::ButtonCopied.to_string(language);
    let label = if just_copied {
        &copied_label
    } else {
        &copy_label
    };
    let width = ui.available_width().max(1.0);
    let spacing = ui.spacing().item_spacing;
    let button_width = ui.fonts(|fonts| {
        [&copy_label, &copied_label]
            .into_iter()
            .map(|label| {
                fonts
                    .layout_no_wrap(
                        label.clone(),
                        egui::FontId::proportional(scale::BODY),
                        scale::ink(),
                    )
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    }) + 2.0 * ui.spacing().button_padding.x;
    let button_width = button_width.max(scale::BUTTON_MIN_W).min(width);
    let inline = width - button_width - spacing.x >= 120.0;
    let text_width = if inline {
        width - button_width - spacing.x
    } else {
        width
    };
    let mut texts: Vec<(String, egui::FontId, egui::Color32)> = vm
        .addresses
        .iter()
        .take(2)
        .map(|address| {
            (
                address.clone(),
                egui::FontId::monospace(scale::RATE_AUX),
                scale::ink(),
            )
        })
        .collect();
    if texts.is_empty() {
        texts.push((
            TextKey::ValueNotAvailable.to_string(language),
            egui::FontId::proportional(scale::BODY),
            scale::secondary(),
        ));
    }
    let more = vm.addresses.len().saturating_sub(2);
    if more > 0 {
        texts.push((
            format_text_in(language, TextKey::ValueMoreItems, &TextArgs::count(more)).text,
            egui::FontId::proportional(scale::META),
            scale::faint(),
        ));
    }
    let galleys: Vec<_> = ui.fonts(|fonts| {
        texts
            .into_iter()
            .map(|(text, font, color)| fonts.layout(text, font, color, text_width))
            .collect()
    });
    let text_height = galleys.iter().map(|galley| galley.size().y).sum::<f32>()
        + spacing.y * galleys.len().saturating_sub(1) as f32;
    let height = if inline {
        text_height.max(scale::CONTROL_H)
    } else {
        text_height + spacing.y + scale::CONTROL_H
    };
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        if inline {
            egui::Layout::left_to_right(egui::Align::Min)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        },
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(text_width, text_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(text_width);
                    for galley in galleys {
                        ui.add(egui::Label::new(galley));
                    }
                },
            );
            let button = ui
                .add_enabled_ui(!vm.addresses.is_empty(), |ui| {
                    super::components::centered_button(
                        ui,
                        label.as_str(),
                        egui::Button::new("").min_size(egui::vec2(button_width, scale::CONTROL_H)),
                        egui::vec2(button_width, scale::CONTROL_H),
                    )
                })
                .inner;
            #[cfg(test)]
            ui.data_mut(|data| data.insert_temp(copy_id.with("button-rect"), button.rect));
            if button.clicked() {
                ui.ctx().copy_text(vm.addresses.join("\n"));
                ui.data_mut(|data| data.insert_temp(copy_id, Instant::now()));
            }
        },
    )
    .response
}

trait LocalizedKeyText {
    fn to_string(self, language: Language) -> String;
}

impl LocalizedKeyText for TextKey {
    fn to_string(self, language: Language) -> String {
        crate::localization::LocalizedText::new(language, self).text
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn overview_fits_narrow_content_without_invalid_column_bounds() {
        for width in [520.0, 760.0, 1000.0] {
            let context = eframe::egui::Context::default();
            crate::ui::apply_style(&context);
            let snapshot = std::sync::Arc::new(
                dji4g_application::ReducerState::new(std::time::SystemTime::UNIX_EPOCH).snapshot(),
            );
            let _ = context.run(
                eframe::egui::RawInput {
                    screen_rect: Some(eframe::egui::Rect::from_min_size(
                        eframe::egui::Pos2::ZERO,
                        eframe::egui::vec2(width, 1800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    eframe::egui::CentralPanel::default().show(ctx, |ui| {
                        let right = ui.max_rect().right();
                        super::render(
                            ui,
                            &snapshot,
                            crate::localization::Language::ZhCn,
                            &crate::ui::RateHistory::default(),
                            &crate::ui::TemperatureHistory::default(),
                            None,
                        );
                        assert!(
                            ui.min_rect().right() <= right + 1.0,
                            "overview overflow at width {width}"
                        );
                    });
                },
            );
        }
    }

    use super::{
        MAX_LISTED_TEMPERATURE_VALUES, adapter_metrics_vm, identity_vm, serving_cell_text,
        serving_state_phrase, temperature_delta_text, temperature_sensors_note, temperature_vm,
        timeline_rows,
    };
    use crate::feature_probe::FeatureProbeView;
    use crate::localization::{Language, TextKey, template};
    use dji4g_application::AdapterMetrics;
    use dji4g_at_protocol::SensorTemperature;
    use dji4g_domain::{
        AttachState, CellularSnapshot, FeatureStatus, NumberLookup, PhoneNumber, RegistrationState,
        SimIdentity, SimState, Timeline, TimelineDetail, TimelineEvent, TimelineEventKind,
    };
    use std::time::{Duration, SystemTime};

    fn cellular(numbers: Option<NumberLookup>, identity: Option<SimIdentity>) -> CellularSnapshot {
        CellularSnapshot {
            sim: SimState::Ready,
            registration: RegistrationState::RegisteredHome,
            attached: AttachState::Attached,
            carrier: None,
            radio_access_technology: None,
            signal_rssi_dbm: None,
            apn: None,
            pdp_address: None,
            pdp_state: None,
            firmware: None,
            serving_cell: None,
            sim_identity: identity,
            numbers,
            temperature_celsius: None,
            temperature_status: FeatureStatus::NotProbed,
        }
    }

    fn identity() -> SimIdentity {
        SimIdentity {
            iccid_masked: "8986…2345".to_owned(),
            fingerprint: [7; 8],
        }
    }

    fn reported(numbers: &[&str]) -> NumberLookup {
        NumberLookup::Reported(
            numbers
                .iter()
                .map(|number| PhoneNumber::new(*number, 145))
                .collect(),
        )
    }

    #[test]
    fn identity_vm_masks_reported_numbers_and_keeps_order() {
        let snapshot = cellular(Some(reported(&["+8613800138000", "+8613900138000"])), None);
        let vm = identity_vm(Some(&snapshot), None, Language::ZhCn);
        assert_eq!(vm.numbers, vec!["****8000", "****8000"]);
        assert!(vm.number_absent.is_none());
        assert!(vm.number_note.is_none());
        assert!(vm.iccid_masked.is_none());
        assert!(vm.iccid_reveal_full.is_none());
    }

    #[test]
    fn identity_vm_distinguishes_empty_reply_from_failed_read() {
        let empty = cellular(Some(NumberLookup::Empty), None);
        let vm = identity_vm(Some(&empty), None, Language::ZhCn);
        let absent = vm.number_absent.expect("empty reply still explains itself");
        assert_eq!(
            absent.text,
            template(Language::ZhCn, TextKey::ValueNumberNotProvided)
        );
        assert!(!absent.text.contains("故障"));

        let none = cellular(None, None);
        let vm = identity_vm(Some(&none), None, Language::ZhCn);
        let absent = vm.number_absent.expect("an absent read must stay separate");
        assert_eq!(
            absent.text,
            template(Language::ZhCn, TextKey::ValuePhoneNumberNotRead)
        );
    }

    #[test]
    fn identity_vm_notes_only_correlated_classified_failures() {
        let cell = cellular(None, Some(identity()));
        let probes = FeatureProbeView {
            captured: true,
            numbers_status: FeatureStatus::TransportFailure,
            iccid_status: FeatureStatus::UnsupportedConfirmed,
            serving_cell_status: FeatureStatus::Supported,
            iccid_full: Some("89860123456789012345".to_owned()),
            serving_cell_raw: None,
            temperature_sensors: Vec::new(),
            temperature_raw: None,
        };
        let vm = identity_vm(Some(&cell), Some(&probes), Language::ZhCn);
        let number_note = vm.number_note.expect("a timed-out read needs a note");
        assert!(number_note.text.contains("本次超时"));
        let iccid_note = vm.iccid_note.expect("unsupported needs a note");
        assert!(iccid_note.text.contains("固件不支持"));
        assert_eq!(vm.iccid_masked.as_deref(), Some("8986…2345"));
        assert_eq!(
            vm.iccid_reveal_full.as_deref(),
            Some("89860123456789012345")
        );
    }

    #[test]
    fn identity_vm_withholds_the_iccid_plaintext_without_correlation() {
        let cell = cellular(Some(reported(&["+8613800138000"])), Some(identity()));
        let probes = FeatureProbeView {
            captured: false,
            numbers_status: FeatureStatus::NotProbed,
            iccid_status: FeatureStatus::NotProbed,
            serving_cell_status: FeatureStatus::NotProbed,
            iccid_full: Some("89860123456789012345".to_owned()),
            serving_cell_raw: None,
            temperature_sensors: Vec::new(),
            temperature_raw: None,
        };
        let vm = identity_vm(Some(&cell), Some(&probes), Language::ZhCn);
        assert!(
            vm.iccid_reveal_full.is_none(),
            "uncorrelated plaintext never leaves"
        );
        assert!(vm.number_note.is_none());
        assert!(vm.iccid_note.is_none());
    }

    #[test]
    fn serving_cell_state_phrases_cover_the_known_codes() {
        let zh = Language::ZhCn;
        assert_eq!(serving_state_phrase("CONNECT", zh), None);
        for (code, key) in [
            ("NOCONN", crate::localization::TextKey::ServingCampedIdle),
            ("SEARCH", crate::localization::TextKey::ServingSearching),
            (
                "LIMSRV",
                crate::localization::TextKey::ServingLimitedService,
            ),
            ("NOCELL", crate::localization::TextKey::ServingNoCell),
        ] {
            assert_eq!(
                serving_state_phrase(code, zh),
                Some(crate::localization::LocalizedText::new(zh, key).text),
                "{code}"
            );
        }
        assert_eq!(serving_state_phrase("WEIRD", zh).as_deref(), Some("WEIRD"));
    }

    fn empty_cell(state: Option<&str>) -> dji4g_domain::ServingCell {
        dji4g_domain::ServingCell {
            state: state.map(str::to_owned),
            duplex: None,
            rat: None,
            mcc: None,
            mnc: None,
            cell_id: None,
            pci: None,
            earfcn: None,
            band: None,
            ul_mhz: None,
            dl_mhz: None,
            tac: None,
            rsrp_dbm: None,
            rsrq_db: None,
            rssi_dbm: None,
            sinr_raw: None,
            srxlev_raw: None,
        }
    }

    #[test]
    fn serving_cell_text_handles_state_only_and_full_reports() {
        // State-only reports (research §10: SEARCH/LIMSRV are valid states, not errors).
        for (state, key) in [
            ("SEARCH", crate::localization::TextKey::ServingSearching),
            (
                "LIMSRV",
                crate::localization::TextKey::ServingLimitedService,
            ),
            ("NOCELL", crate::localization::TextKey::ServingNoCell),
        ] {
            assert_eq!(
                serving_cell_text(&empty_cell(Some(state)), Language::ZhCn),
                crate::localization::LocalizedText::new(Language::ZhCn, key).text
            );
        }

        // A full NOCONN report renders the idle phrase, the PLMN, hex identifiers and the raw
        // SINR with its unconfirmed-unit note (research §5.2: never display it as dB).
        let full = dji4g_domain::ServingCell {
            state: Some("NOCONN".to_owned()),
            duplex: Some("FDD".to_owned()),
            rat: Some("LTE".to_owned()),
            mcc: Some("460".to_owned()),
            mnc: Some("01".to_owned()),
            cell_id: Some(0x1A2B3C4),
            pci: Some(123),
            earfcn: Some(1650),
            band: Some(3),
            ul_mhz: None,
            dl_mhz: None,
            tac: Some(0xABC),
            rsrp_dbm: Some(-95),
            rsrq_db: Some(-10),
            rssi_dbm: None,
            sinr_raw: Some(15),
            srxlev_raw: None,
        };
        let text = serving_cell_text(&full, Language::ZhCn);
        assert!(text.contains("LTE"), "RAT first: {text}");
        assert!(
            text.contains(
                crate::localization::LocalizedText::new(
                    Language::ZhCn,
                    crate::localization::TextKey::ServingCampedIdle,
                )
                .text
                .as_str()
            ),
            "NOCONN reads as idle: {text}"
        );
        assert!(
            text.contains("PLMN 460-01"),
            "PLMN survives leading zeros: {text}"
        );
        assert!(text.contains("Cell 01A2B3C4"), "cell id is hex: {text}");
        assert!(text.contains("TAC 0ABC"), "TAC is hex: {text}");
        assert!(text.contains("SINR 15（单位待确认）"));
        assert!(
            !text.contains("15 dB"),
            "unconfirmed SINR must never be shown as dB"
        );
    }

    #[test]
    fn serving_cell_unknown_state_degrades_to_searching_note_without_a_rat() {
        assert_eq!(
            serving_cell_text(&empty_cell(Some("???")), Language::ZhCn),
            crate::localization::LocalizedText::new(
                Language::ZhCn,
                crate::localization::TextKey::ServingNotCamped
            )
            .text
        );
    }

    fn cellular_with_temperature(celsius: Option<i16>, status: FeatureStatus) -> CellularSnapshot {
        let mut cell = cellular(None, None);
        cell.temperature_celsius = celsius;
        cell.temperature_status = status;
        cell
    }

    #[test]
    fn temperature_row_distinguishes_reading_gap_and_classified_failure() {
        // A reading carries the firmware-defined-sensor disclaimer.
        let cell = cellular_with_temperature(Some(28), FeatureStatus::Supported);
        let (value, note) = temperature_vm(Some(&cell), Language::ZhCn);
        assert_eq!(value.text, "28 °C");
        assert_eq!(
            note.map(|note| note.text),
            Some(template(Language::ZhCn, TextKey::TemperatureSensorNote).to_owned())
        );

        // No reading with a classified failure carries that failure's precise note.
        let cell = cellular_with_temperature(None, FeatureStatus::TransportFailure);
        let (value, note) = temperature_vm(Some(&cell), Language::ZhCn);
        assert_eq!(value.text, "未读取到");
        assert!(
            note.is_some_and(|note| note.text.contains("本次超时")),
            "a failed probe must keep its classification"
        );

        // No reading and no probe at all stays honest at 未读取到 with no note.
        let cell = cellular_with_temperature(None, FeatureStatus::NotProbed);
        let (value, note) = temperature_vm(Some(&cell), Language::ZhCn);
        assert_eq!(value.text, "未读取到");
        assert!(note.is_none());

        // Without any cellular evidence the row degrades to 未获取.
        let (value, note) = temperature_vm(None, Language::ZhCn);
        assert_eq!(
            value.text,
            crate::localization::LocalizedText::new(
                Language::ZhCn,
                crate::localization::TextKey::ValueNotAvailable
            )
            .text
        );
        assert!(note.is_none());
    }

    #[test]
    fn temperature_delta_phrases_carry_the_magnitude_with_its_sign() {
        use crate::ui::TemperatureDelta;
        assert_eq!(
            temperature_delta_text(TemperatureDelta::Up(1), Language::ZhCn).text,
            "较上次 +1 °C"
        );
        assert_eq!(
            temperature_delta_text(TemperatureDelta::Down(6), Language::ZhCn).text,
            "较上次 -6 °C"
        );
        assert_eq!(
            temperature_delta_text(TemperatureDelta::Flat, Language::ZhCn).text,
            "与上次相同"
        );
    }

    #[test]
    fn temperature_sensors_note_lists_the_reported_channels_in_order() {
        let readings = [
            SensorTemperature {
                name: None,
                celsius: 57,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
        ];
        let note = temperature_sensors_note(&readings, Language::ZhCn).expect("three readings");
        assert!(
            note.text.contains('3'),
            "the count is stated: {}",
            note.text
        );
        assert!(
            note.text.contains("57 / 51 / 51"),
            "the values stay in report order: {}",
            note.text
        );
        assert!(
            note.text.contains("以固件为准"),
            "the meaning of the channels is never claimed: {}",
            note.text
        );
        // One reading needs no list — the row already shows it.
        assert!(temperature_sensors_note(&readings[..1], Language::ZhCn).is_none());
        assert!(temperature_sensors_note(&[], Language::ZhCn).is_none());
        // A long report is visibly cut instead of looking complete.
        let long: Vec<SensorTemperature> = (0..MAX_LISTED_TEMPERATURE_VALUES + 2)
            .map(|index| SensorTemperature {
                name: None,
                celsius: 40 + index as i16,
            })
            .collect();
        let note = temperature_sensors_note(&long, Language::ZhCn).expect("a long report");
        assert!(note.text.contains('…'), "a cut list says so: {}", note.text);
    }

    #[test]
    fn adapter_metrics_rows_render_rx_tx_counts_and_mbps_link_rates() {
        let metrics = AdapterMetrics {
            rx_bytes: 0,
            tx_bytes: 0,
            in_errors: 1,
            out_errors: 2,
            in_discards: 3,
            out_discards: 4,
            link_rx_bits_per_second: 100_000_000,
            link_tx_bits_per_second: 50_000_000,
        };
        let vm = adapter_metrics_vm(Some(metrics), Language::ZhCn);
        assert_eq!(vm.errors.text, "收 1 / 发 2");
        assert_eq!(vm.discards.text, "收 3 / 发 4");
        assert_eq!(vm.link_rate.text, "收 100.0 Mbps / 发 50.0 Mbps");
        assert_eq!(
            vm.link_note.map(|note| note.text),
            Some(template(Language::ZhCn, TextKey::AdapterLinkRateNote).to_owned())
        );

        let vm = adapter_metrics_vm(None, Language::ZhCn);
        assert_eq!(
            vm.errors.text,
            crate::localization::LocalizedText::new(
                Language::ZhCn,
                crate::localization::TextKey::ValueNotAvailable
            )
            .text
        );
        assert_eq!(
            vm.discards.text,
            crate::localization::LocalizedText::new(
                Language::ZhCn,
                crate::localization::TextKey::ValueNotAvailable
            )
            .text
        );
        assert_eq!(
            vm.link_rate.text,
            crate::localization::LocalizedText::new(
                Language::ZhCn,
                crate::localization::TextKey::ValueNotAvailable
            )
            .text
        );
        assert!(vm.link_note.is_none());
    }

    fn timeline_event(at: SystemTime, detail: TimelineDetail) -> TimelineEvent {
        TimelineEvent {
            at,
            kind: TimelineEventKind::CellChanged,
            detail,
        }
    }

    #[test]
    fn timeline_rows_are_newest_first_limited_to_ten_and_clocked() {
        let mut timeline = Timeline::new();
        for index in 0..12_u64 {
            timeline.push(timeline_event(
                SystemTime::UNIX_EPOCH + Duration::from_secs(index),
                TimelineDetail::Kind,
            ));
        }
        let rows = timeline_rows(&timeline, Language::ZhCn);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].time.as_deref(), Some("00:00:11"));
        assert_eq!(
            rows[0].text.text,
            template(Language::ZhCn, TextKey::TimelineCellChangedDetail)
        );
        assert_eq!(rows[9].time.as_deref(), Some("00:00:02"));
        assert_eq!(
            rows[9].text.text,
            template(Language::ZhCn, TextKey::TimelineCellChangedDetail)
        );

        assert!(timeline_rows(&Timeline::new(), Language::ZhCn).is_empty());
    }

    /// The row is written by the page, not by the reducer: the same recorded values read in the
    /// page's language, with the transition's values in the catalog's `{}` slots.
    #[test]
    fn timeline_rows_render_the_recorded_values_in_the_page_language() {
        let mut timeline = Timeline::new();
        timeline.push(TimelineEvent {
            at: SystemTime::UNIX_EPOCH,
            kind: TimelineEventKind::RegistrationChanged,
            detail: TimelineDetail::Registration {
                from: RegistrationState::Searching,
                to: RegistrationState::RegisteredHome,
            },
        });
        let zh = timeline_rows(&timeline, Language::ZhCn);
        assert_eq!(zh[0].text.text, "注册状态：正在搜索 → 已注册到本地网络");
        assert_eq!(zh[0].text.key, TextKey::TimelineRegistrationChangedDetail);

        let en = timeline_rows(&timeline, Language::EnUs);
        assert_eq!(
            en[0].text.text,
            "Registration: Searching → Registered on the home network"
        );

        let mut dns = Timeline::new();
        dns.push(TimelineEvent {
            at: SystemTime::UNIX_EPOCH,
            kind: TimelineEventKind::DnsChanged,
            detail: TimelineDetail::Dns {
                from: dji4g_domain::BoundDnsStatus::Succeeded,
                to: dji4g_domain::BoundDnsStatus::Failed,
            },
        });
        assert_eq!(
            timeline_rows(&dns, Language::ZhCn)[0].text.text,
            "DNS 探测：通过 → 失败"
        );
    }
}

#[cfg(test)]
mod address_layout_regressions {
    use super::*;
    fn fixture(count: usize, language: Language) -> OverviewVm {
        let snapshot =
            dji4g_application::ReducerState::new(std::time::SystemTime::now()).snapshot();
        let mut vm = overview_vm_with_probes(&snapshot, language, None);
        vm.addresses = (0..count)
            .map(|n| format!("2408:845d:b02:89dc:ffff:ffff:ffff:{n:04x}"))
            .collect();
        vm
    }
    fn address_row_height(height: f32) -> f32 {
        let ctx = egui::Context::default();
        crate::ui::apply_style(&ctx);
        let vm = fixture(4, Language::ZhCn);
        let mut measured = 0.0;
        for _ in 0..6 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(540.0, height),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        info_grid(ui, "address-row-regression", |ui| {
                            ui.label("IP");
                            measured = ui
                                .scope(|ui| {
                                    render_addresses(ui, &vm, Language::ZhCn);
                                })
                                .response
                                .rect
                                .height();
                            ui.end_row();
                            ui.label("errors");
                            ui.label("0 / 0");
                            ui.end_row();
                        });
                    });
                },
            );
        }
        measured
    }
    #[test]
    fn multi_ipv6_row_is_compact_and_independent_of_window_height() {
        let short = address_row_height(600.0);
        let tall = address_row_height(1400.0);
        assert!(short < 100.0, "unexpected address row height: {short}");
        assert!(
            (short - tall).abs() < 1.0,
            "address row expanded with the window: {short} -> {tall}"
        );
    }
    #[test]
    fn cards_follow_the_shared_breakpoint_in_all_languages_and_tabs() {
        for width in [759.0, 760.0, 761.0] {
            for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
                for count in [0, 1, 2, 4] {
                    for tab in [0_u8, 1] {
                        let ctx = egui::Context::default();
                        crate::ui::apply_style(&ctx);
                        ctx.data_mut(|data| {
                            data.insert_temp(egui::Id::new("overview-chart-tab"), tab)
                        });
                        let vm = fixture(count, language);
                        for _ in 0..6 {
                            let _ = ctx.run(
                                egui::RawInput {
                                    screen_rect: Some(egui::Rect::from_min_size(
                                        egui::Pos2::ZERO,
                                        egui::vec2(width, 1800.0),
                                    )),
                                    ..Default::default()
                                },
                                |ctx| {
                                    egui::CentralPanel::default()
                                        .frame(egui::Frame::none())
                                        .show(ctx, |ui| {
                                            render_cards(
                                                ui,
                                                &vm,
                                                language,
                                                &crate::ui::RateHistory::default(),
                                                &crate::ui::TemperatureHistory::default(),
                                                std::time::SystemTime::now(),
                                                |ui| {
                                                    render_device_card(
                                                        ui,
                                                        &vm,
                                                        None,
                                                        language,
                                                        &mut |_| {},
                                                    )
                                                },
                                            );
                                        });
                                },
                            );
                        }
                        let rect = |name: &str| {
                            ctx.data(|data| {
                                data.get_temp::<egui::Rect>(egui::Id::new(format!(
                                    "overview-{name}-rect"
                                )))
                                .unwrap()
                            })
                        };
                        let (trend, device, network, timeline) = (
                            rect("trend"),
                            rect("device"),
                            rect("network"),
                            rect("timeline"),
                        );
                        assert!(timeline.top() >= network.bottom() - 1.0);
                        if width >= super::super::TWO_COLUMN_MIN_WIDTH {
                            assert!(network.top() >= trend.bottom() - 1.0);
                            assert!((network.left() - trend.left()).abs() < 1.0);
                            assert!(device.left() > trend.right());
                            assert!((device.top() - trend.top()).abs() < 1.0);
                        } else {
                            assert!(device.top() >= trend.bottom() - 1.0);
                            assert!(network.top() >= device.bottom() - 1.0);
                        }
                        let button = ctx.data(|data| {
                            data.get_temp::<egui::Rect>(
                                egui::Id::new("overview-copy-address").with("button-rect"),
                            )
                            .unwrap()
                        });
                        assert!(
                            network.expand(1.0).contains_rect(button),
                            "button overflow: {width} {language:?} {count} {button:?} vs {network:?}"
                        );
                        assert!(network.right() <= width + 1.0, "network overflow: {width}");
                    }
                }
            }
        }
    }

    #[test]
    fn address_copy_uses_all_addresses_and_keeps_the_button_slot() {
        for width in [210.0, 500.0] {
            for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
                for count in [0, 1, 2, 4] {
                    let ctx = egui::Context::default();
                    crate::ui::apply_style(&ctx);
                    let vm = fixture(count, language);
                    let frame = |events| {
                        ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 800.0),
                                )),
                                events,
                                ..Default::default()
                            },
                            |ctx| {
                                egui::CentralPanel::default().show(ctx, |ui| {
                                    render_addresses(ui, &vm, language);
                                });
                            },
                        )
                    };
                    for _ in 0..4 {
                        let _ = frame(vec![]);
                    }
                    let before = ctx.data(|data| {
                        data.get_temp::<egui::Rect>(
                            egui::Id::new("overview-copy-address").with("button-rect"),
                        )
                        .unwrap()
                    });
                    let point = before.center();
                    let mut copied = String::new();
                    for pressed in [true, false] {
                        let output = frame(vec![
                            egui::Event::PointerMoved(point),
                            egui::Event::PointerButton {
                                pos: point,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]);
                        copied.push_str(&output.platform_output.copied_text);
                    }
                    let _ = frame(vec![]);
                    let after = ctx.data(|data| {
                        data.get_temp::<egui::Rect>(
                            egui::Id::new("overview-copy-address").with("button-rect"),
                        )
                        .unwrap()
                    });
                    assert_eq!(copied, vm.addresses.join("\n"));
                    assert_eq!(before, after, "copy changed the button slot");
                }
            }
        }
    }
}
