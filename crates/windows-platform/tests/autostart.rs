use std::ffi::OsStr;

use dji4g_windows_platform::autostart::{
    AutostartControl, AutostartObservedState, InMemoryRunValueBackend, RunValue, RunValueType,
    classify_value, expected_command, quote_windows_arg,
};

#[test]
fn quote_windows_argument_handles_spaces_unicode_and_trailing_slashes() {
    let quoted = quote_windows_arg(OsStr::new(r"C:\Program Files\DJI\dji4g-panel.exe"))
        .expect("quoted path");
    assert_eq!(
        String::from_utf16(&quoted).unwrap(),
        r#""C:\Program Files\DJI\dji4g-panel.exe""#
    );

    let root = quote_windows_arg(OsStr::new(r"C:\")).expect("quoted root path");
    assert_eq!(String::from_utf16(&root).unwrap(), r#""C:\\""#);
    assert!(quote_windows_arg(OsStr::new("bad\"path")).is_err());
    assert!(quote_windows_arg(OsStr::new("bad\0path")).is_err());
}

#[test]
fn expected_run_command_is_current_exe_plus_closed_autostart_flag() {
    let command = expected_command(std::path::Path::new(r"C:\Portable Folder\panel.exe"))
        .expect("expected command");
    assert_eq!(
        String::from_utf16(&command).unwrap(),
        r#""C:\Portable Folder\panel.exe" --autostart"#
    );
}

#[test]
fn run_value_classification_requires_exact_owned_reg_sz_value() {
    let expected = "panel --autostart".encode_utf16().collect::<Vec<_>>();
    assert_eq!(
        classify_value(None, &expected),
        AutostartObservedState::Disabled
    );
    assert_eq!(
        classify_value(Some(&RunValue::reg_sz(expected.clone())), &expected),
        AutostartObservedState::Enabled
    );
    assert_eq!(
        classify_value(
            Some(&RunValue::reg_sz("other".encode_utf16().collect())),
            &expected
        ),
        AutostartObservedState::Drift
    );
    assert_eq!(
        classify_value(
            Some(&RunValue::new(RunValueType::Other(2), expected.clone())),
            &expected
        ),
        AutostartObservedState::Drift
    );
    assert_eq!(
        classify_value(Some(&RunValue::reg_sz(vec![0xD800])), &expected),
        AutostartObservedState::Drift
    );
}

#[test]
fn disabling_only_deletes_an_exact_owned_value() {
    let backend = InMemoryRunValueBackend::with(RunValue::reg_sz(
        "panel --autostart".encode_utf16().collect(),
    ));
    let control =
        AutostartControl::with_backend(std::path::PathBuf::from(r"C:\panel.exe"), backend.clone())
            .expect("control");
    assert_eq!(
        control.set_enabled(false).unwrap(),
        AutostartObservedState::Drift
    );
    assert!(backend.read_value().is_some());
}
