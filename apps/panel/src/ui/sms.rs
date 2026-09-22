//! SMS page: aggregate inbox state from the snapshot plus the stored-message list.
//!
//! `ControllerSnapshot` carries the merged message view (`sms_messages`, a read-only copy whose
//! `SmsMessage` redacts sender/body in `Debug`/`Serialize`); this page only projects it for
//! display, masks senders by default, and never logs or exports message content.
//!
//! Layout: inbox/outgoing filters share a responsive list/detail view below a compact stats strip;
//! the New SMS action opens a UI-owned draft with an explicit frozen confirmation. Outgoing records are local
//! bookkeeping — their `index` is a locally assigned transaction id, not a module storage index,
//! so their rows never issue module commands (no `SmsRead`, no `SmsDelete`) and only present the
//! submission status.

use std::time::{Duration, Instant};
#[path = "sms_compose.rs"]
mod compose;
pub(crate) use compose::SmsComposeState;

use dji4g_application::{ControllerSnapshot, UiCommand};
use dji4g_domain::{
    FeatureStatus, SmsDeleteItemResult, SmsDirection, SmsDisplayMessage, SmsEncoding,
    SmsFragmentKey, SmsMessage, SmsStatus,
};
use eframe::egui::{self, RichText, Ui};

use super::{StatusTone, meta_text, scale, sms_layout, wrapped_label};
use crate::app::UiCommandSink;
use crate::localization::{
    Language, LocalizedText, TextArgs, TextKey, feature_status_note, format_text_in,
};

/// One projected stored message. Sender and body are carried verbatim only inside this UI-local
/// value; the list projection never feeds a log, a diagnostic export, or any serialized document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsRowVm {
    pub index: u32,
    pub stable_id: [u8; 32],
    pub fragments: Vec<SmsFragmentKey>,
    pub delete_allowed: bool,
    /// `Some(true)` unread, `Some(false)` read, `None` while the read state is unknown.
    pub unread: Option<bool>,
    pub sender_masked: String,
    /// Full sender, only revealed after an explicit row click.
    pub sender_full: String,
    pub body: String,
    pub timestamp: Option<String>,
    pub encoding: SmsEncoding,
    /// `(sequence, total)` of a long message, when the PDU carried a concatenation header.
    pub multipart: Option<(u8, u8)>,
    pub status: SmsStatus,
    /// Outgoing rows are local submission records, not module-stored mail.
    pub direction: SmsDirection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsVm {
    pub title: LocalizedText,
    pub intro: LocalizedText,
    pub status: LocalizedText,
    pub message_count: usize,
    pub unread_count: usize,
    pub capacity: Option<(u32, u32)>,
    pub capacity_text: Option<LocalizedText>,
    pub has_incomplete: bool,
    /// Local-cache evictions reported by the store; non-zero means the local view is
    /// incomplete even though the module may still hold the messages.
    pub evicted: u32,
    pub incomplete_warning: LocalizedText,
    pub empty_text: LocalizedText,
    pub list_pending_text: LocalizedText,
    pub rows: Vec<SmsRowVm>,
}

/// Classification text for the inbox-wide [`FeatureStatus`]: a never-probed store reads
/// 「尚未查询」, a completed read reads 「已读取」, and every classified failure keeps its
/// precise note so 「没有数据」 and 「查询失败」 are never confused.
#[must_use]
pub fn sms_status_text(status: FeatureStatus, language: Language) -> LocalizedText {
    match status {
        FeatureStatus::NotProbed => LocalizedText::new(language, TextKey::SmsStatusNotQueried),
        FeatureStatus::Supported | FeatureStatus::Empty => {
            LocalizedText::new(language, TextKey::SmsStatusRead)
        }
        classified => feature_status_note(classified).map_or_else(
            || LocalizedText::new(language, TextKey::SmsStatusNotQueried),
            |key| LocalizedText::new(language, key),
        ),
    }
}

/// Encoding display vocabulary: GSM7/UCS2 are protocol names and stay verbatim; anything the
/// codec does not model is the localized 「其他」.
#[must_use]
pub fn sms_encoding_text(encoding: SmsEncoding, language: Language) -> String {
    match encoding {
        SmsEncoding::Gsm7 => "GSM7".to_owned(),
        SmsEncoding::Ucs2 => "UCS2".to_owned(),
        SmsEncoding::Other => LocalizedText::new(language, TextKey::SmsEncodingOther).text,
    }
}

/// One presentation tag on a row's preview line. `tone` stays neutral unless the message carries
/// a real problem (an incomplete long message), so warning colour keeps its meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsRowTag {
    pub text: String,
    pub tone: StatusTone,
}

/// Read-state presentation for the row's leading dot: a filled progress dot only for unread mail,
/// a hollow neutral one for read or unknown state.
#[must_use]
pub fn row_read_state(unread: Option<bool>) -> (StatusTone, &'static str, TextKey) {
    match unread {
        Some(true) => (StatusTone::Progress, "●", TextKey::SmsUnread),
        Some(false) => (StatusTone::Neutral, "○", TextKey::SmsRead),
        None => (StatusTone::Neutral, "○", TextKey::ValueUnknown),
    }
}

/// Submission status of one outgoing record: the only thing the module proved about it.
/// `Received`/`Incomplete` are inbox vocabulary and are never produced for outgoing records;
/// the impossible combination stays honestly 「未知」.
#[must_use]
pub fn outgoing_state(status: SmsStatus) -> (StatusTone, TextKey) {
    match status {
        SmsStatus::Submitted => (StatusTone::Positive, TextKey::SmsOutgoingSubmitted),
        SmsStatus::Failed => (StatusTone::Negative, TextKey::SmsOutgoingFailed),
        SmsStatus::OutcomeUnknown | SmsStatus::Received | SmsStatus::Incomplete => {
            (StatusTone::Caution, TextKey::SmsOutgoingUnknown)
        }
    }
}

