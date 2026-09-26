//! Device tools page: the three-tier AT terminal (预设 / 查询 / 专家) and its bounded transcript.
//!
//! The page only projects [`DeviceToolsSnapshot`] and dispatches closed [`UiCommand`] values
//! through the panel's sink. Request and response text lives exclusively in this page's on-screen
//! projection: nothing here writes it to `crate::logging`, a diagnostic export, or a `tracing`
//! field, and the expert tab freezes every non-whitelisted line for its own per-command
//! confirmation before anything is written to the module.

use std::time::{Duration, SystemTime};

use dji4g_application::{
    ControlledRepairError, ControlledRepairRequest, ControllerSnapshot, DeviceToolsSnapshot,
    PendingExpertTool, ToolHistoryEntry, ToolOperationKind, ToolOutcome, ToolPhase, UiCommand,
    UiSendError, UsbNetReading,
};
use dji4g_at_protocol::{
    PdpContextState, PdpType, ToolInputError, ToolReadId, ToolWriteId, ValidatedToolLine,
    VerifiedUsbNetProfile, classify_known_write,
};
use dji4g_domain::{DeviceEpoch, FeatureStatus, StableDeviceIdentity};
use eframe::egui::{self, Color32, RichText, Ui};

use super::{
    StatusTone, detail_text, field_label, info_grid, meta_text, scale, section_frame,
    section_heading, wrapped_label,
};
use crate::localization::Language;

/// Which tier of the terminal is on screen.
///
/// The tab row itself never locks: the user may keep reading and switching while a task runs, and
/// only the action buttons inside a tab are gated by [`DeviceToolsSnapshot::busy`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToolTab {
    #[default]
    Preset,
    Query,
    Expert,
}

/// UI-local state of the tools page.
///
/// Never persisted and never serialized. The expert unlock is deliberately session-local and is
/// reset as soon as the device or SIM context changes, so a switch flipped while looking at one
/// module can never carry over to another.
#[derive(Default)]
pub struct DeviceToolsState {
    pub tab: ToolTab,
    expert_unlocked: bool,
    expert_input: String,
    query_input: String,
    query_selected: Option<ToolReadId>,
    point_to_expert: bool,
    apn_cid: String,
    apn_value: String,
    error: Option<String>,
    notice: Option<String>,
    context: Option<(DeviceEpoch, u64)>,
}

impl DeviceToolsState {
    /// Forget everything typed or unlocked against a previous device/SIM context.
    fn observe_context(&mut self, snapshot: &ControllerSnapshot) {
        let context = snapshot
            .app
            .device
            .as_ref()
            .map(|device| (device.epoch, snapshot.sim_epoch));
        if self.context == context {
            return;
        }
        self.context = context;
        self.expert_unlocked = false;
        self.expert_input.clear();
        self.apn_cid.clear();
        self.apn_value.clear();
        self.error = None;
        self.notice = None;
    }

    /// Debug-only tab selector for the screenshot harness; it never changes any other state.
    #[cfg(debug_assertions)]
    pub(crate) fn set_review_tab(&mut self, tab: usize) {
        self.tab = match tab {
            1 => ToolTab::Query,
            2 => ToolTab::Expert,
            _ => ToolTab::Preset,
        };
    }
}

// ---------------------------------------------------------------------------------------------
// Closed display vocabulary
// ---------------------------------------------------------------------------------------------

/// Status of one capability row. `NotProbed` reads 未查询; it is never presented as a failure.
#[must_use]
pub fn feature_status_text(status: FeatureStatus) -> (&'static str, StatusTone) {
    match status {
        FeatureStatus::NotProbed => ("未查询", StatusTone::Neutral),
        FeatureStatus::Supported => ("可用", StatusTone::Positive),
        FeatureStatus::Empty => ("无数据", StatusTone::Neutral),
        FeatureStatus::UnsupportedConfirmed => ("固件不支持", StatusTone::Negative),
        FeatureStatus::TemporarilyUnavailable => ("暂时不可用", StatusTone::Caution),
        FeatureStatus::FormatMismatch => ("格式不匹配", StatusTone::Caution),
        FeatureStatus::TransportFailure => ("查询超时", StatusTone::Negative),
    }
}

/// Localized description of one tool outcome. The stable code is shown next to it, so a report can
/// name the exact result without ever quoting the response text.
#[must_use]
pub fn tool_outcome_text(outcome: ToolOutcome) -> &'static str {
    match outcome {
        ToolOutcome::Ok => "模块返回 OK；配置是否生效需另行确认",
        ToolOutcome::Rejected => "模块明确拒绝了本次命令",
        ToolOutcome::Unsupported => "模块表示不支持此命令",
        ToolOutcome::TransportFailure => "没有收到可用应答（超时、串口错误或已断开）",
        ToolOutcome::FormatMismatch => "模块有应答，但响应格式未被识别",
        ToolOutcome::CancelledBeforeWrite => "未执行的命令已取消；已执行项见终端记录",
        ToolOutcome::OutcomeUnknown => "可能已写入但未收到最终应答；不会自动重试",
        ToolOutcome::ContextChanged => "设备或 SIM 已变化，本次结果已作废",
    }
}

/// Presentation tone of one tool outcome: only a proven refusal or a lost transport reads
/// negative, and an unknown effect stays a caution instead of being rounded either way.
#[must_use]
pub fn tool_outcome_tone(outcome: ToolOutcome) -> StatusTone {
    match outcome {
        ToolOutcome::Ok => StatusTone::Positive,
        ToolOutcome::Rejected | ToolOutcome::Unsupported | ToolOutcome::TransportFailure => {
            StatusTone::Negative
        }
        ToolOutcome::FormatMismatch | ToolOutcome::OutcomeUnknown => StatusTone::Caution,
        ToolOutcome::CancelledBeforeWrite | ToolOutcome::ContextChanged => StatusTone::Neutral,
    }
}

/// Localized description of an input refusal; the parser's stable code stays the machine-readable
/// form and is rendered beside this text.
#[must_use]
pub fn tool_input_error_text(error: ToolInputError) -> &'static str {
    match error {
        ToolInputError::Empty => "请输入一条 AT 命令",
        ToolInputError::TooLong => "命令超过 256 个字符",
        ToolInputError::NonAscii => "命令只能包含 ASCII 字符",
        ToolInputError::ControlCharacter => "命令不能包含控制字符或换行",
        ToolInputError::ChainedCommand => "命令不能包含分号链式调用",
        ToolInputError::InvalidPrefix => "命令必须以 AT 开头",
        ToolInputError::NotWhitelisted => "该命令不在只读白名单内",
        ToolInputError::InteractiveCommand => "此命令族需要交互式会话，文本终端无法安全驱动",
    }
}

