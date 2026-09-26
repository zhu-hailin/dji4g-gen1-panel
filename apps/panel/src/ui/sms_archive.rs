//! A read-only personal history surface. It never constructs live module commands.
use crate::sms_archive::ArchiveService;
use eframe::egui::{self, Ui};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub(crate) struct ArchiveUi {
    search: String,
    days: u64,
    confirm: Option<Confirmation>,
    pub error: Option<String>,
    pub export_path: Option<std::path::PathBuf>,
}

#[derive(Clone, Copy)]
enum Confirmation {
    Clear,
    Export,
}
pub(crate) enum ArchiveAction {
    SetEnabled(bool),
    Clear,
    Export,
}

pub(crate) fn render(
    ui: &mut Ui,
    archive: Option<&ArchiveService>,
    state: &mut ArchiveUi,
) -> Option<ArchiveAction> {
    ui.heading("本地历史");
    ui.label("开启后，已读到的收件短信会保存在这台电脑。断开模块或重启软件后仍可查看；这里不能删除模块中的短信。");
    ui.small("仅当前 Windows 用户可解密；最多保存 5000 条，超过保存日期 90 天自动清除。关闭保存不会删除已有历史。");
    let Some(archive) = archive else {
        ui.label("本地历史暂不可用：无法确定用户目录，或当前处于模拟演示。");
        return None;
    };
    let mut action = None;
    let mut enabled = archive.enabled();
    if ui
        .checkbox(&mut enabled, "在这台电脑保存短信历史（可随时关闭）")
        .changed()
    {
        action = Some(ArchiveAction::SetEnabled(enabled));
    }
    ui.label(archive.status());
    if let Some(error) = &state.error {
        ui.colored_label(super::StatusTone::Negative.color(), error);
    }
    if let Some(path) = &state.export_path {
        ui.label(format!("导出目标：{}", path.display()));
    }
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("搜索号码或正文")
                .desired_width(200.0),
        );
        super::components::segmented_control(
            ui,
            egui::Id::new("archive-date-filter"),
            &mut state.days,
            &[
                super::components::TabItem::new(0, "全部历史"),
                super::components::TabItem::new(7, "近 7 天保存"),
                super::components::TabItem::new(30, "近 30 天保存"),
            ],
        );
        if ui
            .add_enabled(
                !archive.busy() && !archive.rows().is_empty(),
                egui::Button::new("导出 TXT"),
            )
            .clicked()
        {
            state.confirm = Some(Confirmation::Export);
        }
        if ui
            .add_enabled(!archive.busy(), egui::Button::new("清空本地历史"))
            .clicked()
        {
            state.confirm = Some(Confirmation::Clear);
        }
    });
    ui.separator();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let query = state.search.trim().to_lowercase();
    let rows = archive
        .rows()
        .iter()
        .rev()
        .filter(|row| {
            (state.days == 0 || now.saturating_sub(row.captured_unix_secs()) <= state.days * 86400)
                && (query.is_empty()
                    || row.sender().to_lowercase().contains(&query)
                    || row.body().to_lowercase().contains(&query))
        })
        .collect::<Vec<_>>();
    ui.label(format!(
        "显示 {} / {} 条 · 按保存顺序排列",
        rows.len(),
        archive.rows().len()
    ));
    if rows.is_empty() {
        ui.label(if archive.rows().is_empty() { "暂无本地历史。开启保存后，在“模块短信”中读取短信即可；无法找回模块中已被删除且未保存的消息。" } else { "没有符合筛选条件的记录。" });
    }
    egui::ScrollArea::vertical()
        .id_salt("archive-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, row) in rows.iter().enumerate() {
                egui::CollapsingHeader::new(format!(
                    "{}    {}{}",
                    row.sender(),
                    row.reported_timestamp().unwrap_or("发送时间未提供"),
                    if row.incomplete() {
                        " · 分片未齐"
                    } else {
                        ""
                    }
                ))
                .id_salt((
                    "archive-row",
                    index,
                    row.captured_unix_secs(),
                    row.context_label(),
                ))
                .show(ui, |ui| {
                    ui.label(row.body());
                    ui.small(format!("来源分组：{} · 历史副本", row.context_label()));
                    if ui.button("复制正文").clicked() {
                        ui.ctx().copy_text(row.body().to_owned());
                    }
                });
            }
        });
    if let Some(confirm) = state.confirm {
        egui::Window::new(match confirm { Confirmation::Clear => "清空本地历史", Confirmation::Export => "导出短信明文" })
            .collapsible(false).resizable(false).default_width(370.0).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label(match confirm {
                    Confirmation::Clear => "将删除这台电脑保存的全部短信历史，并关闭后续保存。模块中的短信不受影响。此操作无法撤销。",
                    Confirmation::Export => "将全部已保存历史（包含号码和正文）导出为未加密 TXT。请妥善保管，分享前检查隐私。",
                });
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() { state.confirm = None; }
                    if ui.add_enabled(!archive.busy(), egui::Button::new(match confirm { Confirmation::Clear => "确认清空", Confirmation::Export => "确认导出全部" })).clicked() {
                        state.confirm = None;
                        action = Some(match confirm { Confirmation::Clear => ArchiveAction::Clear, Confirmation::Export => ArchiveAction::Export });
                    }
                });
            });
    }
    action
}
