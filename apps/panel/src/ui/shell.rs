//! Fixed shell surfaces; only the central page scrolls.
use super::scale;
use crate::app::{NAV_ITEMS, Page};
use crate::localization::{Language, LocalizedText};
use eframe::egui::{self, Color32, RichText, Ui};
pub(crate) fn navigation(ui: &mut Ui, current: &mut Page, language: Language) {
    for (page, key) in NAV_ITEMS.into_iter().filter(|(p, _)| *p != Page::Settings) {
        nav_item(ui, current, page, LocalizedText::new(language, key).text);
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        nav_item(ui, current, Page::Settings, "设置".into());
    });
}
fn nav_item(ui: &mut Ui, current: &mut Page, page: Page, label: String) {
    let selected = *current == page;
    let button = egui::Button::new(RichText::new(label).size(16.0).color(if selected {
        scale::DOWNLOAD
    } else {
        scale::INK
    }))
    .fill(if selected {
        Color32::from_rgb(0xde, 0xe3, 0xf8)
    } else {
        Color32::TRANSPARENT
    })
    .stroke(egui::Stroke::NONE)
    .rounding(10.0);
    if ui.add_sized([ui.available_width(), 44.0], button).clicked() {
        *current = page;
    }
}
pub(crate) fn sidebar_width(viewport_width: f32) -> f32 {
    if viewport_width < 960.0 { 144.0 } else { 176.0 }
}