/// Display label of one whitelisted read, including the AT request it maps to.
#[must_use]
pub fn tool_read_text(id: ToolReadId) -> &'static str {
    match id {
        ToolReadId::Attention => "模块响应（AT）",
        ToolReadId::Manufacturer => "制造商（AT+CGMI）",
        ToolReadId::Model => "型号（AT+CGMM）",
        ToolReadId::Revision => "固件版本（AT+CGMR）",
        ToolReadId::SimState => "SIM 状态（AT+CPIN?）",
        ToolReadId::SignalQuality => "信号质量（AT+CSQ）",
        ToolReadId::Operator => "运营商（AT+COPS?）",
        ToolReadId::EpsRegistration => "网络注册（AT+CEREG?）",
        ToolReadId::PacketAttach => "分组附着（AT+CGATT?）",
        ToolReadId::PdpContexts => "PDP 上下文（AT+CGDCONT?）",
        ToolReadId::PdpActivation => "PDP 激活状态（AT+CGACT?）",
        ToolReadId::PdpAddresses => "PDP 地址（AT+CGPADDR）",
        ToolReadId::UsbNet => "USB 网络模式（AT+QCFG=\"usbnet\"）",
        ToolReadId::Temperature => "温度（AT+QTEMP）",
        ToolReadId::ServingCell => "服务小区（AT+QENG=\"servingcell\"）",
        ToolReadId::SmsFormat => "短信格式（AT+CMGF?）",
        ToolReadId::SmsStorage => "短信存储（AT+CPMS?）",
    }
}

/// Label of one operation kind, shared by the task strip and the history list.
#[must_use]
pub fn tool_operation_text(kind: ToolOperationKind) -> String {
    match kind {
        ToolOperationKind::Read(id) => tool_read_text(id).to_owned(),
        ToolOperationKind::ProbeAll => "全部预设查询（批量）".to_owned(),
        ToolOperationKind::Expert => "AT 命令（高级）".to_owned(),
    }
}

/// Phase label and tone of the task strip.
#[must_use]
pub fn tool_phase_text(phase: ToolPhase) -> (&'static str, StatusTone) {
    match phase {
        ToolPhase::Idle => ("空闲", StatusTone::Neutral),
        ToolPhase::Queued => ("排队中", StatusTone::Progress),
        ToolPhase::Running => ("执行中", StatusTone::Progress),
        ToolPhase::Cancelling => ("正在取消", StatusTone::Caution),
        ToolPhase::Finished => ("已结束", StatusTone::Neutral),
    }
}

/// Human elapsed time of one task or history entry, in the band that reads honestly.
#[must_use]
pub fn format_elapsed(elapsed: Duration) -> String {
    if elapsed.as_secs() >= 60 {
        format!(
            "{} 分 {} 秒",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60
        )
    } else if elapsed.as_millis() >= 1_000 {
        format!("{:.1} 秒", elapsed.as_secs_f64())
    } else {
        format!("{} 毫秒", elapsed.as_millis())
    }
}

/// Last four characters of the device's container id (or instance path when the container id is
/// absent). The full value never reaches the screen.
#[must_use]
pub fn masked_device_id(identity: &StableDeviceIdentity) -> String {
    let source = if identity.container_id.trim().is_empty() {
        identity.device_instance_id.as_str()
    } else {
        identity.container_id.as_str()
    };
    let tail: String = {
        let chars: Vec<char> = source.trim().chars().collect();
        chars[chars.len().saturating_sub(4)..].iter().collect()
    };
    if tail.trim().is_empty() {
        "未获取".to_owned()
    } else {
        format!("…{}", tail.trim())
    }
}

/// The verified USB network mode's display name.
#[must_use]
pub fn usb_profile_text(profile: VerifiedUsbNetProfile) -> &'static str {
    match profile {
        VerifiedUsbNetProfile::DjiNdis => "DJI NDIS（电脑网卡）",
        VerifiedUsbNetProfile::Ecm => "ECM",
    }
}

fn usb_profile_raw_value(profile: VerifiedUsbNetProfile) -> u8 {
    match profile {
        VerifiedUsbNetProfile::DjiNdis => 0,
        VerifiedUsbNetProfile::Ecm => 1,
    }
}

/// The exact normalized line the reviewed repair flow puts on the wire for a known write. Shown in
/// the expert tab so a recognized write is confirmed against what will really run.
#[must_use]
pub fn normalized_write_text(write: &ToolWriteId) -> String {
    match write {
        ToolWriteId::RestartModule => "AT+CFUN=1,1".to_owned(),
        ToolWriteId::SetApn { cid, apn } => {
            format!("AT+CGDCONT={},\"IP\",\"{}\"", cid.get(), apn.as_str())
        }
        ToolWriteId::SetUsbNetProfile(profile) => {
            format!("AT+QCFG=\"usbnet\",{}", usb_profile_raw_value(*profile))
        }
    }
}

/// The write request a recognized expert line maps onto, or `None` when it is not a known write.
fn known_write_request(write: &ToolWriteId) -> ControlledRepairRequest {
    match write {
        ToolWriteId::RestartModule => ControlledRepairRequest::RestartModule,
        ToolWriteId::SetApn { cid, apn } => ControlledRepairRequest::SetApn {
            cid: *cid,
            apn: apn.clone(),
        },
        ToolWriteId::SetUsbNetProfile(profile) => {
            ControlledRepairRequest::SetUsbNetProfile { profile: *profile }
        }
    }
}

/// Localized text of one PDP type; these are protocol names and stay verbatim.
#[must_use]
pub fn pdp_type_text(pdp_type: PdpType) -> &'static str {
    match pdp_type {
        PdpType::Ip => "IP",
        PdpType::Ipv6 => "IPv6",
        PdpType::Ipv4v6 => "IPv4v6",
    }
}

fn pdp_state_text(state: PdpContextState) -> &'static str {
    match state {
        PdpContextState::Active => "已激活",
        PdpContextState::Inactive => "未激活",
    }
}

fn send_error_text(error: UiSendError) -> String {
    match error {
        UiSendError::QueueFull => "命令队列已满，请稍后重试".to_owned(),
        UiSendError::Closed => "后台连接已关闭，请稍后重试".to_owned(),
    }
}

fn controlled_repair_error_text(error: ControlledRepairError) -> String {
    match error {
        ControlledRepairError::InvalidPdpContextId => {
            "PDP 上下文编号必须在 1 到 16 之间。".to_owned()
        }
        ControlledRepairError::InvalidApn => {
            "APN 不合法：不能为空、不能超过 100 字节，且不能包含引号、逗号、分号或控制字符。"
                .to_owned()
        }
        ControlledRepairError::InvalidDnsProfile | ControlledRepairError::UnsupportedAction => {
            "该受控操作当前不可用。".to_owned()
        }
    }
}

fn badge(ui: &mut Ui, text: impl Into<String>, color: egui::Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.09))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(12.0).color(color));
        });
}

fn value_text(value: Option<&str>) -> RichText {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => RichText::new(value).size(scale::BODY).color(scale::INK),
        None => RichText::new("未查询")
            .size(scale::META)
            .color(scale::FAINT),
    }
}

fn observed_time_text(at: Option<SystemTime>, now: SystemTime, language: Language) -> String {
    let Some(at) = at else {
        return "未查询".to_owned();
    };
    let clock = super::clock_hms(at).unwrap_or_else(|| "时间未知".to_owned());
    match now.duration_since(at) {
        Ok(age) => format!("{clock}（{}前）", super::format_age(age, language).text),
        Err(_) => clock,
    }
}