#[cfg(test)]
#[test]
fn an_unknown_submission_is_a_caution_not_a_neutral_receipt() {
    assert_eq!(
        outgoing_state(SmsStatus::OutcomeUnknown).0,
        StatusTone::Caution
    );
}

/// Right-aligned vocabulary of one row, ordered left to right: encoding, fragment count, then the
/// incomplete warning. Only the warning carries a warning tone. Outgoing rows carry none of it:
/// the send receipt reports no encoding, concatenation headers never apply, and the status label
/// in the row's first line already says the rest.
#[must_use]
pub fn row_tags(row: &SmsRowVm, language: Language) -> Vec<SmsRowTag> {
    if row.direction == SmsDirection::Outgoing {
        return Vec::new();
    }
    let mut tags = vec![SmsRowTag {
        text: sms_encoding_text(row.encoding, language),
        tone: StatusTone::Neutral,
    }];
    if let Some((_, total)) = row.multipart {
        tags.push(SmsRowTag {
            text: format!("已读取 {} / {total} 个分片", row.fragments.len()),
            tone: StatusTone::Neutral,
        });
    }
    if row.status == SmsStatus::Incomplete {
        tags.push(SmsRowTag {
            text: LocalizedText::new(language, TextKey::SmsIncompleteTag).text,
            tone: StatusTone::Caution,
        });
    }
    tags
}

/// Project one stored message. The sender defaults to its masked form; the full sender and body
/// are placed in the VM so an explicit row click can reveal/copy them in place.
#[must_use]
pub fn sms_row_vm(message: &SmsMessage, _language: Language) -> SmsRowVm {
    let fragments = if message.direction == SmsDirection::Incoming {
        vec![message.fragment_key()]
    } else {
        Vec::new()
    };
    let display = SmsDisplayMessage {
        message: message.clone(),
        fragments: fragments.clone(),
        delete_allowed: message.direction == SmsDirection::Incoming,
    };
    SmsRowVm {
        index: message.index,
        stable_id: display.stable_id(),
        fragments,
        delete_allowed: display.delete_allowed,
        unread: message.read.map(|read| !read),
        sender_masked: message.sender_masked(),
        sender_full: message.sender().to_owned(),
        body: message.body().to_owned(),
        timestamp: message.service_centre_timestamp.clone(),
        encoding: message.encoding,
        multipart: message
            .multipart
            .map(|multipart| (multipart.sequence, multipart.total)),
        status: message.status,
        direction: message.direction,
    }
}

fn display_row_vm(message: &SmsDisplayMessage, language: Language) -> SmsRowVm {
    let mut row = sms_row_vm(&message.message, language);
    row.stable_id = message.stable_id();
    row.fragments = message.fragments.clone();
    row.delete_allowed = message.delete_allowed;
    row
}

#[must_use]
pub fn sms_vm(snapshot: &ControllerSnapshot, messages: &[SmsMessage], language: Language) -> SmsVm {
    let summary = &snapshot.sms_inbox;
    let mut status = sms_status_text(summary.status, language);
    if let Some(error) = &snapshot.sms_inbox_failure {
        let reason = match error.code.stable().as_str() {
            "sms:port_busy" => "串口上一个操作尚未结束，请稍后刷新",
            "sms:port_open_failed" | "sms:permission_denied" => "串口访问失败",
            "sms:unsupported" => "未找到可验证的短信 AT 串口",
            "sms:pdu_mode_required" | "sms:pdu_confirm_failed" => "短信 PDU 模式未确认",
            "sms:verification_failed" => "模块响应未通过验证",
            "sms:timeout" | "app:stage_timeout" => "短信查询超时",
            "sms:no_device" => "无已验证的设备，请先检查概览中的模块连接状态",
            "sms:device_removed" => "设备已断开",
            _ => "短信查询失败",
        };
        status.text = format!("{reason}（{}）", error.code.stable().as_str());
        if let Some(code) = error.os_code {
            status.text.push_str(&format!("；系统错误 {code}"));
        }
    }
    SmsVm {
        title: LocalizedText::new(language, TextKey::SmsTitle),
        intro: LocalizedText::new(language, TextKey::SmsIntro),
        status,
        message_count: summary.message_count,
        unread_count: summary.unread_count,
        capacity: summary.capacity,
        capacity_text: summary.capacity.map(|(used, total)| {
            format_text_in(
                language,
                TextKey::SmsCapacityUsed,
                &TextArgs::used_total(used, total),
            )
        }),
        has_incomplete: summary.has_incomplete,
        evicted: summary.evicted,
        incomplete_warning: LocalizedText::new(language, TextKey::SmsIncompleteWarning),
        empty_text: LocalizedText::new(language, TextKey::SmsEmpty),
        list_pending_text: LocalizedText::new(language, TextKey::SmsListPending),
        rows: messages
            .iter()
            .map(|message| sms_row_vm(message, language))
            .collect(),
    }
}

/// How long a delete confirmation stays armed. Sending uses a persistent confirmation window.
pub(crate) const CONFIRM_ARM_WINDOW: Duration = Duration::from_secs(3);

/// The height the workspace really received this frame. Recorded in egui's temp memory so the
/// layout tests can observe the rendered geometry instead of re-deriving it from the arithmetic.
fn workspace_height_id() -> egui::Id {
    egui::Id::new("sms-workspace-height")
}

