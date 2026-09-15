//! The brand icon ships as repository assets and is decoded by three different consumers; pin
//! their exact expectations so a renamed or re-encoded asset fails loudly at test time.

#[test]
fn the_window_icon_decodes_at_full_brand_resolution() {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/brand/icon.png"))
        .expect("the window icon PNG must decode");
    assert_eq!((icon.width, icon.height), (256, 256));
    // Transparency survived the background removal (the chain logo is not a solid square).
    assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0));
    assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
}

#[test]
fn the_tray_icon_is_a_valid_32px_png() {
    let bytes = include_bytes!("../../../crates/windows-platform/assets/tray-32.png");
    let icon = eframe::icon_data::from_png_bytes(bytes).expect("the tray PNG must decode");
    assert_eq!((icon.width, icon.height), (32, 32));
}
