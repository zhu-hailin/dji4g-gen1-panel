//! Material-inspired controls shared by every page. Business commands remain at call sites.
use super::scale;
use eframe::egui::{self, Color32, Response, RichText, Stroke, Ui};

#[derive(Clone, Copy)]
pub(crate) enum ButtonKind {
    Filled,
    Tonal,
    Outlined,
    Text,
    Destructive,
}

pub(crate) fn action_button(
    ui: &mut Ui,
    label: &str,
    kind: ButtonKind,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> Response {
    let (fill, ink, stroke) = match kind {
        ButtonKind::Filled => (scale::DOWNLOAD, Color32::WHITE, Stroke::NONE),
        ButtonKind::Tonal => (Color32::from_rgb(211, 227, 253), scale::INK, Stroke::NONE),
        ButtonKind::Outlined => (
            Color32::TRANSPARENT,
            scale::DOWNLOAD,
            Stroke::new(1.0_f32, scale::SECONDARY),
        ),
        ButtonKind::Text => (Color32::TRANSPARENT, scale::DOWNLOAD, Stroke::NONE),
        ButtonKind::Destructive => (
            Color32::TRANSPARENT,
            Color32::from_rgb(170, 48, 48),
            Stroke::new(1.0_f32, Color32::from_rgb(170, 48, 48)),
        ),
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(ink))
            .fill(fill)
            .stroke(stroke)
            .rounding(20.0)
            .min_size(egui::vec2(80.0, 40.0)),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            22.0,
            Stroke::new(2.0_f32, scale::DOWNLOAD),
        );
    }
    match disabled_reason.filter(|_| !enabled) {
        Some(reason) => response.on_disabled_hover_text(reason),
        None => response,
    }
}

pub(crate) struct TabItem<T> {
    pub value: T,
    pub label: String,
}
impl<T> TabItem<T> {
    pub(crate) fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

pub(crate) fn page_tabs<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: egui::Id,
    selected: &mut T,
    items: &[TabItem<T>],
) -> bool {
    selection(ui, id, selected, items, false)
}
pub(crate) fn segmented_control<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: egui::Id,
    selected: &mut T,
    items: &[TabItem<T>],
) -> bool {
    selection(ui, id, selected, items, true)
}
fn selection<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: egui::Id,
    selected: &mut T,
    items: &[TabItem<T>],
    segmented: bool,
) -> bool {
    let mut changed = false;
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = if segmented { 0.0 } else { 8.0 };
            for (index, item) in items.iter().enumerate() {
                ui.push_id(index, |ui| {
                    let active = *selected == item.value;
                    let fill = if active && segmented {
                        Color32::from_rgb(211, 227, 253)
                    } else {
                        Color32::TRANSPARENT
                    };
                    let button = egui::Button::new(RichText::new(&item.label).color(if active {
                        scale::DOWNLOAD
                    } else {
                        scale::SECONDARY
                    }))
                    .fill(fill)
                    .stroke(if segmented {
                        Stroke::new(1.0_f32, Color32::from_rgb(116, 119, 117))
                    } else {
                        Stroke::NONE
                    })
                    .rounding(if segmented {
                        egui::Rounding {
                            nw: if index == 0 { 20.0 } else { 0.0 },
                            sw: if index == 0 { 20.0 } else { 0.0 },
                            ne: if index + 1 == items.len() { 20.0 } else { 0.0 },
                            se: if index + 1 == items.len() { 20.0 } else { 0.0 },
                        }
                    } else {
                        egui::Rounding::same(4.0)
                    })
                    .min_size(egui::vec2(112.0, if segmented { 40.0 } else { 48.0 }));
                    let response = ui.add(button);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            ui.is_enabled(),
                            active,
                            &item.label,
                        )
                    });
                    if active && !segmented {
                        ui.painter().line_segment(
                            [
                                response.rect.left_bottom() + egui::vec2(12.0, -2.0),
                                response.rect.right_bottom() + egui::vec2(-12.0, -2.0),
                            ],
                            Stroke::new(3.0_f32, scale::DOWNLOAD),
                        );
                    }
                    if response.has_focus() {
                        ui.painter().rect_stroke(
                            response.rect.shrink(2.0),
                            4.0,
                            Stroke::new(2.0_f32, scale::DOWNLOAD),
                        );
                    }
                    if response.clicked() && !active {
                        *selected = item.value;
                        changed = true;
                    }
                });
            }
        });
    });
    changed
}

/// A content-measured footer: the explanation cannot be covered by a fixed-height action row.
pub(crate) fn entry_footer(ui: &mut Ui, label: &str) -> Response {
    let explanation = |ui: &mut Ui| {
        ui.label(RichText::new("可以直接进入，稍后继续检查。").color(scale::SECONDARY));
        ui.label(
            RichText::new("进入后不再自动显示；可在设置中重新打开。")
                .size(scale::META)
                .color(scale::SECONDARY),
        );
    };
    if ui.available_width() >= 520.0 {
        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2((width - 220.0).max(240.0), 48.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width((width - 220.0).max(240.0));
                    explanation(ui);
                },
            );
            ui.add(super::theme::primary_button(label))
        })
        .inner
    } else {
        explanation(ui);
        ui.add(super::theme::primary_button(label))
    }
}