/// The height the message list really received inside its column, after the search field.
fn list_viewport_id() -> egui::Id {
    egui::Id::new("sms-list-viewport")
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    messages: &[SmsDisplayMessage],
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    let mut vm = sms_vm(snapshot, &[], language);
    vm.rows = messages
        .iter()
        .map(|message| display_row_vm(message, language))
        .collect();
    state.serial_busy = snapshot.serial_work_busy;
    ui.horizontal_wrapped(|ui| {
        ui.vertical(|ui| {
            ui.heading("短信中心");
            ui.label(meta_text("管理模块短信，阅读消息与发送记录"));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(super::theme::primary_button("新建短信")).clicked() {
                state.open = true;
            }
            if ui
                .add_enabled(
                    !snapshot.sms_refresh_pending && !snapshot.serial_work_busy,
                    egui::Button::new("刷新列表"),
                )
                .clicked()
            {
                refresh(sink, state);
            }
        });
    });
    ui.add_space(16.0);
    compose::render(ui, state, snapshot, sink);
    if let Some(deletion) = &snapshot.sms_delete {
        render_delete_result(ui, deletion);
    }
    let busy = snapshot.serial_work_busy
        || snapshot
            .sms_send
            .as_ref()
            .is_some_and(|send| send.is_active())
        || snapshot.operation.as_ref().is_some_and(|operation| {
            matches!(
                operation.state,
                dji4g_application::OperationState::Running { .. }
            )
        });
    state.auto_refresh(
        Instant::now(),
        snapshot.app.device.is_some(),
        busy,
        snapshot.sms_refresh_pending,
        sink,
    );
    render_inbox(ui, &vm, snapshot, language, sink, state);
}

fn refresh(sink: &dyn UiCommandSink, state: &mut SmsComposeState) {
    state.request_refresh(Instant::now(), sink);
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

fn empty_panel(ui: &mut Ui, title: &str, note: &str, height: f32) {
    let height = if height.is_finite() {
        height.max(0.0)
    } else {
        0.0
    };
    ui.vertical_centered(|ui| {
        ui.add_space((height * 0.20).clamp(8.0, 96.0));
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 68.0) / 2.0).max(0.0));
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(0xec, 0xef, 0xff))
                .rounding(18.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.label(
                        super::icons::text(ui.ctx(), super::icons::MAIL, 28.0)
                            .color(scale::DOWNLOAD),
                    );
                });
        });
        ui.add_space(16.0);
        ui.label(RichText::new(title).size(18.0).strong().color(scale::INK));
        ui.add_space(4.0);
        wrapped_label(ui, meta_text(note));
    });
}

fn render_inbox(
    ui: &mut Ui,
    vm: &SmsVm,
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    // Query evidence has its own quiet status strip; sending results remain independent.
    ui.horizontal_wrapped(|ui| {
        if snapshot.sms_refresh_pending {
            ui.spinner();
            ui.label(meta_text("正在同步模块短信…"));
        } else {
            ui.label(meta_text(&vm.status.text));
        }
        if vm.unread_count > 0 {
            badge(ui, format!("{} 条未读", vm.unread_count), scale::DOWNLOAD);
        }
        if let Some((used, total)) = vm.capacity {
            ui.label(meta_text(format!("存储 {used} / {total}")));
        }
    });
    ui.label(meta_text(if snapshot.app.device.is_some() {
        "每 15 秒自动同步 · 发送期间暂停"
    } else {
        "设备连接后自动同步短信"
    }));
    // A sync failure is real information, but it used to print a full paragraph above the list
    // and eat the space the messages needed. It stays one click away instead.
    if let Some(error) = &state.refresh_error {
        egui::CollapsingHeader::new(
            RichText::new("短信同步遇到问题")
                .size(13.0)
                .color(StatusTone::Caution.color()),
        )
        .id_salt("sms-refresh-error")
        .default_open(false)
        .show(ui, |ui| {
            wrapped_label(ui, RichText::new(error).color(StatusTone::Caution.color()));
        });
    }
    if vm.has_incomplete {
        wrapped_label(
            ui,
            RichText::new(&vm.incomplete_warning.text).color(StatusTone::Caution.color()),
        );
    }
    if vm.evicted > 0 {
        wrapped_label(
            ui,
            meta_text(format!(
                "本地缓存已移出 {} 条较早短信；模块存储可能仍有记录。",
                vm.evicted
            )),
        );
    }
    ui.add_space(12.0);
    egui::Frame::none()
        .fill(egui::Color32::WHITE)
        .rounding(14.0)
        .inner_margin(18.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                for (outgoing, title) in [(false, "收件箱"), (true, "发送记录")] {
                    let count = vm
                        .rows
                        .iter()
                        .filter(|r| (r.direction == SmsDirection::Outgoing) == outgoing)
                        .count();
                    let active = state.outgoing == outgoing;
                    let response = ui.add(
                        egui::Button::new(RichText::new(format!("{title}  {count}")).color(
                            if active {
                                scale::DOWNLOAD
                            } else {
                                scale::SECONDARY
                            },
                        ))
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE),
                    );
                    if active {
                        let r = response.rect;
                        ui.painter().line_segment(
                            [r.left_bottom(), r.right_bottom()],
                            egui::Stroke::new(2.0_f32, scale::DOWNLOAD),
                        );
                    }
                    if response.clicked() {
                        state.outgoing = outgoing;
                        state.selected = None;
                    }
                }
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            let query = state.search.trim().to_lowercase();
            let rows: Vec<_> = vm
                .rows
                .iter()
                .filter(|row| {
                    (row.direction == SmsDirection::Outgoing) == state.outgoing
                        && (query.is_empty()
                            || row.sender_full.to_lowercase().contains(&query)
                            || row.body.to_lowercase().contains(&query))
                })
                .collect();
            if state
                .selected
                .is_some_and(|key| !rows.iter().any(|r| r.stable_id == key))
            {
                state.selected = None;
            }
            // Everything the frame has left, minus the footer note, is the workspace. One
            // subtraction, from the container's real height — no screen-coordinate estimate and
            // no fixed cap, so a taller window really does show more messages.
            let workspace_height = (ui.available_height() - sms_layout::FOOTER_RESERVE).max(0.0);
            let layout = sms_layout::workspace_layout(
                ui.available_width(),
                workspace_height,
                sms_layout::COLUMN_GAP,
            );
            ui.data_mut(|data| {
                data.insert_temp(workspace_height_id(), layout.body_height);
            });
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), layout.body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if layout.wide {
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(layout.list_width, layout.body_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    list_panel(ui, &rows, snapshot, language, sink, state);
                                },
                            );
                            let (divider, _) = ui.allocate_exact_size(
                                egui::vec2(1.0, layout.body_height),
                                egui::Sense::hover(),
                            );
                            ui.painter().line_segment(
                                [divider.center_top(), divider.center_bottom()],
                                egui::Stroke::new(1.0_f32, scale::LINE),
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(layout.detail_width, layout.body_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::ScrollArea::vertical()
                                        .id_salt("sms-reader")
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            render_detail(ui, &rows, language, sink, state);
                                        });
                                },
                            );
                        });
                    } else if state.selected.is_some() {
                        // Narrow layout: the reader replaces the list, the back button stays
                        // pinned above its own bounded scroll area so a long message scrolls
                        // inside the reader instead of moving the page.
                        if ui.button("返回消息列表").clicked() {
                            state.selected = None;
                        }
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("sms-reader-narrow")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                render_detail(ui, &rows, language, sink, state);
                            });
                    } else {
                        list_panel(ui, &rows, snapshot, language, sink, state);
                    }
                },
            );
            ui.add_space(14.0);
            ui.separator();
            // Truncated to one line so the reserved footer height above stays exact.
            ui.add(
                egui::Label::new(meta_text(
                    "模块接受发送 ≠ 收件人已收到  ·  短信内容不会写入诊断日志",
                ))
                .truncate(),
            );
        });
}

