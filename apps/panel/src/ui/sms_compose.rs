//! UI-owned SMS drafts. Never logged or serialized; confirmation freezes the exact payload.
use crate::app::UiCommandSink;
use dji4g_application::{SmsSendPhase, SmsSendResult, SmsSendSnapshot, UiCommand, UiSendError};
use eframe::egui::{self, Ui};

#[derive(Clone, Default, PartialEq, Eq)]
struct Draft {
    recipient: String,
    body: String,
}

struct Pending {
    draft: Draft,
    after_id: Option<u64>,
    request_id: Option<u64>,
    after_feedback: u64,
    context_changed: bool,
}

/// Content-free result of starting a reply; existing drafts require an explicit decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplyStartResult {
    Opened,
    ReplacementConfirmation,
    InvalidRecipient,
    Busy,
}

#[derive(Default)]
pub(crate) struct SmsComposeState {
    pub open: bool,
    draft: Draft,
    confirmation: Option<Draft>,
    pending: Option<Pending>,
    pub outgoing: bool,
    pub search: String,
    pub refresh_error: Option<String>,
    last_refresh_attempt: Option<std::time::Instant>,
    last_visible: Option<std::time::Instant>,
    pub selected: Option<[u8; 32]>,
    pub serial_busy: bool,
    reply_recipient: Option<String>,
    pub error: Option<String>,
    last_feedback: u64,
    context: Option<(Option<dji4g_domain::DeviceEpoch>, u64)>,
}

impl SmsComposeState {
    pub(super) fn begin_reply(&mut self, recipient: &str) -> ReplyStartResult {
        if self.serial_busy || self.pending.is_some() || self.confirmation.is_some() {
            self.error = Some("当前任务或发送确认尚未结束，草稿已保留。".into());
            return ReplyStartResult::Busy;
        }
        if dji4g_at_protocol::validate_sms_recipient(recipient).is_err() {
            self.error =
                Some("此发件人不是受支持的短信号码，无法直接回复。原号码未被修改。".into());
            return ReplyStartResult::InvalidRecipient;
        }
        self.error = None;
        if !self.draft.recipient.is_empty() || !self.draft.body.is_empty() {
            self.reply_recipient = Some(recipient.to_owned());
            ReplyStartResult::ReplacementConfirmation
        } else {
            self.draft = Draft {
                recipient: recipient.to_owned(),
                body: String::new(),
            };
            self.open = true;
            ReplyStartResult::Opened
        }
    }