// ---------------------------------------------------------------------------------------------
// Page rendering
// ---------------------------------------------------------------------------------------------

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
) {
    state.observe_context(snapshot);
    let tools = &snapshot.device_tools;
    let now = SystemTime::now();
    let device_present = snapshot.app.device.is_some();
    // A tool task and a repair operation share the serial actor, so both disable every write
    // button while the tab row itself stays usable.
    let busy = snapshot.serial_work_busy
        || tools.busy()
        || snapshot.operation.as_ref().is_some_and(|operation| {
            matches!(
                operation.state,
                dji4g_application::OperationState::Running { .. }
            )
        });
    let can_act = device_present && !busy;

    render_header(ui, snapshot, tools);
    ui.add_space(14.0);
    let previous_tab = state.tab;
    render_tabs(ui, state);
    if state.tab != previous_tab {
        state.error = None;
        state.point_to_expert = false;
    }
    ui.add_space(10.0);
    if tools.task.is_some() {
        render_task_strip(ui, tools, sink);
        ui.add_space(14.0);
    }
    match state.tab {
        ToolTab::Preset => render_preset(ui, snapshot, sink, state, now, language, can_act),
        ToolTab::Query => render_query(ui, tools, sink, state, can_act),
        ToolTab::Expert => render_expert(ui, snapshot, sink, state, can_act, language),
    }
    render_feedback(ui, state);
    ui.add_space(14.0);
    render_history(ui, tools, state, sink);
}

/// Current target: identity, AT port and device/SIM epoch. Nothing here is rendered from a
/// fabricated value — an absent device says so and every action stays disabled.
fn render_header(ui: &mut Ui, snapshot: &ControllerSnapshot, tools: &DeviceToolsSnapshot) {
    super::components::page_heading(ui, "设备工具", "读取模块信息，按需执行经过确认的操作");
    let device = snapshot.app.device.as_ref();
    let identity = device.map(|device| &device.identity).or_else(|| {
        tools
            .profile
            .context
            .as_ref()
            .map(|context| &context.identity)
    });
    ui.horizontal_wrapped(|ui| match device {
        Some(device) => {
            badge(ui, "设备已连接", scale::DOWNLOAD);
            if let Some(identity) = identity {
                ui.label(
                    RichText::new(format!(
                        "VID {:04X} · PID {:04X} · 设备标识 {}",
                        identity.vid,
                        identity.pid,
                        masked_device_id(identity)
                    ))
                    .size(scale::BODY)
                    .strong()
                    .color(scale::INK),
                );
            }
            let port = device.at_port.as_deref().unwrap_or("未获取");
            ui.label(meta_text(format!("AT 端口 {port}")));
            ui.label(meta_text(format!(
                "设备代次 {} · SIM 会话 {}",
                device.epoch.0, snapshot.sim_epoch
            )));
        }
        None => {
            badge(ui, "未检测到设备", StatusTone::Negative.color());
            ui.label(meta_text("连接模块后才能执行查询与受控操作"));
            ui.label(meta_text(format!("SIM 会话 {}", snapshot.sim_epoch)));
        }
    });
}

fn render_tabs(ui: &mut Ui, state: &mut DeviceToolsState) {
    super::components::page_tabs(
        ui,
        egui::Id::new("device-tools-tabs"),
        &mut state.tab,
        &[
            super::components::TabItem::new(ToolTab::Preset, "预设"),
            super::components::TabItem::new(ToolTab::Query, "只读查询"),
            super::components::TabItem::new(ToolTab::Expert, "AT 命令（高级）"),
        ],
    );
    ui.label(meta_text("任务执行期间仍可切换标签页，但写入按钮会被禁用"));
}

fn render_task_strip(
    ui: &mut Ui,
    tools: &DeviceToolsSnapshot,
    sink: &dyn crate::app::PanelCommandSink,
) {
    egui::Frame::none()
        .fill(Color32::from_rgb(0xf5, 0xf7, 0xfb))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(section_heading("任务进度"));
                match &tools.task {
                    Some(task) => {
                        let (phase, tone) = tool_phase_text(task.phase);
                        badge(ui, phase, tone.color());
                        ui.label(
                            RichText::new(tool_operation_text(task.operation)).color(scale::INK),
                        );
                        if task.total_items > 0 {
                            ui.label(meta_text(format!(
                                "{}/{}",
                                task.completed_items, task.total_items
                            )));
                        }
                        if task.phase == ToolPhase::Finished {
                            match task.outcome {
                                Some(outcome) => {
                                    ui.label(
                                        RichText::new(
                                            if task.operation == ToolOperationKind::ProbeAll
                                                && outcome == ToolOutcome::Ok
                                            {
                                                "批量查询已结束，请查看逐项结果"
                                            } else {
                                                tool_outcome_text(outcome)
                                            },
                                        )
                                        .color(tool_outcome_tone(outcome).color()),
                                    );
                                    ui.label(meta_text(outcome.code()));
                                }
                                None => {
                                    ui.label(meta_text("已结束，结果未知"));
                                }
                            }
                        }
                        if task.phase.is_active() && ui.button("取消").clicked() {
                            let _ = sink.try_send(UiCommand::CancelDeviceTool { id: task.id });
                        }
                    }
                    None => {
                        ui.label(meta_text("当前没有设备工具任务"));
                    }
                }
            });
            if tools
                .task
                .as_ref()
                .is_some_and(|task| task.phase.is_active())
            {
                wrapped_label(
                    ui,
                    meta_text(
                        "取消只会停止等待，不能撤销已经写入模块的改动；写入超时后不会自动重试。",
                    ),
                );
            }
            if let Some(refusal) = tools.last_refusal {
                wrapped_label(
                    ui,
                    RichText::new(format!(
                        "最近一次请求被拒绝：{}（{}）",
                        tool_outcome_text(refusal),
                        refusal.code()
                    ))
                    .color(tool_outcome_tone(refusal).color()),
                );
            }
        });
}

fn render_preset(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    now: SystemTime,
    language: Language,
    can_act: bool,
) {
    let tools = &snapshot.device_tools;
    let profile = &tools.profile;
    render_profile_section(ui, tools, sink, state, now, language, can_act);
    render_capability_section(ui, tools, now, language, sink, state, can_act);
    render_connection_section(ui, profile);
    render_controlled_actions(ui, snapshot, profile, sink, state, now, language);
}

fn render_profile_section(
    ui: &mut Ui,
    tools: &DeviceToolsSnapshot,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    now: SystemTime,
    language: Language,
    can_act: bool,
) {
    let profile = &tools.profile;
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("模块资料"));
            if ui
                .add_enabled(can_act, egui::Button::new("刷新模块资料"))
                .on_hover_text("按顺序运行全部只读预设查询；不会写入模块")
                .clicked()
            {
                state.notice = None;
                state.error = sink
                    .try_send(UiCommand::ProbeDeviceTools)
                    .err()
                    .map(send_error_text);
            }
        });
        info_grid(ui, "device-tools-profile-grid", |ui| {
            ui.label(field_label("制造商"));
            ui.label(value_text(profile.manufacturer.as_deref()));
            ui.end_row();
            ui.label(field_label("型号"));
            ui.label(value_text(profile.model.as_deref()));
            ui.end_row();
            ui.label(field_label("固件版本"));
            ui.label(value_text(profile.revision.as_deref()));
            ui.end_row();
            ui.label(field_label("USB 网络模式"));
            match profile.usb_net {
                Some(UsbNetReading::Verified(profile)) => {
                    ui.label(RichText::new(usb_profile_text(profile)).color(scale::INK));
                }
                Some(UsbNetReading::Unrecognised) => {
                    ui.label(RichText::new("未识别").color(StatusTone::Caution.color()));
                }
                None => {
                    ui.label(meta_text("未查询"));
                }
            }
            ui.end_row();
            ui.label(field_label("采集时间"));
            ui.label(value_text(Some(
                observed_time_text(profile.observed_at, now, language).as_str(),
            )));
            ui.end_row();
        });
        if matches!(profile.usb_net, Some(UsbNetReading::Unrecognised)) {
            wrapped_label(
                ui,
                RichText::new("模块报告的 USB 网络模式不是本版本已验证的值；不会自动切换。")
                    .color(StatusTone::Caution.color()),
            );
        }
        if profile.is_empty() {
            wrapped_label(
                ui,
                meta_text("尚未读取到模块资料；点击「刷新模块资料」运行一次只读查询。"),
            );
        }
    });
}