fn list_panel(
    ui: &mut Ui,
    rows: &[&SmsRowVm],
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    // The search field is part of the list column, so the list gets the column's leftover height
    // directly instead of receiving "height - 88 - 48" from the caller.
    let column_height = ui.available_height();
    ui.add(
        egui::TextEdit::singleline(&mut state.search)
            .hint_text("搜索号码或短信内容")
            .desired_width(f32::INFINITY)
            .margin(egui::vec2(12.0, 10.0)),
    );
    ui.add_space(10.0);
    let search_consumed = (column_height - ui.available_height()).max(0.0);
    let viewport = sms_layout::list_viewport_height(column_height, search_consumed);
    ui.data_mut(|data| {
        data.insert_temp(list_viewport_id(), viewport);
    });
    if rows.is_empty() {
        let (title, note) = if !state.search.trim().is_empty() {
            ("没有找到相关短信", "试试其他号码或关键词")
        } else if state.outgoing {
            ("还没有发送记录", "点击右上角「新建短信」开始写信")
        } else if snapshot.sms_refresh_pending {
            ("正在读取短信", "正在与模块同步，请稍候")
        } else {
            match snapshot.sms_inbox.status {
                FeatureStatus::NotProbed => ("收件箱等待同步", "连接模块后会自动读取已保存的短信"),
                FeatureStatus::Supported | FeatureStatus::Empty => {
                    ("暂无收到的短信", "模块中暂时没有可显示的短信")
                }
                _ => ("暂时无法读取短信", "请查看上方的具体错误，检查连接后重试"),
            }
        };
        empty_panel(ui, title, note, viewport);
        if !state.outgoing && state.search.is_empty() && !snapshot.sms_refresh_pending {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                if ui.button("刷新短信").clicked() {
                    refresh(sink, state);
                }
            });
        }
    } else {
        egui::ScrollArea::vertical()
            .id_salt("sms-message-list")
            .auto_shrink([false, false])
            .max_height(viewport)
            .show(ui, |ui| {
                render_list(ui, rows, language, sink, state);
            });
    }
}

fn render_list(
    ui: &mut Ui,
    rows: &[&SmsRowVm],
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    for row in rows {
        let outgoing = row.direction == SmsDirection::Outgoing;
        let key = row.stable_id;
        let selected = state.selected == Some(key);
        let frame = egui::Frame::none()
            .rounding(10.0)
            .inner_margin(12.0)
            .fill(if selected {
                egui::Color32::from_rgb(0xed, 0xf0, 0xff)
            } else {
                egui::Color32::from_rgb(0xf8, 0xf9, 0xfc)
            });
        let response = frame
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.y = 18.0;
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    if row.unread == Some(true) && !outgoing {
                        ui.colored_label(scale::DOWNLOAD, "●");
                    }
                    ui.label(
                        RichText::new(&row.sender_masked)
                            .size(15.0)
                            .strong()
                            .color(scale::INK),
                    );
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(body_preview(&row.body))
                            .size(13.0)
                            .color(scale::SECONDARY),
                    )
                    .truncate(),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label(meta_text(row.timestamp.as_deref().unwrap_or("时间未提供")));
                    if outgoing {
                        let (tone, label) = outgoing_state(row.status);
                        ui.colored_label(
                            tone.color(),
                            format!("{} {}", tone.marker(), label.to_string(language)),
                        );
                    }
                });
            })
            .response
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() && !selected {
            ui.painter()
                .rect_stroke(response.rect, 10.0, egui::Stroke::new(1.0_f32, scale::LINE));
        }
        if response.clicked() {
            state.selected = Some(key);
            if !outgoing && !state.serial_busy {
                state.error = sink
                    .try_send(UiCommand::SmsRead { index: row.index })
                    .err()
                    .map(|e| compose::enqueue_error(e).to_owned());
            }
        }
        ui.add_space(6.0);
    }
}