    fn resolve_reply(&mut self, replace: bool) {
        let Some(recipient) = self.reply_recipient.take() else {
            return;
        };
        if replace && !self.serial_busy && self.pending.is_none() && self.confirmation.is_none() {
            self.draft = Draft {
                recipient,
                body: String::new(),
            };
            self.open = true;
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn review_reply_replace(&mut self) {
        self.review_editor();
        self.begin_reply("+8613900000000");
    }
    /// Debug-only visual fixture. This only opens a confirmation; it never dispatches a command.
    #[cfg(debug_assertions)]
    pub(crate) fn review_confirmation(&mut self) {
        self.draft = Draft {
            recipient: "+12025550123".into(),
            body: "【模拟数据·界面验收】这是一条仅用于截图的短信草稿，请勿实际发送。".into(),
        };
        self.confirmation = Some(self.draft.clone());
        self.open = true;
    }

    #[cfg(debug_assertions)]
    pub(crate) fn review_editor(&mut self) {
        self.review_confirmation();
        self.confirmation = None;
    }

    pub(super) fn request_refresh(&mut self, now: std::time::Instant, sink: &dyn UiCommandSink) {
        self.last_refresh_attempt = Some(now);
        self.refresh_error = sink
            .try_send(UiCommand::SmsRefresh)
            .err()
            .map(|e| enqueue_error(e).to_owned());
    }

    pub(super) fn auto_refresh(
        &mut self,
        now: std::time::Instant,
        available: bool,
        busy: bool,
        query_pending: bool,
        sink: &dyn UiCommandSink,
    ) {
        // Entering the page refreshes immediately; ordinary render frames never enqueue duplicates.
        if self.last_visible.is_some_and(|seen| {
            now.saturating_duration_since(seen) > std::time::Duration::from_secs(2)
        }) {
            self.last_refresh_attempt = None;
        }
        self.last_visible = Some(now);
        if !available || busy || query_pending || self.pending.is_some() {
            return;
        }
        if self.last_refresh_attempt.is_none_or(|at| {
            now.saturating_duration_since(at) >= std::time::Duration::from_secs(15)
        }) {
            self.request_refresh(now, sink);
        }
    }

    fn ready(&self) -> bool {
        dji4g_at_protocol::build_ucs2_submit(&self.draft.recipient, &self.draft.body).is_ok()
    }

    fn edited(&mut self) {
        self.confirmation = None;
    }

    fn confirm(&mut self, sink: &dyn UiCommandSink, snapshot: Option<&SmsSendSnapshot>) {
        if self.pending.is_some() || snapshot.is_some_and(|s| s.phase != SmsSendPhase::Finished) {
            return;
        }
        let Some(frozen) = self.confirmation.take() else {
            return;
        };
        if frozen != self.draft || !self.ready() {
            return;
        }
        match sink.try_send(UiCommand::SmsSend {
            recipient: frozen.recipient.clone(),
            body: frozen.body.clone(),
        }) {
            Ok(()) => {
                self.error = None;
                self.pending = Some(Pending {
                    draft: frozen,
                    after_id: snapshot.map(|s| s.request_id),
                    request_id: None,
                    after_feedback: self.last_feedback,
                    context_changed: false,
                });
            }
            Err(error) => self.error = Some(enqueue_error(error).to_owned()),
        }
    }

    pub fn synchronize(&mut self, snapshot: Option<&SmsSendSnapshot>) {
        let (Some(pending), Some(snapshot)) = (&mut self.pending, snapshot) else {
            return;
        };
        if pending.request_id.is_none()
            && pending.after_id.is_none_or(|id| snapshot.request_id > id)
        {
            pending.request_id = Some(snapshot.request_id);
        }
        if pending.request_id != Some(snapshot.request_id)
            || snapshot.phase != SmsSendPhase::Finished
        {
            return;
        }
        if snapshot.result == Some(SmsSendResult::Submitted)
            && self.draft == pending.draft
            && !pending.context_changed
        {
            self.draft = Draft::default();
            self.open = false;
        }
        self.pending = None;
    }

    fn observe_context(&mut self, context: (Option<dji4g_domain::DeviceEpoch>, u64)) {
        if self.context.is_some_and(|old| old != context) {
            self.confirmation = None;
            self.reply_recipient = None;
            self.selected = None;
            self.last_refresh_attempt = None;
            if let Some(pending) = &mut self.pending {
                pending.context_changed = true;
            }
        }
        self.context = Some(context);
    }

    fn observe_rejection(&mut self, seq: u64, code: &str) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request_id.is_none() && seq > pending.after_feedback)
            && code == "sms:busy"
        {
            self.pending = None;
            self.error =
                Some("后台正忙，本条短信未提交。请等待当前任务结束后重试；草稿已保留。".into());
        }
        self.last_feedback = self.last_feedback.max(seq);
    }
}

pub(super) fn enqueue_error(error: UiSendError) -> &'static str {
    match error {
        UiSendError::QueueFull => "操作队列已满，未提交。请稍后重试；草稿已保留。",
        UiSendError::Closed => "后台连接已关闭，未提交。请恢复连接后重试；草稿已保留。",
    }
}

