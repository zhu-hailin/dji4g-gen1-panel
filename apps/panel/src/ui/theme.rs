//! Shared light Material palette and logical-pixel scale.
use eframe::egui::{self, Color32, FontId, TextStyle};
pub(crate) mod scale {
    use eframe::egui::Color32;

    pub(crate) const INK: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
    pub(crate) const SECONDARY: Color32 = Color32::from_rgb(0x44, 0x47, 0x46);
    pub(crate) const LINE: Color32 = Color32::from_rgb(0xda, 0xdc, 0xe0);
    pub(crate) const DOWNLOAD: Color32 = Color32::from_rgb(0x0b, 0x57, 0xd0);
    pub(crate) const UPLOAD: Color32 = Color32::from_rgb(0xb2, 0x67, 0x00);
    /// Measured module temperature.  A hue of its own: the rate chart's blue and amber already mean
    /// download and upload, and the status tones own green/amber/red, so a violet line can never be
    /// read as a throughput series or as a verdict.
    pub(crate) const TEMPERATURE: Color32 = Color32::from_rgb(0x6b, 0x4f, 0xb5);
    pub(crate) const WARNING: Color32 = Color32::from_rgb(0x91, 0x61, 0x00);
    pub(crate) const AXIS_LABEL: Color32 = Color32::from_rgb(0x62, 0x67, 0x6c);
    pub(crate) const GRID: Color32 = Color32::from_rgb(0xeb, 0xeb, 0xeb);
    pub(crate) const AXIS: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);

    pub(crate) const PAGE: f32 = 24.0;
    pub(crate) const SECTION: f32 = 18.0;
    pub(crate) const LABEL: f32 = 14.0;
    pub(crate) const BODY: f32 = 14.0;
    pub(crate) const BUTTON: f32 = 14.0;
    pub(crate) const META: f32 = 12.0;
    pub(crate) const RATE_NUMBER: f32 = 34.0;
    pub(crate) const RATE_AUX: f32 = 13.0;

    pub(crate) const MUTED: Color32 = SECONDARY;
    pub(crate) const FAINT: Color32 = AXIS_LABEL;
    pub(crate) const DETAIL: Color32 = Color32::from_rgb(0x55, 0x5f, 0x6b);

    pub(crate) const SECTION_MARGIN: [f32; 2] = [16.0, 16.0];
    pub(crate) const SECTION_GAP: f32 = 16.0;
    pub(crate) const COLUMN_GAP: f32 = 16.0;
    pub(crate) const ROW_GAP: f32 = 12.0;
    pub(crate) const CONTROL_GAP: [f32; 2] = [8.0, 8.0];

    pub(crate) const LABEL_COLUMN: f32 = 96.0;
}

pub(crate) fn style_root(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(0xf0, 0xf4, 0xf9);
    visuals.window_fill = Color32::WHITE;
    visuals.faint_bg_color = Color32::from_rgb(0xf5, 0xf5, 0xf5);
    visuals.extreme_bg_color = Color32::from_rgb(0xed, 0xed, 0xed);
    visuals.widgets.noninteractive.bg_fill = Color32::WHITE;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, scale::INK);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xf1, 0xf3, 0xf4);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xe8, 0xf0, 0xfe);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0xd3, 0xe3, 0xfd);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.rounding = egui::Rounding::same(20.0);
        widget.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(116, 119, 117));
        widget.fg_stroke = egui::Stroke::new(1.0_f32, scale::INK);
    }
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, scale::DOWNLOAD);
    visuals.selection.bg_fill = scale::DOWNLOAD;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    visuals.hyperlink_color = scale::DOWNLOAD;
    visuals.window_rounding = egui::Rounding::same(28.0);
    visuals.window_stroke = egui::Stroke::NONE;

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, scale::SECTION_GAP);
    style.spacing.button_padding = egui::vec2(20.0, 10.0);
    style.spacing.window_margin = egui::Margin::same(24.0);
    style.visuals.extreme_bg_color = Color32::from_rgb(240, 244, 249);
    style.spacing.interact_size = egui::vec2(80.0, 40.0);

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(scale::PAGE)),
        (TextStyle::Body, FontId::proportional(scale::BODY)),
        (TextStyle::Button, FontId::proportional(scale::BUTTON)),
        (TextStyle::Monospace, FontId::monospace(scale::BODY)),
        (TextStyle::Small, FontId::proportional(scale::META)),
    ]
    .into();
    // Keep both style slots consistent: system preference events must not restore tiny defaults.
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Consistent, clearly visible primary action across page and confirmation surfaces.
pub(crate) fn primary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label.into())
            .color(Color32::WHITE)
            .size(scale::BUTTON),
    )
    .fill(scale::DOWNLOAD)
    .rounding(24.0)
    .min_size(egui::vec2(112.0, 48.0))
}

#[cfg(test)]
mod material_regressions {
    use super::*;

    #[test]
    fn light_theme_keeps_controls_sized_after_system_theme_changes() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Dark);
        style_root(&ctx);
        ctx.set_theme(egui::Theme::Light);
        assert!(ctx.style().spacing.interact_size.y >= 40.0);
        assert_eq!(ctx.style().visuals.selection.bg_fill, scale::DOWNLOAD);
    }
}