fn render_detail(
    ui: &mut Ui,
    rows: &[&SmsRowVm],
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    let Some(row) = rows
        .iter()
        .find(|row| state.selected == Some(row.stable_id))
    else {
        empty_panel(
            ui,
            "消息阅读区",
            "从左侧选择一条短信，即可在这里查看完整内容",
            ui.available_height(),
        );
        return;
    };
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        badge(
            ui,
            if row.direction == SmsDirection::Incoming {
                "收到的短信"
            } else {
                "发送记录"
            },
            scale::DOWNLOAD,
        );
        for tag in row_tags(row, language) {
            badge(ui, tag.text, tag.tone.color());
        }
    });
    ui.add_space(6.0);
    wrapped_label(ui, RichText::new(&row.sender_full).size(18.0).strong());
    ui.label(meta_text(
        row.timestamp.as_deref().unwrap_or("模块未提供时间"),
    ));
    ui.add_space(8.0);
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(0xf5, 0xf7, 0xfb))
        .rounding(12.0)
        .inner_margin(18.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            wrapped_label(ui, RichText::new(&row.body).size(14.0).color(scale::INK));
        });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(TextKey::ButtonCopy.to_string(language)).clicked() {
            ui.ctx()
                .copy_text(format!("{}\n{}", row.sender_full, row.body));
        }
        if row.direction == SmsDirection::Incoming {
            if ui
                .add_enabled(!state.serial_busy, egui::Button::new("回复"))
                .clicked()
            {
                state.begin_reply(&row.sender_full);
            }
            render_delete_button(ui, row, language, sink, state);
        } else {
            badge(
                ui,
                outgoing_state(row.status).1.to_string(language),
                outgoing_state(row.status).0.color(),
            );
        }
    });
}
/// Per-row delete with an in-page two-click confirmation: the first click arms the button for
/// [`CONFIRM_ARM_WINDOW`] and the second dispatches. No deletion is ever sent unconfirmed, and
/// only incoming rows reach this path (an outgoing record has no module copy to delete).
fn render_delete_button(
    ui: &mut Ui,
    row: &SmsRowVm,
    _language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    let armed_id = egui::Id::new(("sms-row-delete-armed", row.stable_id));
    let enabled = row.delete_allowed && !row.fragments.is_empty() && !state.serial_busy;
    if !enabled {
        ui.data_mut(|data| data.remove::<Instant>(armed_id));
    }
    let armed = enabled
        && ui
            .data(|data| data.get_temp::<Instant>(armed_id))
            .is_some_and(|at| at.elapsed() < CONFIRM_ARM_WINDOW);
    let label = if armed {
        format!("确认删除 {} 个已读取分片", row.fragments.len())
    } else if row.status == SmsStatus::Incomplete {
        format!("删除已读取 {} 个分片", row.fragments.len())
    } else {
        format!("删除短信（{} 个分片）", row.fragments.len())
    };
    if ui
        .add_enabled(
            enabled,
            egui::Button::new(RichText::new(label).color(if armed {
                StatusTone::Negative.color()
            } else {
                scale::SECONDARY
            })),
        )
        .clicked()
    {
        if armed {
            ui.data_mut(|data| data.remove::<Instant>(armed_id));
            state.error = sink
                .try_send(UiCommand::SmsDelete {
                    fragments: row.fragments.clone(),
                })
                .err()
                .map(|error| compose::enqueue_error(error).to_owned());
        } else {
            ui.data_mut(|data| data.insert_temp(armed_id, Instant::now()));
        }
    }
    if !row.delete_allowed {
        wrapped_label(
            ui,
            meta_text("分片身份存在冲突，暂不能删除；请重新读取并核对。"),
        );
    }
}

fn delete_result_text(deletion: &dji4g_application::SmsDeleteSnapshot) -> (StatusTone, String) {
    let deleted = deletion
        .items
        .iter()
        .filter(|item| item.result == SmsDeleteItemResult::Deleted)
        .count();
    let unknown = deletion
        .items
        .iter()
        .filter(|item| item.result == SmsDeleteItemResult::OutcomeUnknown)
        .count();
    if !deletion.finished {
        return (
            StatusTone::Progress,
            format!("正在删除：已确认 {deleted} / {} 个分片", deletion.total),
        );
    }
    if deleted == deletion.total && deletion.total > 0 {
        return (
            StatusTone::Positive,
            format!("已确认删除全部 {} 个已读取分片", deletion.total),
        );
    }
    if unknown > 0 {
        return (
            StatusTone::Caution,
            format!(
                "删除结果未知：已确认 {deleted} / {} 个，{unknown} 个未能确认。请刷新核对，不会自动重试。",
                deletion.total
            ),
        );
    }
    if deleted > 0 {
        return (
            StatusTone::Caution,
            format!(
                "部分删除：已确认 {deleted} / {} 个分片，其余失败或未执行。",
                deletion.total
            ),
        );
    }
    (
        StatusTone::Negative,
        "未确认删除任何分片；请查看原因并重新读取。".into(),
    )
}

fn render_delete_result(ui: &mut Ui, deletion: &dji4g_application::SmsDeleteSnapshot) {
    let (tone, text) = delete_result_text(deletion);
    wrapped_label(
        ui,
        RichText::new(format!("{} {text}", tone.marker())).color(tone.color()),
    );
    if deletion.items.len() > 1 || deletion.items.iter().any(|item| item.code.is_some()) {
        egui::CollapsingHeader::new("删除分片结果").show(ui, |ui| {
            for item in &deletion.items {
                let result = match item.result {
                    SmsDeleteItemResult::Deleted => "已确认删除",
                    SmsDeleteItemResult::Failed => "失败",
                    SmsDeleteItemResult::OutcomeUnknown => "结果未知",
                    SmsDeleteItemResult::NotAttempted => "未执行",
                };
                let code = item.code.as_deref().unwrap_or("");
                wrapped_label(
                    ui,
                    meta_text(format!(
                        "{} #{} · {result} {code}",
                        item.fragment.storage.0, item.fragment.index
                    )),
                );
            }
        });
    }
}

/// Single-line body preview for a collapsed row: at most [`BODY_PREVIEW_CHARS`] characters, with
/// an explicit ellipsis when the body continues. The full text stays behind the row click.
const BODY_PREVIEW_CHARS: usize = 42;