fn render_capability_section(
    ui: &mut Ui,
    tools: &DeviceToolsSnapshot,
    now: SystemTime,
    language: Language,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    can_act: bool,
) {
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("能力证据"));
            ui.label(meta_text("每一行只反映一次真实查询的结果"));
        });
        for id in ToolReadId::ALL {
            let row = tools.capability(id);
            let querying = tools.task.as_ref().is_some_and(|task| {
                task.phase.is_active() && task.operation == ToolOperationKind::Read(id)
            });
            let (status_text, tone) = if querying {
                ("本项查询中", StatusTone::Progress)
            } else {
                row.map_or(("未查询", StatusTone::Neutral), |row| {
                    feature_status_text(row.status)
                })
            };
            egui::CollapsingHeader::new(
                RichText::new(format!(
                    "{}    {} {}",
                    tool_read_text(id),
                    tone.marker(),
                    status_text
                ))
                .color(tone.color()),
            )
            .id_salt(("device-tools-capability", id.key()))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            can_act,
                            egui::Button::new(if row.is_some() {
                                "重新查询此项"
                            } else {
                                "查询此项"
                            }),
                        )
                        .clicked()
                    {
                        send_tool_command(sink, state, UiCommand::RunToolRead { id });
                    }
                    if querying {
                        ui.spinner();
                        ui.label(meta_text("本项查询中；下方保留上次结果与采集时间"));
                    }
                });
                match row {
                    Some(row) => {
                        wrapped_label(
                            ui,
                            detail_text(format!(
                                "原因：{}（{}）",
                                tool_outcome_text(row.reason),
                                row.reason.code()
                            )),
                        );
                        wrapped_label(
                            ui,
                            detail_text(format!(
                                "采集：{} · 设备代次 {} · SIM 会话 {}",
                                observed_time_text(Some(row.observed_at), now, language),
                                row.context.device_epoch.0,
                                row.context.sim_epoch
                            )),
                        );
                        if id == ToolReadId::SmsStorage
                            && matches!(row.status, FeatureStatus::Supported | FeatureStatus::Empty)
                        {
                            wrapped_label(ui, meta_text("存储查询可用不代表模块支持发送短信。"));
                        }
                    }
                    None => {
                        wrapped_label(ui, meta_text("尚未执行此查询。"));
                    }
                }
            });
            ui.separator();
        }
        wrapped_label(
            ui,
            meta_text("注意：「短信存储」查询成功仅表示存储查询可用，不代表模块支持发送短信。"),
        );
    });
}

fn render_connection_section(ui: &mut Ui, profile: &dji4g_application::ModuleProfile) {
    section_frame(ui, |ui| {
        ui.label(section_heading("连接配置"));
        if profile.pdp_contexts.is_empty() {
            wrapped_label(
                ui,
                meta_text("尚未读取到 PDP 上下文；点击「刷新模块资料」。"),
            );
        } else {
            info_grid(ui, "device-tools-pdp-grid", |ui| {
                for context in &profile.pdp_contexts {
                    ui.label(field_label(format!("CID {}", context.cid().get())));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(meta_text(pdp_type_text(context.pdp_type())));
                        ui.label(meta_text(pdp_state_text(context.state())));
                        ui.label(
                            RichText::new(format!("APN {}", context.apn().as_str()))
                                .color(scale::INK),
                        );
                    });
                    ui.end_row();
                }
            });
        }
        if profile.temperature.is_empty() {
            wrapped_label(ui, meta_text("尚未读取到温度传感器。"));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (index, reading) in profile.temperature.iter().enumerate() {
                    // A firmware channel without a name is shown by its position — the module gave
                    // no identity for it, and this page never invents one.
                    let label = reading
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("传感器{}", index + 1));
                    badge(
                        ui,
                        format!("{label} {} ℃", reading.celsius),
                        scale::SECONDARY,
                    );
                }
            });
            wrapped_label(ui, meta_text("传感器定义以固件为准。"));
        }
    });
}

fn render_controlled_actions(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    profile: &dji4g_application::ModuleProfile,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    now: SystemTime,
    language: Language,
) {
    use dji4g_application::ActionReadinessKey as Key;
    let apn = super::action_availability::repair_action_availability(
        snapshot,
        Key::EditApn,
        now,
        language,
    );
    let usb = super::action_availability::repair_action_availability(
        snapshot,
        Key::SetUsbNetworkProfile,
        now,
        language,
    );
    let restart = super::action_availability::repair_action_availability(
        snapshot,
        Key::RestartModule,
        now,
        language,
    );
    section_frame(ui, |ui| {
        ui.label(section_heading("受控操作"));
        wrapped_label(
            ui,
            meta_text(
                "以下写入沿用修复页的受控流程：提交后仍需复核目标与风险，且超时不会自动重试。",
            ),
        );
        ui.add_space(6.0);
        // 修改 APN：cid + apn 两个输入，校验通过后交给受控修复流程。
        ui.horizontal_wrapped(|ui| {
            ui.label(field_label("PDP 上下文"));
            ui.add(
                egui::TextEdit::singleline(&mut state.apn_cid)
                    .desired_width(44.0)
                    .hint_text("1"),
            );
            ui.label(field_label("APN"));
            ui.add(
                egui::TextEdit::singleline(&mut state.apn_value)
                    .desired_width(180.0)
                    .hint_text("例如 internet"),
            );
            let filled = !state.apn_cid.trim().is_empty() && !state.apn_value.trim().is_empty();
            if ui
                .add_enabled(apn.enabled && filled, egui::Button::new("修改 APN"))
                .clicked()
            {
                state.notice = None;
                match state.apn_cid.trim().parse::<u8>() {
                    Ok(cid) => {
                        match ControlledRepairRequest::try_apn(cid, state.apn_value.trim()) {
                            Ok(request) => {
                                // The reviewed confirmation box opens now, at click time, exactly as
                                // it does on the repairs page; the plan it prepares usually arrives
                                // while the user is still reading it.
                                sink.prepare_repair_now(request);
                                state.notice = Some(format!(
                                    "已弹出确认窗口；确认后执行：AT+CGDCONT={cid},\"IP\",\"{}\"",
                                    state.apn_value.trim()
                                ));
                            }
                            Err(error) => {
                                state.error = Some(controlled_repair_error_text(error));
                            }
                        }
                    }
                    Err(_) => {
                        state.error = Some("PDP 上下文编号必须是 1 到 16 的整数。".to_owned());
                    }
                }
            }
        });
        if let Some(reason) = &apn.reason {
            wrapped_label(ui, meta_text(&reason.text));
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(field_label("USB 网络模式"));
            match profile.usb_net {
                Some(UsbNetReading::Verified(current)) => {
                    let target = match current {
                        VerifiedUsbNetProfile::DjiNdis => VerifiedUsbNetProfile::Ecm,
                        VerifiedUsbNetProfile::Ecm => VerifiedUsbNetProfile::DjiNdis,
                    };
                    ui.label(meta_text(usb_profile_text(current)));
                    if ui
                        .add_enabled(
                            usb.enabled,
                            egui::Button::new(format!("切换为{}", usb_profile_text(target))),
                        )
                        .on_hover_text("通过受控修复流程切换；会重枚举模块")
                        .clicked()
                    {
                        sink.prepare_repair_now(ControlledRepairRequest::SetUsbNetProfile {
                            profile: target,
                        });
                        state.notice = Some(format!(
                            "已弹出确认窗口；确认后执行：{}",
                            normalized_write_text(&ToolWriteId::SetUsbNetProfile(target))
                        ));
                    }
                }
                Some(UsbNetReading::Unrecognised) => {
                    ui.label(RichText::new("未识别").color(StatusTone::Caution.color()));
                    ui.label(meta_text("当前值未识别，本版本不提供切换。"));
                }
                None => {
                    ui.label(meta_text("未查询；请先刷新模块资料。"));
                }
            }
        });
        if let Some(reason) = &usb.reason {
            wrapped_label(ui, meta_text(&reason.text));
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(restart.enabled, egui::Button::new("重启模块"))
                .on_hover_text("AT+CFUN=1,1；会中断当前连接")
                .clicked()
            {
                sink.prepare_repair_now(ControlledRepairRequest::RestartModule);
                state.notice = Some("已弹出确认窗口；确认后执行：AT+CFUN=1,1".to_owned());
            }
            ui.label(meta_text("重启会暂时中断模块连接。"));
        });
        if let Some(reason) = &restart.reason {
            wrapped_label(ui, meta_text(&reason.text));
        }
    });
}