pub(super) fn render(
    ui: &mut Ui,
    state: &mut SmsComposeState,
    controller: &dji4g_application::ControllerSnapshot,
    sink: &dyn UiCommandSink,
) {
    let snapshot = controller.sms_send.as_ref();
    state.observe_context((
        controller.app.device.as_ref().map(|device| device.epoch),
        controller.sim_epoch,
    ));
    state.synchronize(snapshot);
    if let Some(feedback) = &controller.feedback {
        state.observe_rejection(feedback.seq, feedback.code.stable.as_str());
    }
    state.serial_busy = controller.serial_work_busy;
    let busy = state.serial_busy
        || state.pending.is_some()
        || snapshot.is_some_and(|s| s.phase != SmsSendPhase::Finished);
    if let Some(snapshot) = snapshot {
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(0xf3, 0xf5, 0xfc))
            .rounding(10.0)
            .inner_margin(14.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let text = match snapshot.phase {
                    SmsSendPhase::Queued => "短信已排队，请勿重复发送",
                    SmsSendPhase::Preparing => "正在准备短信",
                    SmsSendPhase::Submitting => "正在提交短信",
                    SmsSendPhase::WaitingForResult => "正在等待模块确认",
                    SmsSendPhase::Finished => match snapshot.result {
                        Some(SmsSendResult::Submitted) => "已提交给模块，尚不能确认对方收到。",
                        Some(SmsSendResult::Failed) => "发送失败，草稿已保留。",
                        _ => "发送结果未知，可能已提交。请先核实，避免重复发送；草稿已保留。",
                    },
                };
                ui.horizontal_wrapped(|ui| {
                    let tone = send_result_tone(snapshot);
                    ui.label(
                        egui::RichText::new(format!("{} {text}", tone.marker()))
                            .color(tone.color())
                            .strong(),
                    );
                    if let Some(failure) = &snapshot.failure {
                        if let Some(code) = failure.cms_code {
                            ui.label(crate::ui::meta_text(format!("CMS {code}")));
                        }
                        if let Some(code) = failure.cme_code {
                            ui.label(crate::ui::meta_text(format!("CME {code}")));
                        }
                    }
                });
                if let Some(failure) = &snapshot.failure {
                    egui::CollapsingHeader::new("查看原因和处理建议").show(ui, |ui| {
                        ui.label(format!(
                            "失败阶段：{} · 错误码：{}",
                            phase_name(failure.stage),
                            failure.code
                        ));
                        ui.label(failure_advice(&failure.code));
                        if let Some(code) = failure.cms_code {
                            ui.label(format!("CMS：{code}"));
                        }
                        if let Some(code) = failure.cme_code {
                            ui.label(format!("CME：{code}"));
                        }
                        if let Some(code) = failure.os_code {
                            ui.label(format!("系统错误：{code}"));
                        }
                        ui.label(submission_notice(snapshot.result, failure));
                    });
                }
            });
    } else if busy {
        ui.label("短信已排队，请勿重复发送");
    }
    if let Some(error) = &state.error {
        ui.colored_label(super::StatusTone::Negative.color(), error);
    }
    if state.open && state.confirmation.is_none() {
        let mut opened = true;
        egui::Window::new("新建短信")
            .id(egui::Id::new("sms-compose-window"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(500.0)
            .max_width((ui.ctx().screen_rect().width() - 48.0).max(260.0))
            .collapsible(false)
            .resizable(false)
            .vscroll(false)
            .open(&mut opened)
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                egui::ScrollArea::vertical()
                    .max_height((ui.ctx().screen_rect().height() - 210.0).clamp(240.0, 420.0))
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            egui::Frame::none()
                                .fill(egui::Color32::from_rgb(0xec, 0xef, 0xff))
                                .rounding(12.0)
                                .inner_margin(12.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        crate::ui::icons::text(
                                            ui.ctx(),
                                            crate::ui::icons::EDIT,
                                            24.0,
                                        )
                                        .color(crate::ui::scale::DOWNLOAD),
                                    );
                                });
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new("写一条短信").size(20.0).strong());
                                ui.label(crate::ui::meta_text("通过当前连接的 4G 模块发送"));
                            });
                        });
                        ui.add_space(18.0);
                        ui.label(egui::RichText::new("收件人").strong());
                        let recipient = ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut state.draft.recipient)
                                .hint_text("+86 手机号码")
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(12.0, 10.0)),
                        );
                        ui.label(crate::ui::meta_text(
                            "请输入含国家码的完整号码，例如 +8613800138000",
                        ));
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("短信内容").strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(crate::ui::meta_text(format!(
                                        "{} / 70 字",
                                        state.draft.body.chars().count()
                                    )));
                                },
                            );
                        });
                        let body = ui.add_enabled(
                            !busy,
                            egui::TextEdit::multiline(&mut state.draft.body)
                                .hint_text("在这里输入短信内容…")
                                .desired_width(f32::INFINITY)
                                .desired_rows(4)
                                .margin(egui::vec2(12.0, 12.0)),
                        );
                        if recipient.changed() || body.changed() {
                            state.edited();
                        }
                        ui.add_space(10.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(0xf5, 0xf7, 0xfb))
                            .rounding(8.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.label(crate::ui::meta_text(
                                    "单条短信 · 最多 70 字 · 不支持 Emoji",
                                ));
                                ui.label(crate::ui::meta_text(
                                    "可能产生运营商费用；下一步将核对号码和正文。",
                                ));
                            });
                    });
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::ui::meta_text(if busy {
                        "正在发送，请等待结果"
                    } else if !state.ready() {
                        "填写有效号码和内容后即可继续"
                    } else {
                        "草稿已就绪"
                    }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                state.ready() && !busy,
                                crate::ui::theme::primary_button("下一步：确认发送"),
                            )
                            .clicked()
                        {
                            state.confirmation = Some(state.draft.clone());
                        }
                    });
                });
                ui.add_space(6.0);
            });
        state.open = opened;
    }
    if let Some(frozen) = state.confirmation.clone() {
        let mut open = true;
        egui::Window::new("确认发送短信")
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(460.0)
            .max_width((ui.ctx().screen_rect().width() - 48.0).min(460.0))
            .default_height(260.0)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label("请核对以下完整号码和正文：");
                ui.label(egui::RichText::new(&frozen.recipient).strong());
                ui.separator();
                ui.add(egui::Label::new(&frozen.body).wrap());
                ui.separator();
                ui.add(
                    egui::Label::new(
                        "本次发送 1 条短信，可能产生运营商费用。模块接受不代表对方收到。",
                    )
                    .wrap(),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("取消").clicked() {
                        state.confirmation = None;
                    }
                    if ui
                        .add_enabled(!busy, crate::ui::theme::primary_button("确认发送这条短信"))
                        .clicked()
                    {
                        state.confirm(sink, snapshot);
                    }
                });
            });
        if !open {
            state.confirmation = None;
        }
    }
    if state.reply_recipient.is_some() {
        let mut open = true;
        egui::Window::new("保留当前草稿？")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label("已有未发送草稿。默认保留；替换后将只填写回复号码，正文为空。");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("保留草稿").clicked() {
                        state.resolve_reply(false);
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("替换为回复草稿"))
                        .clicked()
                    {
                        state.resolve_reply(true);
                    }
                });
            });
        if !open {
            state.resolve_reply(false);
        }
    }
}