fn body_preview(body: &str) -> String {
    let flattened = body.replace(['\r', '\n'], " ");
    let mut chars = flattened.chars();
    let preview: String = chars.by_ref().take(BODY_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
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
    use super::*;
    use std::sync::Arc;

    use dji4g_application::{CommandStateSnapshot, ControllerSnapshot, SettingsSnapshot};
    use dji4g_domain::{
        AppSnapshot, Availability, DeviceEpoch, FeatureStatus, Freshness, HotspotStatus,
        SmsInboxSummary, SmsMessage, SmsMultipartInfo, SmsStatus, SmsStorageId,
    };

    fn display_messages(messages: &[SmsMessage]) -> Vec<SmsDisplayMessage> {
        messages
            .iter()
            .map(|message| SmsDisplayMessage {
                message: message.clone(),
                fragments: vec![message.fragment_key()],
                delete_allowed: message.direction == SmsDirection::Incoming,
            })
            .collect()
    }

    fn snapshot(summary: SmsInboxSummary) -> ControllerSnapshot {
        ControllerSnapshot {
            publication_revision: 7,
            app: Arc::new(AppSnapshot {
                revision: 7,
                observed_at: std::time::SystemTime::UNIX_EPOCH,
                freshness: Freshness::Fresh,
                availability: Availability::Available,
                hotspot: HotspotStatus::Off,
                device: None,
                cellular: None,
                network: None,
                active_operation: None,
                issues: Vec::new(),
            }),
            diagnostics: dji4g_application::DiagnosticSet::new(DeviceEpoch(1)),
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
            sms_inbox: summary,
            sms_messages: Vec::new(),
            sms_send: None,
            sms_delete: None,
            serial_work_busy: false,
            sms_refresh_pending: false,
            sms_inbox_failure: None,
            device_tools: Default::default(),
        }
    }

    fn message(index: u32, read: Option<bool>, status: SmsStatus) -> SmsMessage {
        let mut message = SmsMessage::new(
            index,
            SmsStorageId("SM".to_owned()),
            1,
            0,
            "+8613800138000",
            "sensitive body",
            SmsEncoding::Ucs2,
            status,
        );
        message.service_centre_timestamp = Some("24/09/10,12:00:00+32".to_owned());
        message.multipart = Some(SmsMultipartInfo {
            reference: dji4g_domain::SmsConcatReference::EightBit(1),
            total: 3,
            sequence: 2,
        });
        message.read = read;
        message
    }

    fn outgoing_message(status: SmsStatus) -> SmsMessage {
        SmsMessage::new_outgoing(
            4,
            1,
            0,
            "+8613800138000",
            "sent body",
            SmsEncoding::Other,
            status,
        )
    }

    #[test]
    fn physical_storage_and_payload_changes_cannot_reuse_a_selection() {
        let original = message(7, Some(false), SmsStatus::Received);
        let mut other_storage = original.clone();
        other_storage.storage = SmsStorageId("ME".into());
        let replacement = SmsMessage::new(
            7,
            original.storage.clone(),
            1,
            0,
            "+8613800138000",
            "replacement",
            SmsEncoding::Ucs2,
            SmsStatus::Received,
        );
        assert_ne!(
            sms_row_vm(&original, Language::ZhCn).stable_id,
            sms_row_vm(&other_storage, Language::ZhCn).stable_id
        );
        assert_ne!(
            sms_row_vm(&original, Language::ZhCn).stable_id,
            sms_row_vm(&replacement, Language::ZhCn).stable_id
        );
        let mut read = original.clone();
        read.read = Some(true);
        assert_eq!(
            sms_row_vm(&original, Language::ZhCn).stable_id,
            sms_row_vm(&read, Language::ZhCn).stable_id
        );
    }

    #[test]
    fn delete_receipts_distinguish_all_partial_and_unknown() {
        use dji4g_application::{SmsDeleteItemSnapshot, SmsDeleteSnapshot};
        let fragment = message(7, None, SmsStatus::Received).fragment_key();
        let mut deletion = SmsDeleteSnapshot {
            request_id: 1,
            total: 2,
            finished: true,
            items: vec![
                SmsDeleteItemSnapshot {
                    fragment: fragment.clone(),
                    result: SmsDeleteItemResult::Deleted,
                    code: None,
                },
                SmsDeleteItemSnapshot {
                    fragment,
                    result: SmsDeleteItemResult::Failed,
                    code: Some("sms:delete_rejected".into()),
                },
            ],
        };
        assert!(delete_result_text(&deletion).1.contains("部分删除"));
        deletion.items[1].result = SmsDeleteItemResult::OutcomeUnknown;
        assert_eq!(delete_result_text(&deletion).0, StatusTone::Caution);
        assert!(delete_result_text(&deletion).1.contains("不会自动重试"));
        deletion.items[1].result = SmsDeleteItemResult::Deleted;
        assert_eq!(delete_result_text(&deletion).0, StatusTone::Positive);
        assert!(delete_result_text(&deletion).1.contains("全部 2"));
    }

    #[test]
    fn sms_vm_reports_status_counts_capacity_and_incomplete_flag() {
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                message_count: 2,
                unread_count: 1,
                capacity: Some((2, 30)),
                status: FeatureStatus::Supported,
                has_incomplete: true,
                evicted: 0,
            }),
            &[],
            Language::ZhCn,
        );
        assert_eq!(vm.status.text, "已读取");
        assert_eq!(vm.message_count, 2);
        assert_eq!(vm.unread_count, 1);
        assert_eq!(
            vm.capacity_text.as_ref().map(|text| text.text.as_str()),
            Some("已用 2 / 总数 30")
        );
        assert!(vm.has_incomplete);
        assert_eq!(vm.incomplete_warning.text, "存在未完整接收的长短信");
        assert!(vm.rows.is_empty());
        assert!(!vm.empty_text.text.trim().is_empty());
        assert!(!vm.list_pending_text.text.trim().is_empty());
    }

    #[test]
    fn sms_vm_wires_the_local_eviction_count_from_the_summary() {
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                evicted: 2,
                ..SmsInboxSummary::default()
            }),
            &[],
            Language::ZhCn,
        );
        assert_eq!(vm.evicted, 2);
    }

    #[test]
    fn sms_vm_marks_an_unqueried_inbox_and_omits_absent_capacity() {
        let vm = sms_vm(&snapshot(SmsInboxSummary::default()), &[], Language::ZhCn);
        assert_eq!(vm.status.text, "尚未查询");
        assert_eq!(vm.capacity_text, None);
        assert!(!vm.has_incomplete);
    }

    #[test]
    fn sms_vm_maps_a_classified_failure_to_its_precise_note() {
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                status: FeatureStatus::TransportFailure,
                ..SmsInboxSummary::default()
            }),
            &[],
            Language::ZhCn,
        );
        assert!(
            vm.status.text.contains("本次超时"),
            "got: {}",
            vm.status.text
        );
        assert!(!vm.status.text.contains("尚未查询"));
    }

    #[test]
    fn sms_row_vm_masks_the_sender_and_keeps_read_state_and_parts() {
        let row = sms_row_vm(
            &message(7, Some(false), SmsStatus::Incomplete),
            Language::ZhCn,
        );
        assert_eq!(row.index, 7);
        assert_eq!(row.direction, SmsDirection::Incoming);
        assert_eq!(row.unread, Some(true));
        assert_eq!(row.sender_masked, "****8000");
        assert_eq!(row.sender_full, "+8613800138000");
        assert_eq!(row.body, "sensitive body");
        assert_eq!(row.timestamp.as_deref(), Some("24/09/10,12:00:00+32"));
        assert_eq!(row.encoding, SmsEncoding::Ucs2);
        assert_eq!(row.multipart, Some((2, 3)));
        assert_eq!(row.status, SmsStatus::Incomplete);
    }

    #[test]
    fn sms_row_vm_projects_outgoing_records_as_local_bookkeeping() {
        let row = sms_row_vm(&outgoing_message(SmsStatus::Submitted), Language::ZhCn);
        assert_eq!(row.direction, SmsDirection::Outgoing);
        assert_eq!(row.status, SmsStatus::Submitted);
        // Outgoing mail is never unread mail.
        assert_eq!(row.unread, Some(false));
        assert_eq!(row.timestamp, None);
        assert_eq!(row.body, "sent body");
        assert_eq!(row.sender_masked, "****8000");
    }

    #[test]
    fn sms_row_vm_keeps_an_unknown_read_state_unknown() {
        let row = sms_row_vm(&message(1, None, SmsStatus::Received), Language::ZhCn);
        assert_eq!(row.unread, None);
    }

    #[test]
    fn sms_vm_projects_rows_in_store_order() {
        let messages = vec![
            message(3, Some(true), SmsStatus::Received),
            outgoing_message(SmsStatus::OutcomeUnknown),
            message(9, Some(false), SmsStatus::Received),
        ];
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                message_count: 3,
                status: FeatureStatus::Supported,
                ..SmsInboxSummary::default()
            }),
            &messages,
            Language::ZhCn,
        );
        assert_eq!(
            vm.rows.iter().map(|row| row.index).collect::<Vec<_>>(),
            vec![3, 4, 9]
        );
        assert_eq!(vm.rows[0].unread, Some(false));
        assert_eq!(vm.rows[1].direction, SmsDirection::Outgoing);
        assert_eq!(vm.rows[2].unread, Some(true));
    }

    #[test]
    fn encoding_vocabulary_is_closed() {
        assert_eq!(sms_encoding_text(SmsEncoding::Gsm7, Language::ZhCn), "GSM7");
        assert_eq!(sms_encoding_text(SmsEncoding::Ucs2, Language::ZhCn), "UCS2");
        assert_eq!(
            sms_encoding_text(SmsEncoding::Other, Language::ZhCn),
            "其他"
        );
    }

    #[test]
    fn the_body_preview_is_single_line_bounded_and_marks_truncation() {
        assert_eq!(body_preview("短正文"), "短正文");
        let exact = "字".repeat(BODY_PREVIEW_CHARS);
        assert_eq!(
            body_preview(&exact),
            exact,
            "an exact-fit body is not elided"
        );
        let long = format!("{exact}尾");
        let preview = body_preview(&long);
        assert_eq!(preview.chars().count(), BODY_PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
        assert!(
            !body_preview("第一行\n第二行").contains('\n'),
            "the preview must stay on one line"
        );
    }

    #[test]
    fn the_read_marker_is_a_filled_progress_dot_only_when_unread() {
        assert_eq!(
            row_read_state(Some(true)),
            (StatusTone::Progress, "●", TextKey::SmsUnread)
        );
        assert_eq!(
            row_read_state(Some(false)),
            (StatusTone::Neutral, "○", TextKey::SmsRead)
        );
        assert_eq!(
            row_read_state(None),
            (StatusTone::Neutral, "○", TextKey::ValueUnknown)
        );
    }

    #[test]
    fn outgoing_submission_status_maps_to_its_tone_and_label() {
        assert_eq!(
            outgoing_state(SmsStatus::Submitted),
            (StatusTone::Positive, TextKey::SmsOutgoingSubmitted)
        );
        assert_eq!(
            outgoing_state(SmsStatus::Failed),
            (StatusTone::Negative, TextKey::SmsOutgoingFailed)
        );
        assert_eq!(
            outgoing_state(SmsStatus::OutcomeUnknown),
            (StatusTone::Caution, TextKey::SmsOutgoingUnknown)
        );
    }

    #[test]
    fn row_tags_keep_encoding_and_fragments_neutral_and_flag_the_incomplete_message() {
        let row = sms_row_vm(
            &message(7, Some(true), SmsStatus::Incomplete),
            Language::ZhCn,
        );
        let tags = row_tags(&row, Language::ZhCn);
        let texts = tags.iter().map(|tag| tag.text.as_str()).collect::<Vec<_>>();
        assert_eq!(texts, ["UCS2", "已读取 1 / 3 个分片", "未完整"]);
        assert_eq!(tags[0].tone, StatusTone::Neutral);
        assert_eq!(tags[1].tone, StatusTone::Neutral);
        assert_eq!(tags[2].tone, StatusTone::Caution);
    }

    #[test]
    fn row_tags_drop_the_incomplete_warning_for_a_complete_message() {
        let mut complete = message(8, Some(false), SmsStatus::Received);
        complete.multipart = None;
        let row = sms_row_vm(&complete, Language::ZhCn);
        let tags = row_tags(&row, Language::ZhCn);
        let texts = tags.iter().map(|tag| tag.text.as_str()).collect::<Vec<_>>();
        assert_eq!(texts, ["UCS2"]);
        assert!(tags.iter().all(|tag| tag.tone == StatusTone::Neutral));
    }

    #[test]
    fn row_tags_are_empty_for_outgoing_rows() {
        let row = sms_row_vm(&outgoing_message(SmsStatus::Submitted), Language::ZhCn);
        assert_eq!(row_tags(&row, Language::ZhCn), Vec::new());
    }

    #[test]
    fn the_page_renders_its_empty_state_without_panicking() {
        struct NoopSink;
        impl UiCommandSink for NoopSink {
            fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
                Ok(())
            }
        }

        let context = egui::Context::default();
        let snapshot = snapshot(SmsInboxSummary {
            message_count: 2,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 0,
        });
        let sink = NoopSink;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 600.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    render(
                        ui,
                        &snapshot,
                        &[],
                        Language::ZhCn,
                        &sink,
                        &mut SmsComposeState::default(),
                    );
                });
            },
        );
    }

    #[test]
    fn the_page_renders_message_rows_without_panicking() {
        struct NoopSink;
        impl UiCommandSink for NoopSink {
            fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
                Ok(())
            }
        }

        let context = egui::Context::default();
        let snapshot = snapshot(SmsInboxSummary {
            message_count: 3,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 2,
        });
        let messages = vec![
            message(1, Some(true), SmsStatus::Incomplete),
            outgoing_message(SmsStatus::Failed),
            message(2, Some(false), SmsStatus::Received),
        ];
        let sink = NoopSink;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 900.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    render(
                        ui,
                        &snapshot,
                        &display_messages(&messages),
                        Language::ZhCn,
                        &sink,
                        &mut SmsComposeState::default(),
                    );
                });
            },
        );
    }

    struct NoopSink;
    impl UiCommandSink for NoopSink {
        fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
            Ok(())
        }
    }

    /// Render the page into a window of `width` x `height` logical points and report the heights
    /// the workspace and the message list really received. Two passes are run so the values from
    /// the first frame (used by egui to size scroll areas) are settled by the second.
    fn measured_heights(width: f32, height: f32) -> (f32, f32) {
        let snapshot = snapshot(SmsInboxSummary {
            message_count: 3,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: FeatureStatus::Supported,
            has_incomplete: false,
            evicted: 0,
        });
        let messages = vec![
            message(1, Some(true), SmsStatus::Received),
            message(2, Some(false), SmsStatus::Received),
            message(3, Some(false), SmsStatus::Received),
        ];
        let sink = NoopSink;
        let mut state = SmsComposeState::default();
        let context = egui::Context::default();
        for _ in 0..2 {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        render(
                            ui,
                            &snapshot,
                            &display_messages(&messages),
                            Language::ZhCn,
                            &sink,
                            &mut state,
                        );
                    });
                },
            );
        }
        context.data(|data| {
            (
                data.get_temp::<f32>(workspace_height_id()).unwrap_or(0.0),
                data.get_temp::<f32>(list_viewport_id()).unwrap_or(0.0),
            )
        })
    }

    /// The user-visible regression: the list viewport used to be capped, so making the window
    /// taller did not show more messages. It must now track the window one-for-one.
    #[test]
    fn the_list_viewport_grows_with_the_window_instead_of_hitting_a_cap() {
        let (short_workspace, short_list) = measured_heights(1100.0, 760.0);
        let (tall_workspace, tall_list) = measured_heights(1100.0, 1000.0);
        let workspace_gain = tall_workspace - short_workspace;
        let list_gain = tall_list - short_list;
        eprintln!(
            "1100x760 -> workspace {short_workspace}, list {short_list}; \
             1100x1000 -> workspace {tall_workspace}, list {tall_list}"
        );
        assert!(
            (workspace_gain - 240.0).abs() <= 16.0,
            "workspace gain {workspace_gain} (short {short_workspace}, tall {tall_workspace})"
        );
        assert!(
            (list_gain - 240.0).abs() <= 16.0,
            "list gain {list_gain} (short {short_list}, tall {tall_list})"
        );
        // The old implementation froze the body at 340 - 88 - 48 = 204 points.
        assert!(
            short_list > 204.0 + 16.0,
            "760-point window still looks capped: {short_list}"
        );
    }

    #[test]
    fn the_workspace_stays_inside_the_page_and_never_panics_at_odd_sizes() {
        for (width, height) in [
            (320.0_f32, 600.0_f32),
            (719.0, 600.0),
            (720.0, 600.0),
            (800.0, 600.0),
            (1440.0, 1000.0),
            (1100.0, 300.0),
        ] {
            let (workspace, list) = measured_heights(width, height);
            assert!(
                workspace >= 0.0 && workspace.is_finite(),
                "{width}x{height} workspace {workspace}"
            );
            assert!(
                list >= 0.0 && list.is_finite(),
                "{width}x{height} list {list}"
            );
            assert!(
                list <= workspace + 0.5,
                "{width}x{height} list {list} exceeds workspace {workspace}"
            );
        }
    }
}
