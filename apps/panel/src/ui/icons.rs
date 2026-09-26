//! Bundled Google Material Symbols outline icons; UI text always remains independently readable.
use eframe::egui::{self, RichText};
pub(crate) const MAIL: &str = "\u{e159}";
pub(crate) const EDIT: &str = "\u{f097}";
pub(crate) fn text(ctx: &egui::Context, glyph: &str, size: f32) -> RichText {
    if !ctx.data(|data| {
        data.get_temp::<bool>(egui::Id::new("panel-icons-installed"))
            .unwrap_or(false)
    }) {
        return RichText::new("");
    }
    RichText::new(glyph).font(egui::FontId::new(
        size,
        egui::FontFamily::Name("panel-icons".into()),
    ))
}
