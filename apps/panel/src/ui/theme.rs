//! Shared light Windows palette and logical-pixel scale.
use eframe::egui::{self, Color32, FontId, TextStyle};
pub(crate) mod scale {
    use eframe::egui::Color32;

    pub(crate) const INK: Color32 = Color32::from_rgb(0x20, 0x24, 0x2b);
    pub(crate) const SECONDARY: Color32 = Color32::from_rgb(0x62, 0x6b, 0x78);
    pub(crate) const LINE: Color32 = Color32::from_rgb(0xe2, 0xe6, 0xec);
    pub(crate) const DOWNLOAD: Color32 = Color32::from_rgb(0x4c, 0x6f, 0xff);
    pub(crate) const UPLOAD: Color32 = Color32::from_rgb(0xb2, 0x67, 0x00);
    /// Measured module temperature.  A hue of its own: the rate chart's blue and amber already mean
    /// download and upload, and the status tones own green/amber/red, so a violet line can never be
    /// read as a throughput series or as a verdict.
    pub(crate) const TEMPERATURE: Color32 = Color32::from_rgb(0x6b, 0x4f, 0xb5);
    pub(crate) const WARNING: Color32 = Color32::from_rgb(0x91, 0x61, 0x00);
    pub(crate) const AXIS_LABEL: Color32 = Color32::from_rgb(0x73, 0x73, 0x73);
    pub(crate) const GRID: Color32 = Color32::from_rgb(0xeb, 0xeb, 0xeb);
    pub(crate) const AXIS: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);

    pub(crate) const PAGE: f32 = 22.0;
    pub(crate) const SECTION: f32 = 16.0;
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
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(0xee, 0xee, 0xf0);
    visuals.window_fill = Color32::WHITE;
    visuals.faint_bg_color = Color32::from_rgb(0xf5, 0xf5, 0xf5);
    visuals.extreme_bg_color = Color32::from_rgb(0xed, 0xed, 0xed);
    visuals.widgets.noninteractive.bg_fill = Color32::WHITE;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, scale::INK);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xf4, 0xf4, 0xf5);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xe9, 0xed, 0xff);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0xda, 0xe1, 0xff);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.rounding = egui::Rounding::same(8.0);
        widget.bg_stroke = egui::Stroke::new(1.0_f32, scale::LINE);
        widget.fg_stroke = egui::Stroke::new(1.0_f32, scale::INK);
    }
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, scale::DOWNLOAD);
    visuals.selection.bg_fill = scale::DOWNLOAD;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    visuals.hyperlink_color = scale::DOWNLOAD;
    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.window_stroke = egui::Stroke::new(1.0_f32, scale::LINE);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(72.0, 36.0);

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(scale::PAGE)),
        (TextStyle::Body, FontId::proportional(scale::BODY)),
        (TextStyle::Button, FontId::proportional(scale::BUTTON)),
        (TextStyle::Monospace, FontId::monospace(scale::BODY)),
        (TextStyle::Small, FontId::proportional(scale::META)),
    ]
    .into();
    ctx.set_style(style);
}

/// Consistent, clearly visible primary action across page and confirmation surfaces.
pub(crate) fn primary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label.into())
            .color(Color32::WHITE)
            .size(scale::BUTTON),
    )
    .fill(scale::DOWNLOAD)
    .rounding(8.0)
    .min_size(egui::vec2(96.0, 36.0))
}
