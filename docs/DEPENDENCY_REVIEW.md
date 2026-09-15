# Dependency review — 2026-09-15

The Windows CI build, Clippy and full test suite passed. The initial cargo-deny run identified the following release-policy omissions.

- `clipboard-win 5.4.1` and `error-code 3.4.0` use Boost Software License 1.0 (BSL-1.0). This permissive license is explicitly allowed. Its original text is included in THIRD-PARTY-NOTICES.txt.
- `epaint_default_fonts 0.29.1` contains unchanged fallback fonts under OFL-1.1 and Ubuntu Font Licence 1.0, in addition to its Rust code licenses. The exception is limited to this exact crate/version. Original license texts and notices extracted from the bundled font name tables ship with portable ZIP and MSIX. Windows system fonts are not redistributed.
- [RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192.html) is an informational unmaintained notice for `ttf-parser`, pulled through `ab_glyph` and egui. There is no patched version. This development release retains the existing renderer and records one advisory-specific exception. This is acceptance of maintenance risk, not a vulnerability fix. Only embedded fallback fonts and fixed Windows font paths are used; there is no font download/import feature. All other advisories remain gated.

Reassess the renderer/parser dependency before adding external font import or by 2026-12-31. Consider a maintained fontations/skrifa based rendering path when supported by the UI stack. Do not broaden this exception to new advisories.

References: [Boost license](https://spdx.org/licenses/BSL-1.0.html), [OFL](https://spdx.org/licenses/OFL-1.1.html), [Ubuntu font license](https://canonical.com/legal/font-licence).