fn send_result_tone(snapshot: &SmsSendSnapshot) -> super::StatusTone {
    if snapshot.phase != SmsSendPhase::Finished {
        return super::StatusTone::Progress;
    }
    match snapshot.result {
        Some(SmsSendResult::Submitted) => super::StatusTone::Positive,
        Some(SmsSendResult::Failed) => super::StatusTone::Negative,
        Some(SmsSendResult::OutcomeUnknown) | None => super::StatusTone::Caution,
    }
}

fn submission_notice(
    result: Option<SmsSendResult>,
    failure: &dji4g_application::SmsFailureDetail,
) -> &'static str {
    if result == Some(SmsSendResult::Failed) && failure.code == "sms:module_rejected" {
        "模块已明确拒绝本条短信，未接受提交。请根据错误码排查后再手动发送。"
    } else if failure.submission_possible {
        "可能已经提交，请勿直接重发。"
    } else {
        "本次未提交，可检查连接、SIM 卡和短信服务后重试。"
    }
}

fn phase_name(phase: SmsSendPhase) -> &'static str {
    match phase {
        SmsSendPhase::Queued => "排队",
        SmsSendPhase::Preparing => "准备",
        SmsSendPhase::Submitting => "提交",
        SmsSendPhase::WaitingForResult => "等待模块结果",
        SmsSendPhase::Finished => "完成",
    }
}