fn render_query(
    ui: &mut Ui,
    tools: &DeviceToolsSnapshot,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    can_act: bool,
) {
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("只读 AT 查询"));
            ui.label(meta_text("只读白名单内的查询不需要逐条确认"));
        });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("device-tools-read-preset")
                .selected_text(state.query_selected.map_or("选择预设查询", tool_read_text))
                .width(280.0)
                .show_ui(ui, |ui| {
                    for id in ToolReadId::ALL {
                        if ui
                            .selectable_label(state.query_selected == Some(id), tool_read_text(id))
                            .clicked()
                        {
                            state.query_selected = Some(id);
                            state.query_input.clear();
                            state.error = None;
                            state.point_to_expert = false;
                        }
                    }
                });
            let has_input = state.query_selected.is_some() || !state.query_input.trim().is_empty();
            if ui
                .add_enabled(can_act && has_input, egui::Button::new("运行查询"))
                .clicked()
            {
                run_query(sink, state);
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut state.query_input)
                .hint_text("或输入白名单内的只读命令，例如 AT+CSQ")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
        if state.point_to_expert {
            ui.horizontal_wrapped(|ui| {
                ui.label(meta_text(
                    "这条命令不在只读查询列表内。若了解其作用，可到 AT 命令（高级）检查并逐条确认；不确定时请使用预设查询。",
                ));
                if ui.button("打开 AT 命令（高级）").clicked() {
                    state.tab = ToolTab::Expert;
                    state.point_to_expert = false;
                }
            });
        }
        if tools
            .task
            .as_ref()
            .is_some_and(|task| task.phase.is_active())
        {
            wrapped_label(
                ui,
                meta_text("任务执行期间只能读取已有结果，查询按钮已禁用。"),
            );
        }
    });
}

fn run_query(sink: &dyn crate::app::PanelCommandSink, state: &mut DeviceToolsState) {
    state.notice = None;
    state.point_to_expert = false;
    let text = state.query_input.trim();
    if text.is_empty() {
        if let Some(id) = state.query_selected {
            state.error = sink
                .try_send(UiCommand::RunToolRead { id })
                .err()
                .map(send_error_text);
        }
        return;
    }
    match ValidatedToolLine::parse_read_only(text) {
        Ok((_line, id)) => {
            state.error = sink
                .try_send(UiCommand::RunToolRead { id })
                .err()
                .map(send_error_text);
        }
        Err(ToolInputError::NotWhitelisted) => {
            state.error = Some(format!(
                "{}（{}）。",
                tool_input_error_text(ToolInputError::NotWhitelisted),
                ToolInputError::NotWhitelisted.code()
            ));
            state.point_to_expert = true;
        }
        Err(error) => {
            state.error = Some(format!(
                "输入无效：{}（{}）",
                tool_input_error_text(error),
                error.code()
            ));
        }
    }
}

fn render_expert(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    can_act: bool,
    language: Language,
) {
    let tools = &snapshot.device_tools;
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("AT 命令（高级）"));
            if state.expert_unlocked {
                badge(ui, "本次会话已解锁", StatusTone::Caution.color());
            } else {
                if ui
                    .add_enabled(can_act, egui::Button::new("启用 AT 命令输入"))
                    .clicked()
                {
                    state.expert_unlocked = true;
                }
                wrapped_label(
                    ui,
                    meta_text("解锁只在本次会话内有效，设备或 SIM 变化后会自动重新锁定。"),
                );
            }
        });
        wrapped_label(
            ui,
            meta_text(
                "供了解 AT 命令的用户排查问题。每次只发送一条经校验的命令；确认前会显示完整内容。命令可能修改配置或中断连接。",
            ),
        );
        if state.expert_unlocked {
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut state.expert_input)
                    .hint_text("输入一条 AT 命令，例如 AT+CSQ")
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            let write_availability = ValidatedToolLine::parse(state.expert_input.trim())
                .ok()
                .and_then(|line| classify_known_write(&line))
                .map(|write| {
                    use dji4g_application::ActionReadinessKey as Key;
                    let key = match write {
                        ToolWriteId::RestartModule => Key::RestartModule,
                        ToolWriteId::SetUsbNetProfile(_) => Key::SetUsbNetworkProfile,
                        ToolWriteId::SetApn { .. } => Key::EditApn,
                    };
                    super::action_availability::repair_action_availability(
                        snapshot,
                        key,
                        SystemTime::now(),
                        language,
                    )
                });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        can_act
                            && write_availability
                                .as_ref()
                                .is_none_or(|value| value.enabled)
                            && !state.expert_input.trim().is_empty(),
                        egui::Button::new("执行"),
                    )
                    .clicked()
                {
                    submit_expert(sink, state);
                }
                if ui.button("清空输入").clicked() {
                    state.expert_input.clear();
                    state.error = None;
                    state.notice = None;
                }
            });
            if let Some(reason) = write_availability.and_then(|value| value.reason) {
                wrapped_label(ui, meta_text(reason.text));
            }
        } else {
            wrapped_label(
                ui,
                meta_text("终端处于锁定状态：解锁前不会显示输入框与执行按钮。"),
            );
        }
        // The per-command confirmation is independent of the unlock switch: opening the terminal
        // never substitutes for approving one exact command.
        if let Some(pending) = &tools.pending_expert {
            ui.add_space(8.0);
            render_pending_expert(ui, pending, sink);
        }
    });
}

