//! Read-only radio observations. Only completed AT samples enter the bounded history.
use super::{meta_text, scale, section_frame, section_heading, wrapped_label};
use dji4g_application::{ControllerSnapshot, DiagnosticCheckId, DiagnosticCheckState};
use dji4g_domain::{DeviceEpoch, ServingCell};
use eframe::egui::{self, Color32, RichText, Ui};
use std::{collections::VecDeque, time::SystemTime};

#[derive(Clone)]
struct Sample {
    at: SystemTime,
    cell: Option<ServingCell>,
}
struct CellChange {
    at: SystemTime,
    before: String,
    after: String,
}
#[derive(Default)]
pub(crate) struct WirelessHistory {
    context: Option<(DeviceEpoch, u64)>,
    samples: VecDeque<Sample>,
    changes: VecDeque<CellChange>,
    previous_cell: Option<String>,
}
impl WirelessHistory {
    #[cfg(debug_assertions)]
    pub(crate) fn review_fixture(&mut self) {
        let now = SystemTime::now();
        for i in 0..60u64 {
            let cell: ServingCell = serde_json::from_value(serde_json::json!({
                "state":"NOCONN","duplex":"FDD","rat":"LTE","mcc":"460","mnc":"01",
                "cell_id": if i < 40 { 123456 } else { 123457 }, "pci": if i < 40 { 123 } else { 124 },
                "earfcn":1650,"band":3,"ul_mhz":20.0,"dl_mhz":20.0,"tac":2748,
                "rsrp_dbm": -99 + (i % 12) as i16, "rsrq_db":-10,"rssi_dbm":-65,"sinr_db":15
            })).expect("static visual fixture");
            self.push(
                (DeviceEpoch(1), 0),
                now - std::time::Duration::from_secs((59 - i) * 3),
                Some(cell),
            );
        }
    }
    pub(crate) fn observe(&mut self, snapshot: &ControllerSnapshot) {
        let Some(check) = snapshot
            .diagnostics
            .iter()
            .find(|c| c.id == DiagnosticCheckId::AtControl)
        else {
            return;
        };
        let context = (check.epoch, snapshot.sim_epoch);
        if self.context != Some(context) {
            self.reset(context);
        }
        if matches!(
            check.state,
            DiagnosticCheckState::Running { .. } | DiagnosticCheckState::Unexecuted { .. }
        ) {
            return;
        }
        let Some(at) = check.finished_at else {
            return;
        };
        let cell = if check.state == DiagnosticCheckState::Passed {
            snapshot
                .app
                .cellular
                .as_ref()
                .and_then(|c| c.serving_cell.clone())
        } else {
            None
        };
        self.push(context, at, cell);
    }
    fn reset(&mut self, context: (DeviceEpoch, u64)) {
        self.context = Some(context);
        self.samples.clear();
        self.changes.clear();
        self.previous_cell = None;
    }
    fn push(&mut self, context: (DeviceEpoch, u64), at: SystemTime, cell: Option<ServingCell>) {
        if self.context != Some(context) {
            self.reset(context);
        }
        if self.samples.back().is_some_and(|s| s.at >= at) {
            return;
        }
        if let Some(cell) = cell
            .as_ref()
            .filter(|c| c.cell_id.is_some() || c.pci.is_some())
        {
            let identity = cell_identity(cell);
            if let Some(previous) = self.previous_cell.as_ref().filter(|old| **old != identity) {
                self.changes.push_back(CellChange {
                    at,
                    before: previous.clone(),
                    after: identity.clone(),
                });
                if self.changes.len() > 20 {
                    self.changes.pop_front();
                }
            }
            self.previous_cell = Some(identity);
        }
        self.samples.push_back(Sample { at, cell });
        if self.samples.len() > 120 {
            self.samples.pop_front();
        }
    }
}
fn cell_identity(cell: &ServingCell) -> String {
    format!(
        "{}-{} · Cell {} · PCI {} · EARFCN {} · B{}",
        cell.mcc.as_deref().unwrap_or("?"),
        cell.mnc.as_deref().unwrap_or("?"),
        cell.cell_id
            .map(|v| format!("{v:X}"))
            .unwrap_or_else(|| "?".into()),
        value(cell.pci),
        value(cell.earfcn),
        value(cell.band)
    )
}
fn value<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
}
fn measurement(value: Option<i16>, unit: &str) -> String {
    value
        .map(|v| format!("{v} {unit}"))
        .unwrap_or_else(|| "未报告".into())
}
fn summary(cell: &ServingCell) -> String {
    format!(
        "无线观测（模块 AT+QENG 报告）\n{}\n制式 {} / {}\nRSRP {}\nRSRQ {}\nRSSI {}\nSINR 原始值 {}（单位未确认）\n上行带宽 {} MHz / 下行带宽 {} MHz\nTAC {}",
        cell_identity(cell),
        cell.rat.as_deref().unwrap_or("—"),
        cell.duplex.as_deref().unwrap_or("—"),
        measurement(cell.rsrp_dbm, "dBm"),
        measurement(cell.rsrq_db, "dB"),
        measurement(cell.rssi_dbm, "dBm"),
        value(cell.sinr_raw),
        value(cell.ul_mhz),
        value(cell.dl_mhz),
        cell.tac
            .map(|v| format!("{v:04X}"))
            .unwrap_or_else(|| "—".into())
    )
}
fn metric(ui: &mut Ui, name: &str, value: String, note: &str) {
    egui::Frame::none()
        .fill(Color32::from_rgb(0xf5, 0xf7, 0xfc))
        .rounding(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(meta_text(name));
            ui.label(RichText::new(value).size(21.0).strong().color(scale::INK));
            ui.label(meta_text(note));
        });
}
pub(crate) fn render(ui: &mut Ui, snapshot: &ControllerSnapshot, history: &WirelessHistory) {
    ui.heading("无线观测");
    ui.label(meta_text(
        "查看真实无线参数，观察摆放位置、遮挡与小区变化带来的差异",
    ));
    let cell = history.samples.back().and_then(|s| s.cell.as_ref());
    let check = snapshot
        .diagnostics
        .iter()
        .find(|c| c.id == DiagnosticCheckId::AtControl);
    let fresh = check.is_some_and(|c| {
        c.freshness(SystemTime::now()) == dji4g_domain::Freshness::Fresh
            && c.state == DiagnosticCheckState::Passed
    });
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("当前服务小区"));
            ui.label(meta_text(if fresh {
                "最近一次 AT 查询已完成"
            } else {
                "等待有效采样 / 已有数据仅供回看"
            }));
            if let Some(cell) = cell {
                if ui.button("复制无线摘要").clicked() {
                    ui.ctx().copy_text(summary(cell));
                }
            }
        });
        let Some(cell) = cell else {
            ui.add_space(8.0);
            wrapped_label(
                ui,
                "暂未获取服务小区数据。连接模块后随后台监测自动采样；短信发送期间暂停。未报告的字段保持为空。",
            );
            return;
        };
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            for item in [
                format!("频段 B{}", value(cell.band)),
                format!(
                    "{} / {}",
                    cell.rat.as_deref().unwrap_or("—"),
                    cell.duplex.as_deref().unwrap_or("—")
                ),
                format!("PCI {}", value(cell.pci)),
                format!("EARFCN {}", value(cell.earfcn)),
            ] {
                ui.label(RichText::new(item).color(scale::DOWNLOAD).strong());
                ui.add_space(10.0);
            }
        });
        ui.add_space(10.0);
        let fields = [
            ("RSRP", measurement(cell.rsrp_dbm, "dBm"), "参考信号功率"),
            ("RSRQ", measurement(cell.rsrq_db, "dB"), "参考信号质量"),
            ("RSSI", measurement(cell.rssi_dbm, "dBm"), "接收总功率"),
            ("SINR · 原始值", value(cell.sinr_raw), "单位尚未确认"),
        ];
        let columns = if ui.available_width() >= 680.0 { 4 } else { 2 };
        for chunk in fields.chunks(columns) {
            ui.columns(columns, |cols| {
                for (i, (label, v, note)) in chunk.iter().enumerate() {
                    metric(&mut cols[i], label, v.clone(), note);
                }
            });
            ui.add_space(8.0);
        }
        egui::CollapsingHeader::new("小区与带宽详情").show(ui, |ui| {
            wrapped_label(ui, cell_identity(cell));
            ui.label(format!(
                "上行带宽 {} MHz  ·  下行带宽 {} MHz",
                value(cell.ul_mhz),
                value(cell.dl_mhz)
            ));
            ui.label(format!(
                "TAC {}  ·  模块状态 {}",
                cell.tac
                    .map(|v| format!("{v:04X}"))
                    .unwrap_or_else(|| "—".into()),
                cell.state.as_deref().unwrap_or("未报告")
            ));
            ui.label(meta_text(
                "NOCONN 表示注册后空闲；不单独据此判定网络断开。信号数值不能代替实际吞吐测试。",
            ));
        });
    });
    section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(section_heading("信号变化 · RSRP"));
            ui.label(meta_text(format!(
                "最近 {} / 120 次采样",
                history.samples.len()
            )));
        });
        signal_chart(ui, history);
        ui.label(meta_text("按 AT 采样顺序显示；缺失值断开曲线，未收到新回执时不重复造点。设备或 SIM 更换后重新记录。"));
    });
    section_frame(ui, |ui| {
        ui.label(section_heading(format!(
            "小区变化记录 · {}",
            history.changes.len()
        )));
        ui.label(meta_text(
            "仅记录观测到的小区标识变化，不将它直接解释为切换失败或断线。最近保留 20 条。",
        ));
        if history.changes.is_empty() {
            ui.label("当前会话尚未观测到小区变化。");
        }
        for change in history.changes.iter().rev() {
            ui.separator();
            ui.label(meta_text(format!(
                "{} 秒前",
                SystemTime::now()
                    .duration_since(change.at)
                    .unwrap_or_default()
                    .as_secs()
            )));
            wrapped_label(ui, format!("原小区：{}", change.before));
            wrapped_label(ui, format!("新小区：{}", change.after));
        }
    });
}
fn signal_chart(ui: &mut Ui, history: &WirelessHistory) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 170.0),
        egui::Sense::hover(),
    );
    let plot = rect.shrink2(egui::vec2(40.0, 15.0));
    let painter = ui.painter_at(rect);
    for dbm in [-140, -110, -80, -50] {
        let y = plot.bottom() - ((dbm + 140) as f32 / 96.0) * plot.height();
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0_f32, scale::GRID),
        );
        painter.text(
            egui::pos2(plot.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            dbm.to_string(),
            egui::FontId::proportional(11.0),
            scale::SECONDARY,
        );
    }
    let mut previous = None;
    let mut count = 0;
    for (i, sample) in history.samples.iter().enumerate() {
        if let Some(rsrp) = sample.cell.as_ref().and_then(|c| c.rsrp_dbm) {
            count += 1;
            let x = plot.left()
                + i as f32 / history.samples.len().saturating_sub(1).max(1) as f32 * plot.width();
            let y =
                plot.bottom() - ((f32::from(rsrp) + 140.0) / 96.0).clamp(0.0, 1.0) * plot.height();
            let point = egui::pos2(x, y);
            if let Some(previous) = previous {
                painter.line_segment(
                    [previous, point],
                    egui::Stroke::new(2.0_f32, scale::DOWNLOAD),
                );
            }
            painter.circle_filled(point, 2.0, scale::DOWNLOAD);
            previous = Some(point);
        } else {
            previous = None;
        }
    }
    if count == 0 {
        painter.text(
            plot.center(),
            egui::Align2::CENTER_CENTER,
            "等待有效 RSRP 采样",
            egui::FontId::proportional(14.0),
            scale::SECONDARY,
        );
    }
    painter.text(
        rect.right_top(),
        egui::Align2::RIGHT_TOP,
        "dBm",
        egui::FontId::proportional(11.0),
        scale::SECONDARY,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cell(id: u32) -> ServingCell {
        serde_json::from_value(serde_json::json!({"rat":"LTE","pci":1,"earfcn":1650,"band":3,"rsrp_dbm":-95,"rsrq_db":-10,"cell_id":id})).unwrap()
    }
    #[test]
    fn history_deduplicates_samples_breaks_gaps_and_resets_context() {
        let mut h = WirelessHistory::default();
        let ctx = (DeviceEpoch(1), 0);
        let at = SystemTime::UNIX_EPOCH;
        h.push(ctx, at, Some(cell(1)));
        h.push(ctx, at, Some(cell(2)));
        assert_eq!(h.samples.len(), 1);
        assert!(h.changes.is_empty());
        h.push(ctx, at + std::time::Duration::from_secs(1), None);
        assert!(h.samples.back().unwrap().cell.is_none());
        h.push(ctx, at + std::time::Duration::from_secs(2), Some(cell(2)));
        assert_eq!(h.changes.len(), 1);
        h.push(
            (DeviceEpoch(1), 1),
            at + std::time::Duration::from_secs(3),
            Some(cell(3)),
        );
        assert_eq!(h.samples.len(), 1);
        assert!(h.changes.is_empty());
    }
    #[test]
    fn history_and_changes_are_bounded_and_sinr_stays_raw() {
        let mut h = WirelessHistory::default();
        for i in 0..150 {
            h.push(
                (DeviceEpoch(1), 0),
                SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(i),
                Some(cell(i as u32)),
            );
        }
        assert_eq!(h.samples.len(), 120);
        assert_eq!(h.changes.len(), 20);
        let mut c = cell(1);
        c.sinr_raw = Some(15);
        assert!(summary(&c).contains("SINR 原始值 15（单位未确认）"));
    }
}