fn failure_advice(code: &str) -> &'static str {
    match code {
        "sms:port_busy" => return "串口正被其他任务占用。请等待任务结束，再手动重试。",
        "sms:port_open_failed" => return "无法打开短信串口。请检查设备连接及其他串口程序是否占用。",
        "sms:cleanup_timeout" => {
            return "串口关闭超时，后台未能确认资源已释放。请恢复设备连接后再操作。";
        }
        "sms:no_device" => return "当前没有可用设备。请连接模块并刷新设备状态。",
        "sms:device_changed" => {
            return "发送过程中设备或 SIM 卡发生变化。请核对当前设备及发送记录。";
        }
        "sms:module_rejected" => {
            return "模块拒绝了短信。请结合 CMS/CME 错误码检查 SIM 卡、余额及运营商短信服务。";
        }
        "sms:transport_failed" => return "串口通信失败。请检查 USB 连接和模块供电。",
        "sms:timeout" => return "等待模块响应超时。请核实发送记录及连接状态，避免重复发送。",
        "sms:missing_reference" => {
            return "模块没有返回短信提交编号，无法确认提交结果。请先核实是否已发送。";
        }
        "sms:unexpected_final_code" => {
            return "模块返回了非预期的结束响应，无法确认提交结果。请保留错误码并核实发送情况。";
        }
        _ => {}
    }
    if code.contains("lease") || code.contains("busy") {
        "串口正在被其他任务占用，请等待任务结束。"
    } else if code.contains("open") || code.contains("access") {
        "无法打开串口，请检查设备连接及端口占用。"
    } else if code.contains("timeout") || code.contains("deadline") {
        "等待模块响应超时，请检查连接及模块状态。"
    } else if code.contains("invalid") {
        "号码或正文未通过校验，请检查国际号码和正文长度。"
    } else if code.contains("disconnect") || code.contains("closed") || code.contains("removed") {
        "设备连接中断，请重新连接设备并刷新。"
    } else {
        "请检查设备连接、SIM 卡状态和运营商短信服务；保留错误码以便排查。"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replying_only_prefills_the_original_recipient() {
        let mut state = SmsComposeState::default();
        assert_eq!(
            state.begin_reply("+8613800138000"),
            ReplyStartResult::Opened
        );
        assert_eq!(state.draft.recipient, "+8613800138000");
        assert!(state.draft.body.is_empty());
        assert!(state.open);
        assert!(state.confirmation.is_none());
        assert!(state.pending.is_none());
    }
    #[test]
    fn existing_draft_is_kept_until_explicit_replacement() {
        let mut state = ready();
        state.confirmation = None;
        let original = state.draft.clone();
        assert_eq!(
            state.begin_reply("+8613900000000"),
            ReplyStartResult::ReplacementConfirmation
        );
        assert!(state.draft == original);
        state.resolve_reply(false);
        assert!(state.draft == original);
        state.begin_reply("+8613900000000");
        state.resolve_reply(true);
        assert_eq!(state.draft.recipient, "+8613900000000");
        assert!(state.draft.body.is_empty());
    }
    #[test]
    fn invalid_sender_and_busy_or_frozen_send_cannot_replace_draft() {
        let mut state = ready();
        let original = state.draft.clone();
        assert_eq!(state.begin_reply("+8613900000000"), ReplyStartResult::Busy);
        assert!(state.draft == original);
        assert!(state.confirmation.is_some());
        state.confirmation = None;
        assert_eq!(
            state.begin_reply("BANK"),
            ReplyStartResult::InvalidRecipient
        );
        assert!(state.reply_recipient.is_none());
        state.serial_busy = true;
        assert_eq!(state.begin_reply("+8613900000000"), ReplyStartResult::Busy);
        assert!(state.reply_recipient.is_none());
        assert!(state.draft == original);
    }

    #[test]
    fn service_and_national_numbers_are_not_normalized_into_reply_recipients() {
        for recipient in [
            "10086",
            "10690000",
            "13800138000",
            "BANK",
            " +8613800138000",
        ] {
            let mut state = SmsComposeState::default();
            assert_eq!(
                state.begin_reply(recipient),
                ReplyStartResult::InvalidRecipient
            );
            assert!(state.draft.recipient.is_empty());
            assert!(state.reply_recipient.is_none());
            assert!(!state.open);
        }
    }

    #[test]
    fn device_or_sim_change_clears_waiting_reply_replacement_and_preserves_draft() {
        for next in [
            (Some(dji4g_domain::DeviceEpoch(2)), 0),
            (Some(dji4g_domain::DeviceEpoch(1)), 1),
        ] {
            let mut state = ready();
            state.confirmation = None;
            state.observe_context((Some(dji4g_domain::DeviceEpoch(1)), 0));
            let original = state.draft.clone();
            assert_eq!(
                state.begin_reply("+8613900000000"),
                ReplyStartResult::ReplacementConfirmation
            );
            state.observe_context(next);
            assert!(state.reply_recipient.is_none());
            state.resolve_reply(true);
            assert!(state.draft == original);
            assert!(state.confirmation.is_none());
        }
    }

    #[test]
    fn replaced_reply_sends_only_the_exact_frozen_recipient_and_body() {
        struct Capture(std::sync::Mutex<Vec<(String, String)>>);
        impl UiCommandSink for Capture {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                if let UiCommand::SmsSend { recipient, body } = command {
                    self.0.lock().unwrap().push((recipient, body));
                }
                Ok(())
            }
        }
        let sink = Capture(std::sync::Mutex::new(Vec::new()));
        for changed_field in [None, Some("recipient"), Some("body")] {
            let mut state = ready();
            state.confirmation = None;
            assert_eq!(
                state.begin_reply("+8613900000000"),
                ReplyStartResult::ReplacementConfirmation
            );
            state.resolve_reply(true);
            state.draft.body = "明确确认的回复".into();
            state.confirmation = Some(state.draft.clone());
            match changed_field {
                Some("recipient") => state.draft.recipient = "+8613700000000".into(),
                Some("body") => state.draft.body.push('新'),
                _ => {}
            }
            state.confirm(&sink, None);
            if changed_field.is_none() {
                assert_eq!(state.begin_reply("+8613600000000"), ReplyStartResult::Busy);
            } else {
                assert!(state.pending.is_none());
            }
        }
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec![("+8613900000000".to_owned(), "明确确认的回复".to_owned())]
        );
    }
    #[test]
    fn sending_and_three_results_have_distinct_tones() {
        assert_eq!(
            send_result_tone(&done(1, SmsSendResult::Submitted)),
            super::super::StatusTone::Positive
        );
        assert_eq!(
            send_result_tone(&done(1, SmsSendResult::Failed)),
            super::super::StatusTone::Negative
        );
        assert_eq!(
            send_result_tone(&done(1, SmsSendResult::OutcomeUnknown)),
            super::super::StatusTone::Caution
        );
        let mut sending = done(1, SmsSendResult::Submitted);
        sending.phase = SmsSendPhase::WaitingForResult;
        sending.result = None;
        assert_eq!(
            send_result_tone(&sending),
            super::super::StatusTone::Progress
        );
    }
    #[test]
    fn explicit_module_rejection_overrides_body_write_uncertainty_in_notice() {
        let failure = dji4g_application::SmsFailureDetail::new(
            SmsSendPhase::WaitingForResult,
            "sms:module_rejected",
            true,
        );
        assert!(submission_notice(Some(SmsSendResult::Failed), &failure).contains("明确拒绝"));
        assert!(failure.submission_possible);
        assert!(
            submission_notice(Some(SmsSendResult::OutcomeUnknown), &failure)
                .contains("可能已经提交")
        );
    }
    struct Sink(Result<(), UiSendError>);
    impl UiCommandSink for Sink {
        fn try_send(&self, _: UiCommand) -> Result<(), UiSendError> {
            self.0.clone()
        }
    }
    fn ready() -> SmsComposeState {
        let mut state = SmsComposeState::default();
        state.draft = Draft {
            recipient: "+8613800138000".into(),
            body: "测试".into(),
        };
        state.confirmation = Some(state.draft.clone());
        state
    }
    #[test]
    fn edit_invalidates_confirmation() {
        let mut s = ready();
        s.draft.body.push('新');
        s.edited();
        assert!(s.confirmation.is_none());
    }
    #[test]
    fn queue_errors_preserve_draft_and_unlock() {
        for error in [UiSendError::QueueFull, UiSendError::Closed] {
            let mut s = ready();
            let draft = s.draft.clone();
            s.confirm(&Sink(Err(error)), None);
            assert!(s.draft == draft);
            assert!(s.pending.is_none());
            assert!(s.error.is_some());
        }
    }
    #[test]
    fn acceptance_locks_and_preserves_draft() {
        let mut s = ready();
        s.confirm(&Sink(Ok(())), None);
        assert!(s.pending.is_some());
        assert_eq!(s.draft.body, "测试");
    }
    #[test]
    fn shared_validation_rejects_invalid_payload() {
        let mut s = ready();
        s.draft.recipient = "123".into();
        assert!(!s.ready());
        s = ready();
        s.draft.body = "😀".into();
        assert!(!s.ready());
    }
    fn done(id: u64, result: SmsSendResult) -> SmsSendSnapshot {
        SmsSendSnapshot {
            request_id: id,
            phase: SmsSendPhase::Finished,
            result: Some(result),
            failure: None,
        }
    }
    #[test]
    fn only_new_matching_success_clears_original_draft() {
        let old = done(8, SmsSendResult::Submitted);
        let mut s = ready();
        s.confirm(&Sink(Ok(())), Some(&old));
        s.synchronize(Some(&old));
        assert!(s.pending.is_some());
        assert_eq!(s.draft.body, "测试");
        s.synchronize(Some(&done(9, SmsSendResult::Submitted)));
        assert!(s.pending.is_none());
        assert!(s.draft.body.is_empty());
    }
    #[test]
    fn edits_during_send_survive_success() {
        let mut s = ready();
        s.confirm(&Sink(Ok(())), None);
        s.draft.body = "下一条".into();
        s.synchronize(Some(&done(1, SmsSendResult::Submitted)));
        assert_eq!(s.draft.body, "下一条");
    }
    #[test]
    fn failure_and_unknown_preserve_original_draft() {
        for result in [SmsSendResult::Failed, SmsSendResult::OutcomeUnknown] {
            let mut s = ready();
            s.confirm(&Sink(Ok(())), None);
            s.synchronize(Some(&done(1, result)));
            assert_eq!(s.draft.body, "测试");
            assert!(s.pending.is_none());
        }
    }
    #[test]
    fn duplicate_confirmation_does_not_enqueue_twice() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(AtomicUsize);
        impl UiCommandSink for Counting {
            fn try_send(&self, _: UiCommand) -> Result<(), UiSendError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        let sink = Counting(AtomicUsize::new(0));
        let mut s = ready();
        s.confirm(&sink, None);
        s.confirmation = Some(s.draft.clone());
        s.confirm(&sink, None);
        assert_eq!(sink.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn newer_command_rejection_unlocks_and_keeps_draft() {
        let mut s = ready();
        s.observe_rejection(4, "sms:busy");
        s.confirm(&Sink(Ok(())), None);
        s.observe_rejection(4, "sms:busy");
        assert!(s.pending.is_some());
        s.observe_rejection(5, "other:failure");
        assert!(s.pending.is_some());
        s.observe_rejection(6, "sms:busy");
        assert!(s.pending.is_none());
        assert_eq!(s.draft.body, "测试");
        assert!(s.error.is_some());
    }

    #[test]
    fn unrelated_rejection_does_not_unlock_bound_send() {
        let mut s = ready();
        s.confirm(&Sink(Ok(())), None);
        let active = SmsSendSnapshot {
            request_id: 1,
            phase: SmsSendPhase::Preparing,
            result: None,
            failure: None,
        };
        s.synchronize(Some(&active));
        s.observe_rejection(1, "sms:busy");
        assert!(s.pending.is_some());
    }

    #[test]
    fn stale_device_success_does_not_clear_draft() {
        let mut s = ready();
        s.observe_context((Some(dji4g_domain::DeviceEpoch(1)), 0));
        s.confirm(&Sink(Ok(())), None);
        s.observe_context((Some(dji4g_domain::DeviceEpoch(2)), 0));
        s.synchronize(Some(&done(1, SmsSendResult::Submitted)));
        assert_eq!(s.draft.body, "测试");
        assert!(s.pending.is_none());
    }

    #[test]
    fn exact_frozen_payload_is_sent_and_unannounced_edit_is_rejected() {
        use std::sync::Mutex;
        struct Capture(Mutex<Vec<(String, String)>>);
        impl UiCommandSink for Capture {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                if let UiCommand::SmsSend { recipient, body } = command {
                    self.0.lock().unwrap().push((recipient, body));
                }
                Ok(())
            }
        }
        let sink = Capture(Mutex::new(Vec::new()));
        let mut s = ready();
        s.confirm(&sink, None);
        assert_eq!(
            sink.0.lock().unwrap().as_slice(),
            &[("+8613800138000".to_owned(), "测试".to_owned())]
        );
        let mut changed = ready();
        changed.draft.body.push('新');
        changed.confirm(&sink, None);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }
    #[test]
    fn automatic_refresh_is_bounded_and_keeps_draft_selection() {
        struct RefreshSink(std::sync::atomic::AtomicUsize);
        impl UiCommandSink for RefreshSink {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                assert!(matches!(command, UiCommand::SmsRefresh));
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        let sink = RefreshSink(std::sync::atomic::AtomicUsize::new(0));
        let now = std::time::Instant::now();
        let mut state = ready();
        state.selected = Some([7; 32]);
        let draft = state.draft.clone();
        state.auto_refresh(now, false, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        state.auto_refresh(now, true, false, false, &sink);
        state.auto_refresh(now, true, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let later = now + std::time::Duration::from_secs(16);
        state.auto_refresh(later, true, true, false, &sink);
        state.auto_refresh(later, true, false, true, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        state.auto_refresh(later, true, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(state.draft == draft);
        assert_eq!(state.selected, Some([7; 32]));
    }
}