/// Material switch, with a 52x48 interaction target and native egui keyboard semantics.
pub(crate) fn switch(ui: &mut Ui, value: &mut bool, label: &str, enabled: bool) -> Response {
    let mut response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .min_size(egui::vec2(52.0, 48.0)),
    );
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *value, label)
    });
    let rect = egui::Rect::from_center_size(response.rect.center(), egui::vec2(52.0, 32.0));
    let track = if !enabled {
        Color32::from_rgb(224, 227, 231)
    } else if *value {
        scale::DOWNLOAD
    } else {
        Color32::from_rgb(225, 227, 225)
    };
    ui.painter().rect(
        rect,
        16.0,
        track,
        if *value {
            Stroke::NONE
        } else {
            Stroke::new(
                2.0_f32,
                scale::SECONDARY.gamma_multiply(if enabled { 1.0 } else { 0.4 }),
            )
        },
    );
    let x = if *value {
        rect.right() - 16.0
    } else {
        rect.left() + 16.0
    };
    ui.painter().circle_filled(
        egui::pos2(x, rect.center().y),
        if *value { 12.0 } else { 8.0 },
        if *value {
            Color32::WHITE
        } else {
            scale::SECONDARY
        },
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            18.0,
            Stroke::new(2.0_f32, scale::DOWNLOAD),
        );
    }
    response.on_hover_text(label)
}

pub(crate) fn page_heading(ui: &mut Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(28.0).color(scale::INK));
    if !description.is_empty() {
        ui.label(
            RichText::new(description)
                .size(14.0)
                .color(scale::SECONDARY),
        );
    }
    ui.add_space(8.0);
}

pub(crate) fn status_banner(ui: &mut Ui, title: &str, detail: &str, tone: super::StatusTone) {
    let fill = match tone {
        super::StatusTone::Positive => Color32::from_rgb(232, 245, 233),
        super::StatusTone::Negative => Color32::from_rgb(252, 232, 230),
        super::StatusTone::Caution => Color32::from_rgb(254, 247, 224),
        _ => Color32::from_rgb(232, 240, 254),
    };
    egui::Frame::none()
        .fill(fill)
        .rounding(16.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal_wrapped(|ui| {
                let glyph = match tone {
                    super::StatusTone::Positive => "\u{f0be}",
                    super::StatusTone::Negative | super::StatusTone::Caution => "\u{f8b6}",
                    _ => "\u{e88e}",
                };
                ui.label(super::icons::text(ui.ctx(), glyph, 24.0).color(tone.color()));
                ui.label(RichText::new(title).size(16.0).strong().color(tone.color()));
            });
            ui.label(RichText::new(detail).color(scale::SECONDARY));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tabs_switch_only_after_a_click_and_disabled_actions_do_not_fire() {
        let ctx = egui::Context::default();
        super::super::style_root(&ctx);
        let mut selected = false;
        let mut fired = 0;
        for tick in 0..3 {
            let events = if tick == 0 {
                Vec::new()
            } else {
                vec![
                    egui::Event::PointerMoved(egui::pos2(180.0, 30.0)),
                    egui::Event::PointerButton {
                        pos: egui::pos2(180.0, 30.0),
                        button: egui::PointerButton::Primary,
                        pressed: tick == 1,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        page_tabs(
                            ui,
                            egui::Id::new("click-tabs"),
                            &mut selected,
                            &[
                                TabItem::new(false, "连接概况"),
                                TabItem::new(true, "无线观测"),
                            ],
                        );
                        if action_button(
                            ui,
                            "不可执行",
                            ButtonKind::Filled,
                            false,
                            Some("通信忙碌"),
                        )
                        .clicked()
                        {
                            fired += 1;
                        }
                    });
                },
            );
            if tick < 2 {
                assert!(!selected);
            }
        }
        assert!(selected);
        assert_eq!(fired, 0);
    }

    #[test]
    fn keyboard_activation_emits_one_click_and_keeps_focus() {
        let ctx = egui::Context::default();
        super::super::style_root(&ctx);
        let mut id = None;
        let mut clicks = 0;
        for tick in 0..3 {
            if let Some(id) = id {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            let events = if tick == 0 {
                vec![]
            } else {
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: tick == 1,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            };
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = action_button(ui, "检查", ButtonKind::Filled, true, None);
                        id = Some(response.id);
                        if response.clicked() {
                            clicks += 1;
                        }
                        if tick > 0 {
                            assert!(response.has_focus());
                        }
                    });
                },
            );
        }
        assert_eq!(clicks, 1);
    }

    #[test]
    fn footer_and_navigation_fit_small_content_widths() {
        for width in [260.0, 480.0, 800.0] {
            let ctx = egui::Context::default();
            super::super::style_root(&ctx);
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = entry_footer(ui, "进入面板");
                        assert!(response.rect.height() >= 40.0);
                        assert!(ui.clip_rect().contains_rect(response.rect));
                        let mut choice = false;
                        assert!(!page_tabs(
                            ui,
                            egui::Id::new("test"),
                            &mut choice,
                            &[
                                TabItem::new(false, "连接概况"),
                                TabItem::new(true, "无线观测")
                            ]
                        ));
                        assert!(!choice);
                    });
                },
            );
        }
    }
}