fn submit_expert(sink: &dyn crate::app::PanelCommandSink, state: &mut DeviceToolsState) {
    state.notice = None;
    let text = state.expert_input.trim();
    match ValidatedToolLine::parse(text) {
        Err(error) => {
            state.error = Some(format!(
                "命令未通过校验：{}（{}）",
                tool_input_error_text(error),
                error.code()
            ));
        }
        Ok(line) => match classify_known_write(&line) {
            Some(write) => {
                // A recognized write is routed through the reviewed repair flow, and the exact
                // normalized line is shown before/while that flow runs.
                state.notice = Some(format!(
                    "已识别为受控写入，将执行规范化命令：{}",
                    normalized_write_text(&write)
                ));
                // One frozen confirmation for this action, never two: the expert switch does not
                // add a second dialog on top of the repair confirmation.
                sink.prepare_repair_now(known_write_request(&write));
            }
            None => {
                state.error = sink
                    .try_send(UiCommand::PrepareExpertTool { line })
                    .err()
                    .map(send_error_text);
                if state.error.is_none() {
                    state.notice = Some("命令已冻结，请在下方逐条确认后才会写入模块。".to_owned());
                }
            }
        },
    }
}

fn render_pending_expert(
    ui: &mut Ui,
    pending: &PendingExpertTool,
    sink: &dyn crate::app::PanelCommandSink,
) {
    let remaining = pending
        .expires_at
        .duration_since(SystemTime::now())
        .unwrap_or_default();
    egui::Frame::none()
        .fill(Color32::from_rgb(0xff, 0xf7, 0xe8))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(section_heading("AT 命令待确认"));
            wrapped_label(ui, meta_text("以下命令已冻结，确认后才会写入模块："));
            egui::Frame::none()
                .fill(Color32::from_rgb(0xf5, 0xf7, 0xfb))
                .rounding(8.0)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    wrapped_label(
                        ui,
                        RichText::new(pending.line.expose_for_confirmation())
                            .monospace()
                            .color(scale::INK),
                    );
                });
            wrapped_label(
                ui,
                RichText::new("效果未知，可能改变配置或中断连接。")
                    .color(StatusTone::Caution.color()),
            );
            ui.horizontal_wrapped(|ui| {
                if remaining.is_zero() {
                    ui.label(meta_text("已过期；需要重新准备同一条命令。"));
                } else {
                    ui.label(meta_text(format!(
                        "剩余 {} 秒内有效，过期后需要重新准备。",
                        remaining.as_secs()
                    )));
                }
                if ui.button("确认执行").clicked() {
                    let _ = sink.try_send(UiCommand::ConfirmExpertTool { id: pending.id });
                }
                if ui.button("取消").clicked() {
                    let _ = sink.try_send(UiCommand::CancelExpertToolPlan { id: pending.id });
                }
            });
        });
}

fn render_feedback(ui: &mut Ui, state: &DeviceToolsState) {
    if let Some(notice) = &state.notice {
        ui.add_space(10.0);
        wrapped_label(
            ui,
            RichText::new(notice).color(StatusTone::Positive.color()),
        );
    }
    if let Some(error) = &state.error {
        ui.add_space(10.0);
        wrapped_label(ui, RichText::new(error).color(StatusTone::Negative.color()));
    }
}

// ---------------------------------------------------------------------------------------------
// Terminal output
// ---------------------------------------------------------------------------------------------

fn render_history(
    ui: &mut Ui,
    tools: &DeviceToolsSnapshot,
    state: &mut DeviceToolsState,
    sink: &dyn crate::app::PanelCommandSink,
) {
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("命令记录"));
            ui.label(meta_text("只保存在本机内存中的最近任务记录"));
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!tools.history.is_empty(), egui::Button::new("复制诊断摘要"))
                .on_hover_text("仅包含操作类型、耗时和稳定结果码，不含响应内容")
                .clicked()
            {
                copy_diagnostic_summary(ui, tools);
            }
            if ui
                .add_enabled(!tools.history.is_empty(), egui::Button::new("复制原始响应"))
                .on_hover_text("响应可能包含设备标识、号码或账户信息")
                .clicked()
            {
                copy_raw_response(ui, tools);
            }
            if ui
                .add_enabled(!tools.history.is_empty(), egui::Button::new("清空"))
                .on_hover_text("清空现有内存记录；正在运行的任务完成后仍可能产生新记录。")
                .clicked()
            {
                send_tool_command(sink, state, UiCommand::ClearToolHistory);
            }
        });
        wrapped_label(
            ui,
            meta_text("「复制原始响应」可能包含设备或账户信息，请谨慎粘贴分享。"),
        );
        if tools.history.is_empty() {
            wrapped_label(ui, meta_text("暂无任务记录；运行任意查询后在此查看响应。"));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("device-tools-history")
            .auto_shrink([false, false])
            .max_height(320.0)
            .show(ui, |ui| {
                // Newest first: the entry the user just triggered stays in view.
                for entry in tools.history.entries().iter().rev() {
                    render_history_entry(ui, entry);
                }
            });
    });
}

fn send_tool_command(
    sink: &dyn crate::app::PanelCommandSink,
    state: &mut DeviceToolsState,
    command: UiCommand,
) {
    state.notice = None;
    state.error = sink.try_send(command).err().map(send_error_text);
}

fn render_history_entry(ui: &mut Ui, entry: &ToolHistoryEntry) {
    egui::Frame::none()
        .fill(Color32::from_rgb(0xf8, 0xf9, 0xfc))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(tool_operation_text(entry.operation))
                        .strong()
                        .color(scale::INK),
                );
                ui.label(
                    RichText::new(tool_outcome_text(entry.outcome))
                        .color(tool_outcome_tone(entry.outcome).color()),
                );
                ui.label(meta_text(format!(
                    "{} · {}",
                    entry.outcome.code(),
                    format_elapsed(entry.elapsed)
                )));
                if let Some(clock) = super::clock_hms(entry.finished_at) {
                    ui.label(meta_text(clock));
                }
            });
            for line in entry.transcript.lines() {
                wrapped_label(
                    ui,
                    RichText::new(line)
                        .monospace()
                        .size(scale::META)
                        .color(scale::DETAIL),
                );
            }
            if entry.transcript.is_empty() {
                ui.label(meta_text("（无响应内容）"));
            }
            if entry.transcript.is_truncated() {
                ui.label(
                    RichText::new("响应过长，已截断")
                        .size(scale::META)
                        .color(StatusTone::Caution.color()),
                );
            }
        });
    ui.add_space(6.0);
}

/// Diagnostic summary: operation type, elapsed time and the stable outcome code only — never the
/// response text.
fn copy_diagnostic_summary(ui: &mut Ui, tools: &DeviceToolsSnapshot) {
    let mut text = String::from("设备工具历史摘要（不含响应内容）\n");
    for entry in tools.history.entries() {
        text.push_str(&format!(
            "- {} | {} | {}\n",
            tool_operation_text(entry.operation),
            entry.outcome.code(),
            format_elapsed(entry.elapsed)
        ));
    }
    ui.ctx().copy_text(text);
}

