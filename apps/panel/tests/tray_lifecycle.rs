use dji4g_panel::tray::{
    CloseAction, TrayCommand, TrayLabels, WindowState, close_window, merge_activation,
    off_ui_command,
};

#[test]
fn tray_labels_and_command_mapping_are_closed_and_chinese() {
    let labels = TrayLabels::zh_cn();
    assert_eq!(
        labels.menu_items(),
        ["打开面板", "立即刷新", "热点状态", "退出"]
    );
    assert_eq!(TrayCommand::from_menu_id(1), Some(TrayCommand::Open));
    assert_eq!(TrayCommand::from_menu_id(2), Some(TrayCommand::RefreshNow));
    assert_eq!(
        TrayCommand::from_menu_id(3),
        Some(TrayCommand::HotspotStatus)
    );
    assert_eq!(TrayCommand::from_menu_id(4), Some(TrayCommand::Exit));
    assert_eq!(TrayCommand::from_menu_id(5), None);
}

#[test]
fn off_ui_commands_cover_exactly_the_frame_dependent_items() {
    // A window hidden to the tray runs no frames, so each command has exactly one owner that
    // works in that state: 「打开面板」 the native worker's ShowWindow, 「退出」 the hard-exit
    // backstop, and the two below the worker-side action hook. Nothing may be owned twice
    // (double refresh) or not at all (dead menu item).
    assert!(off_ui_command(TrayCommand::RefreshNow));
    assert!(off_ui_command(TrayCommand::HotspotStatus));
    assert!(
        !off_ui_command(TrayCommand::Open),
        "the worker re-shows the window natively; the hook must not touch it"
    );
    assert!(
        !off_ui_command(TrayCommand::Exit),
        "only the backstop and the graceful UI path may end the process"
    );
}

#[test]
fn close_hides_to_tray_until_explicit_exit() {
    let mut window = WindowState::default();
    assert_eq!(close_window(&mut window, false), CloseAction::HideToTray);
    assert!(!window.visible);
    assert_eq!(close_window(&mut window, true), CloseAction::Exit);
    assert!(window.explicit_exit);
}

#[test]
fn duplicate_activation_merges_without_losing_refresh_intent() {
    assert_eq!(
        merge_activation(None, TrayCommand::Open),
        Some(TrayCommand::Open)
    );
    assert_eq!(
        merge_activation(Some(TrayCommand::Open), TrayCommand::RefreshNow),
        Some(TrayCommand::RefreshNow)
    );
    assert_eq!(
        merge_activation(Some(TrayCommand::RefreshNow), TrayCommand::Open),
        Some(TrayCommand::RefreshNow)
    );
}
