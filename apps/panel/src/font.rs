//! Windows system-font loading for the Chinese-first panel.
//!
//! egui's bundled fonts intentionally stay small and do not contain CJK glyphs. The panel therefore
//! uses an installed Windows system font as the first family entry, while retaining egui's bundled
//! Latin and symbol fallbacks. CJK fonts are not redistributed. A small licensed Material Symbols icon subset is bundled.

use std::path::{Path, PathBuf};

use eframe::egui::{self, FontFamily};

pub const CJK_FONT_NAME: &str = "dji4g-cjk-system";
pub const CJK_FONT_ERROR_CODE: &str = "ui:cjk_font_unavailable";

const CJK_FONT_FILES: [&str; 4] = ["msyh.ttc", "msyhbd.ttc", "simhei.ttf", "simsun.ttc"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontInstallError {
    CjkFontUnavailable,
}

impl FontInstallError {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::CjkFontUnavailable => CJK_FONT_ERROR_CODE,
        }
    }
}

#[derive(Debug)]
pub struct LoadedCjkFont {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl LoadedCjkFont {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Return the fixed filename order used below a Windows installation root.
///
/// This helper is kept separate from environment access so fallback ordering can be tested without
/// changing process-global variables.
#[must_use]
pub(crate) fn candidate_paths_for_root(root: &Path) -> Vec<PathBuf> {
    CJK_FONT_FILES
        .iter()
        .map(|name| root.join("Fonts").join(name))
        .collect()
}

/// Resolve only the Windows system font directory and a fixed fallback root.
///
/// `WINDIR` is accepted only when it is an absolute path without parent-directory components;
/// otherwise the fixed `C:\\Windows\\Fonts` fallback is used. No user-configured font path is
/// consulted.
#[must_use]
pub fn windows_font_candidates() -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(2);
    if let Some(windir) = std::env::var_os("WINDIR") {
        let path = PathBuf::from(windir);
        if path.is_absolute()
            && path
                .components()
                .all(|component| !matches!(component, std::path::Component::ParentDir))
        {
            roots.push(path);
        }
    }
    let fallback = PathBuf::from(r"C:\Windows");
    if !roots.iter().any(|root| root == &fallback) {
        roots.push(fallback);
    }

    let mut candidates = Vec::with_capacity(roots.len() * CJK_FONT_FILES.len());
    for root in roots {
        for path in candidate_paths_for_root(&root) {
            if !candidates.iter().any(|candidate| candidate == &path) {
                candidates.push(path);
            }
        }
    }
    candidates
}

/// Select the first non-empty candidate using an injected loader.
///
/// Keeping filesystem access behind this function makes fallback behavior deterministic and keeps
/// the production path from ever guessing at a font outside the fixed candidate list.
pub(crate) fn load_first_font<F, E>(
    candidates: &[PathBuf],
    mut load: F,
) -> Result<LoadedCjkFont, FontInstallError>
where
    F: FnMut(&Path) -> Result<Vec<u8>, E>,
{
    for path in candidates {
        if let Ok(bytes) = load(path) {
            if !bytes.is_empty() {
                return Ok(LoadedCjkFont {
                    path: path.clone(),
                    bytes,
                });
            }
        }
    }
    Err(FontInstallError::CjkFontUnavailable)
}

/// Install the selected system font before the first egui pass.
pub fn install_chinese_font(ctx: &egui::Context) -> Result<PathBuf, FontInstallError> {
    let loaded = load_first_font(&windows_font_candidates(), |path| std::fs::read(path))?;
    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert(
        CJK_FONT_NAME.to_owned(),
        egui::FontData::from_owned(loaded.bytes),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let names = definitions.families.entry(family).or_default();
        names.retain(|name| name != CJK_FONT_NAME);
        names.insert(0, CJK_FONT_NAME.to_owned());
    }
    definitions.font_data.insert(
        "material-symbols".into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/material/MaterialSymbolsOutlined.ttf"
        )),
    );
    definitions.families.insert(
        FontFamily::Name("panel-icons".into()),
        vec!["material-symbols".into()],
    );
    ctx.set_fonts(definitions);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("panel-icons-installed"), true));
    Ok(loaded.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_keep_microsoft_yahei_first_and_fixed_fallback_order() {
        let paths = candidate_paths_for_root(Path::new(r"C:\Windows"));
        let names = paths
            .iter()
            .map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                Some("msyh.ttc"),
                Some("msyhbd.ttc"),
                Some("simhei.ttf"),
                Some("simsun.ttc")
            ]
        );
    }

    #[test]
    fn loader_uses_the_first_successful_non_empty_fallback() {
        let candidates = candidate_paths_for_root(Path::new(r"C:\Windows"));
        let selected = load_first_font(&candidates, |path| {
            if path.file_name().and_then(|name| name.to_str()) == Some("msyh.ttc") {
                Err(())
            } else {
                Ok(vec![1, 2, 3])
            }
        })
        .expect("second candidate should be selected");
        assert_eq!(
            selected.path().file_name().and_then(|name| name.to_str()),
            Some("msyhbd.ttc")
        );
        assert_eq!(selected.bytes(), &[1, 2, 3]);
    }

    #[test]
    fn loader_reports_one_stable_code_when_all_candidates_fail() {
        let candidates = candidate_paths_for_root(Path::new(r"C:\Windows"));
        let error = load_first_font(&candidates, |_path| -> Result<Vec<u8>, ()> { Err(()) })
            .expect_err("all candidates should fail");
        assert_eq!(error.stable_code(), CJK_FONT_ERROR_CODE);
    }

    #[cfg(windows)]
    #[test]
    fn local_windows_installation_has_a_cjk_candidate() {
        let candidates = windows_font_candidates();
        assert!(
            candidates.iter().any(|path| path.is_file()),
            "expected at least one fixed Windows CJK font candidate"
        );
    }

    #[cfg(windows)]
    #[test]
    fn installed_font_lays_out_required_panel_labels_without_missing_glyphs() {
        let context = egui::Context::default();
        install_chinese_font(&context).expect("local Windows CJK font should load");
        let _ = context.run(egui::RawInput::default(), |_| {});
        let _ = context.run(egui::RawInput::default(), |_| {});
        let has_glyphs = context.fonts(|fonts| {
            let font_id = egui::FontId::proportional(14.0);
            ["模块网络：受限", "诊断", "设置", "●○▲■→"]
                .iter()
                .all(|text| fonts.has_glyphs(&font_id, text))
        });
        assert!(has_glyphs);
    }
}