/// Raw response copy: a separate, explicitly labelled action whose output may contain device or
/// account information.
fn copy_raw_response(ui: &mut Ui, tools: &DeviceToolsSnapshot) {
    let mut text = String::new();
    for entry in tools.history.entries() {
        text.push_str(&format!(
            "### {} | {} | {}\n",
            tool_operation_text(entry.operation),
            entry.outcome.code(),
            format_elapsed(entry.elapsed)
        ));
        for line in entry.transcript.lines() {
            text.push_str(line);
            text.push('\n');
        }
        if entry.transcript.is_truncated() {
            text.push_str("[响应过长，已截断]\n");
        }
    }
    ui.ctx().copy_text(text);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use dji4g_application::{ToolCapabilityRow, ToolContext, ToolHistoryEntry, ToolTranscript};
    use dji4g_at_protocol::SensorTemperature;
    use dji4g_domain::StableDeviceIdentity;

    fn identity() -> StableDeviceIdentity {
        StableDeviceIdentity {
            container_id: "SWD\\VID_2CA3&PID_4006\\5&2A1B3C4D&0&1".to_owned(),
            device_instance_id: "USB\\VID_2CA3&PID_4006\\1234567890AB".to_owned(),
            vid: 0x2CA3,
            pid: 0x4006,
        }
    }

    fn context() -> ToolContext {
        ToolContext {
            device_epoch: DeviceEpoch(4),
            sim_epoch: 2,
            identity: identity(),
            at_port: "COM7".to_owned(),
        }
    }

    fn snapshot_with_tools(tools: DeviceToolsSnapshot) -> ControllerSnapshot {
        let mut snapshot = dji4g_application::ReducerState::new(SystemTime::UNIX_EPOCH).snapshot();
        snapshot.device_tools = tools;
        snapshot
    }

    fn tools_fixture() -> DeviceToolsSnapshot {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut tools = DeviceToolsSnapshot {
            profile: dji4g_application::ModuleProfile {
                manufacturer: Some("Quectel".to_owned()),
                model: Some("EC200A-CN".to_owned()),
                revision: Some("EC200ACNAAR02A05M08".to_owned()),
                usb_net: Some(UsbNetReading::Verified(VerifiedUsbNetProfile::DjiNdis)),
                pdp_contexts: Vec::new(),
                temperature: vec![SensorTemperature {
                    name: Some("cpu".to_owned()),
                    celsius: 42,
                }],
                observed_at: Some(now),
                context: Some(context()),
            },
            ..DeviceToolsSnapshot::default()
        };
        tools.record_capability(ToolCapabilityRow::new(
            ToolReadId::Manufacturer,
            ToolOutcome::Ok,
            context(),
            now,
        ));
        tools.record_capability(ToolCapabilityRow::empty(
            ToolReadId::SmsStorage,
            context(),
            now,
        ));
        tools.record_capability(ToolCapabilityRow::new(
            ToolReadId::ServingCell,
            ToolOutcome::TransportFailure,
            context(),
            now,
        ));
        tools.history.push(ToolHistoryEntry {
            id: 1,
            operation: ToolOperationKind::Read(ToolReadId::Manufacturer),
            outcome: ToolOutcome::Ok,
            elapsed: Duration::from_millis(420),
            finished_at: now,
            transcript: Arc::new(ToolTranscript::from_lines(vec![
                "+CGMI: \"Quectel\"".to_owned(),
                "OK".to_owned(),
            ])),
        });
        tools
    }

    fn render_into(tools: DeviceToolsSnapshot, state: &mut DeviceToolsState) {
        struct NoopSink;
        impl crate::app::PanelCommandSink for NoopSink {
            fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
                Ok(())
            }

            fn prepare_repair_now(&self, _request: ControlledRepairRequest) {}

            fn prepare_action_now(&self, _request: dji4g_application::ActionRequest) {}
        }
        let snapshot = snapshot_with_tools(tools);
        let context = egui::Context::default();
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 900.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    render(ui, &snapshot, Language::ZhCn, &NoopSink, state);
                });
            },
        );
    }

    #[test]
    fn the_masked_device_id_keeps_only_a_short_tail() {
        let masked = masked_device_id(&identity());
        assert_eq!(masked, "…&0&1");
        assert!(!masked.contains("2A1B3C4D"));
        assert_eq!(
            masked_device_id(&StableDeviceIdentity {
                container_id: String::new(),
                device_instance_id: "USB\\VID_2CA3&PID_4006\\ABCDEF".to_owned(),
                vid: 0x2CA3,
                pid: 0x4006,
            }),
            "…CDEF"
        );
    }

    #[test]
    fn known_writes_show_the_normalized_command_the_executor_runs() {
        assert_eq!(
            normalized_write_text(&ToolWriteId::RestartModule),
            "AT+CFUN=1,1"
        );
        assert_eq!(
            normalized_write_text(&ToolWriteId::SetUsbNetProfile(VerifiedUsbNetProfile::Ecm)),
            "AT+QCFG=\"usbnet\",1"
        );
        let line = ValidatedToolLine::parse("AT+CGDCONT=1,\"IP\",\"internet\"")
            .expect("the canonical CGDCONT write validates");
        match classify_known_write(&line) {
            Some(write @ ToolWriteId::SetApn { .. }) => {
                assert_eq!(
                    normalized_write_text(&write),
                    "AT+CGDCONT=1,\"IP\",\"internet\""
                );
            }
            other => panic!("expected a known APN write, got {other:?}"),
        }
    }

    #[test]
    fn every_outcome_and_input_error_has_a_closed_description() {
        for outcome in [
            ToolOutcome::Ok,
            ToolOutcome::Rejected,
            ToolOutcome::Unsupported,
            ToolOutcome::TransportFailure,
            ToolOutcome::FormatMismatch,
            ToolOutcome::CancelledBeforeWrite,
            ToolOutcome::OutcomeUnknown,
            ToolOutcome::ContextChanged,
        ] {
            assert!(!tool_outcome_text(outcome).trim().is_empty());
            assert!(!outcome.code().trim().is_empty());
        }
        for error in [
            ToolInputError::Empty,
            ToolInputError::TooLong,
            ToolInputError::NonAscii,
            ToolInputError::ControlCharacter,
            ToolInputError::ChainedCommand,
            ToolInputError::InvalidPrefix,
            ToolInputError::NotWhitelisted,
            ToolInputError::InteractiveCommand,
        ] {
            assert!(!tool_input_error_text(error).trim().is_empty());
            assert!(!error.code().trim().is_empty());
        }
        for status in [
            FeatureStatus::NotProbed,
            FeatureStatus::Supported,
            FeatureStatus::Empty,
            FeatureStatus::UnsupportedConfirmed,
            FeatureStatus::TemporarilyUnavailable,
            FeatureStatus::FormatMismatch,
            FeatureStatus::TransportFailure,
        ] {
            assert!(!feature_status_text(status).0.trim().is_empty());
        }
    }

    #[test]
    fn the_query_helper_text_names_the_advanced_terminal() {
        assert_eq!(
            ValidatedToolLine::parse_read_only("AT+CFUN=1,1"),
            Err(ToolInputError::NotWhitelisted)
        );
        assert!(tool_input_error_text(ToolInputError::NotWhitelisted).contains("白名单"));
    }

    /// Records both the plain commands and the controlled writes the page prepared, so a test can
    /// tell a refused command from a confirmed one.
    #[derive(Default)]
    struct RecordingSink {
        sent: std::sync::Mutex<Vec<UiCommand>>,
        repairs: std::sync::Mutex<Vec<ControlledRepairRequest>>,
    }

    #[test]
    fn capability_retry_sends_only_that_read_and_clear_is_a_real_command() {
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState::default();
        send_tool_command(
            &sink,
            &mut state,
            UiCommand::RunToolRead {
                id: ToolReadId::Temperature,
            },
        );
        send_tool_command(&sink, &mut state, UiCommand::ClearToolHistory);
        assert!(matches!(
            sink.sent.lock().unwrap().as_slice(),
            [
                UiCommand::RunToolRead {
                    id: ToolReadId::Temperature
                },
                UiCommand::ClearToolHistory
            ]
        ));
        assert!(sink.repairs.lock().unwrap().is_empty());
    }

    #[test]
    fn rejected_clear_reports_error_without_claiming_the_history_was_cleared() {
        struct Full;
        impl crate::app::PanelCommandSink for Full {
            fn try_send(&self, _: UiCommand) -> Result<(), UiSendError> {
                Err(UiSendError::QueueFull)
            }
            fn prepare_repair_now(&self, _: ControlledRepairRequest) {}
            fn prepare_action_now(&self, _: dji4g_application::ActionRequest) {}
        }
        let mut state = DeviceToolsState {
            notice: Some("上次提示".into()),
            ..Default::default()
        };
        send_tool_command(&Full, &mut state, UiCommand::ClearToolHistory);
        assert!(state.notice.is_none());
        assert!(state.error.as_deref().unwrap().contains("队列"));
    }

    impl crate::app::PanelCommandSink for RecordingSink {
        fn try_send(&self, command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
            self.sent.lock().expect("healthy lock").push(command);
            Ok(())
        }

        fn prepare_repair_now(&self, request: ControlledRepairRequest) {
            self.repairs.lock().expect("healthy lock").push(request);
        }

        fn prepare_action_now(&self, _request: dji4g_application::ActionRequest) {}
    }

    #[test]
    fn the_query_tab_runs_only_whitelisted_reads() {
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState {
            query_input: "AT+CSQ".to_owned(),
            ..DeviceToolsState::default()
        };
        run_query(&sink, &mut state);
        assert!(state.error.is_none());
        assert!(matches!(
            sink.sent.lock().expect("healthy lock").as_slice(),
            [UiCommand::RunToolRead {
                id: ToolReadId::SignalQuality
            }]
        ));

        // The ComboBox selection runs its preset with no free text.
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState {
            query_selected: Some(ToolReadId::SmsStorage),
            ..DeviceToolsState::default()
        };
        run_query(&sink, &mut state);
        assert!(matches!(
            sink.sent.lock().expect("healthy lock").as_slice(),
            [UiCommand::RunToolRead {
                id: ToolReadId::SmsStorage
            }]
        ));

        // A non-whitelisted line never leaves this tab; it points at the expert terminal instead.
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState {
            query_input: "AT+CFUN=1,1".to_owned(),
            ..DeviceToolsState::default()
        };
        run_query(&sink, &mut state);
        assert!(
            sink.sent.lock().expect("healthy lock").is_empty(),
            "the query tab must never dispatch a non-whitelisted line"
        );
        assert!(state.point_to_expert);
        assert!(
            state
                .error
                .as_deref()
                .is_some_and(|text| text.contains("白名单"))
        );
    }

    #[test]
    fn the_expert_tab_routes_known_writes_to_the_repair_flow() {
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState {
            expert_input: "AT+CFUN=1,1".to_owned(),
            ..DeviceToolsState::default()
        };
        submit_expert(&sink, &mut state);
        // A recognized write goes through the reviewed repair flow's own confirmation; it must not
        // be queued as a plain command, so it cannot bypass that confirmation.
        assert!(sink.sent.lock().expect("healthy lock").is_empty());
        assert!(matches!(
            sink.repairs.lock().expect("healthy lock").as_slice(),
            [ControlledRepairRequest::RestartModule]
        ));
        assert!(
            state
                .notice
                .as_deref()
                .is_some_and(|text| text.contains("AT+CFUN=1,1"))
        );

        // Anything the repair flow does not already know is frozen for its own confirmation.
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState {
            expert_input: "AT+QCFG=\"usbnet\"".to_owned(),
            ..DeviceToolsState::default()
        };
        submit_expert(&sink, &mut state);
        match sink.sent.lock().expect("healthy lock").as_slice() {
            [UiCommand::PrepareExpertTool { line }] => {
                assert_eq!(line.expose_for_confirmation(), "AT+QCFG=\"usbnet\"");
            }
            other => panic!("expected a frozen expert plan, got {other:?}"),
        }
        assert!(state.error.is_none());

        // An interactive family is refused by the parser itself and never reaches the sink.
        let sink = RecordingSink::default();
        let mut state = DeviceToolsState {
            expert_input: "AT+CMGS=12".to_owned(),
            ..DeviceToolsState::default()
        };
        submit_expert(&sink, &mut state);
        assert!(sink.sent.lock().expect("healthy lock").is_empty());
        assert!(
            state
                .error
                .as_deref()
                .is_some_and(|text| text.contains("交互"))
        );
    }

    #[test]
    fn all_three_tabs_render_without_panicking() {
        for tab in [ToolTab::Preset, ToolTab::Query, ToolTab::Expert] {
            let mut state = DeviceToolsState {
                tab,
                ..DeviceToolsState::default()
            };
            render_into(tools_fixture(), &mut state);
        }
    }

    #[test]
    fn the_expert_tab_stays_locked_until_an_explicit_unlock() {
        let mut state = DeviceToolsState::default();
        assert!(!state.expert_unlocked);
        state.expert_unlocked = true;
        assert!(state.expert_input.is_empty());
    }

    #[test]
    fn a_device_or_sim_change_relocks_the_expert_terminal() {
        let mut ready =
            dji4g_application::ReducerState::test_ready(SystemTime::UNIX_EPOCH).snapshot();
        let mut state = DeviceToolsState::default();
        state.observe_context(&ready);
        state.expert_unlocked = true;
        state.expert_input = "AT+CSQ".to_owned();
        state.apn_value = "internet".to_owned();

        state.observe_context(&ready);
        assert!(state.expert_unlocked, "the same context keeps the unlock");

        let device = ready.app.device.clone();
        ready.app = Arc::new(dji4g_domain::AppSnapshot {
            device: None,
            ..(*ready.app).clone()
        });
        state.observe_context(&ready);
        assert!(
            !state.expert_unlocked,
            "a changed context relocks the terminal"
        );
        assert!(state.expert_input.is_empty());
        assert!(state.apn_value.is_empty());

        let mut restored = ready;
        restored.app = Arc::new(dji4g_domain::AppSnapshot {
            device,
            ..(*restored.app).clone()
        });
        state.observe_context(&restored);
        assert!(
            !state.expert_unlocked,
            "returning to a device is a new context and must relock the terminal"
        );
    }
}
