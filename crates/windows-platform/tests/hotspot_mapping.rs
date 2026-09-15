use dji4g_domain::{ErrorCode, HotspotStatus, HotspotUnsupportedReason};
use dji4g_windows_platform::{
    HotspotTransitionHint, ProfileDescriptor, map_capability, map_hresult, map_operation_status,
    map_operational_state, select_source_profile_index,
};

const DJI_GUID: &str = "{00112233-4455-6677-8899-aabbccddeeff}";
const META_GUID: &str = "{11112222-3333-4444-5555-666677778888}";
const PHONE_GUID: &str = "{99990000-aaaa-bbbb-cccc-ddddeeeeffff}";

#[test]
fn exact_dji_adapter_wins_regardless_of_profile_order() {
    let profiles = vec![
        ProfileDescriptor {
            ordinal: 17,
            adapter_id: PHONE_GUID.to_owned(),
        },
        ProfileDescriptor {
            ordinal: 4,
            adapter_id: DJI_GUID.to_ascii_lowercase(),
        },
        ProfileDescriptor {
            ordinal: 9,
            adapter_id: META_GUID.to_owned(),
        },
    ];

    assert_eq!(select_source_profile_index(DJI_GUID, &profiles), Ok(4));
}

#[test]
fn selector_accepts_winrt_guid_without_braces() {
    let target = DJI_GUID.trim_matches(['{', '}']);
    let profiles = vec![ProfileDescriptor {
        ordinal: 3,
        adapter_id: target.to_ascii_uppercase(),
    }];

    assert_eq!(select_source_profile_index(target, &profiles), Ok(3));
}

#[test]
fn selector_rejects_malformed_target_before_profile_fallback() {
    let error = select_source_profile_index("DJI Wi-Fi", &[])
        .expect_err("friendly name must never select a source profile");
    assert_eq!(error.code, "hotspot:target_adapter_id_invalid");
}

#[test]
fn unpackaged_state_change_is_disabled_by_default_but_read_policy_is_explicit() {
    let policy = dji4g_windows_platform::HotspotPolicy::default();
    assert!(!policy.allow_unpacked_state_change);
    assert_eq!(policy.operation_timeout, std::time::Duration::from_secs(30));
}

#[test]
fn selector_fails_closed_for_missing_ambiguous_or_malformed_profiles() {
    let missing = vec![ProfileDescriptor {
        ordinal: 0,
        adapter_id: META_GUID.to_owned(),
    }];
    assert_eq!(
        select_source_profile_index(DJI_GUID, &missing)
            .expect_err("missing source profile must fail"),
        dji4g_windows_platform::PlatformError {
            code: "hotspot:source_profile_unavailable",
            os_code: None,
        }
    );

    let ambiguous = vec![
        ProfileDescriptor {
            ordinal: 1,
            adapter_id: DJI_GUID.to_owned(),
        },
        ProfileDescriptor {
            ordinal: 2,
            adapter_id: DJI_GUID.to_ascii_lowercase(),
        },
    ];
    assert_eq!(
        select_source_profile_index(DJI_GUID, &ambiguous)
            .expect_err("duplicate source profile must fail"),
        dji4g_windows_platform::PlatformError {
            code: "hotspot:source_profile_ambiguous",
            os_code: None,
        }
    );

    let malformed = vec![ProfileDescriptor {
        ordinal: 1,
        adapter_id: "friendly-name-is-not-an-adapter-id".to_owned(),
    }];
    assert_eq!(
        select_source_profile_index(DJI_GUID, &malformed)
            .expect_err("malformed profile must fail closed"),
        dji4g_windows_platform::PlatformError {
            code: "hotspot:profile_adapter_id_invalid",
            os_code: None,
        }
    );
}

#[test]
fn capability_mapping_never_treats_unknown_as_enabled() {
    assert_eq!(
        map_capability(0),
        dji4g_windows_platform::HotspotCapabilityState::Enabled
    );
    assert_eq!(
        map_capability(1),
        dji4g_windows_platform::HotspotCapabilityState::Unsupported(
            HotspotUnsupportedReason::PolicyDisabled
        )
    );
    assert_eq!(
        map_capability(2),
        dji4g_windows_platform::HotspotCapabilityState::Unsupported(
            HotspotUnsupportedReason::NoWifiAdapter
        )
    );
    assert_eq!(
        map_capability(7),
        dji4g_windows_platform::HotspotCapabilityState::Unsupported(
            HotspotUnsupportedReason::MissingWifiControlCapability
        )
    );
    assert_eq!(
        map_capability(3),
        dji4g_windows_platform::HotspotCapabilityState::Disabled {
            code: "hotspot:operator_disabled",
        }
    );
    assert_eq!(
        map_capability(4),
        dji4g_windows_platform::HotspotCapabilityState::Disabled {
            code: "hotspot:sku_unsupported",
        }
    );
    assert_eq!(
        map_capability(5),
        dji4g_windows_platform::HotspotCapabilityState::Disabled {
            code: "hotspot:required_app_missing",
        }
    );
    assert_eq!(
        map_capability(0xffff),
        dji4g_windows_platform::HotspotCapabilityState::Disabled {
            code: "hotspot:capability_unknown",
        }
    );
}

#[test]
fn capability_mapping_covers_each_documented_disabled_cause() {
    for (raw, expected) in [
        (3, "hotspot:operator_disabled"),
        (4, "hotspot:sku_unsupported"),
        (5, "hotspot:required_app_missing"),
        (6, "hotspot:capability_unknown"),
    ] {
        assert_eq!(
            map_capability(raw),
            dji4g_windows_platform::HotspotCapabilityState::Disabled { code: expected }
        );
    }
}

#[test]
fn operational_mapping_preserves_on_when_client_count_fails() {
    let mapped = map_operational_state(
        1,
        None,
        Err(dji4g_windows_platform::PlatformError {
            code: "hotspot:client_count_failed",
            os_code: Some(5),
        }),
    );
    assert_eq!(mapped.status, HotspotStatus::On { clients: None });
    assert_eq!(mapped.client_count, None);
    assert_eq!(
        mapped.client_count_error,
        Some("hotspot:client_count_failed")
    );
}

#[test]
fn operational_mapping_keeps_off_on_counts_and_transition_hints_distinct() {
    assert_eq!(
        map_operational_state(2, None, Ok(0)).status,
        HotspotStatus::Off
    );
    assert_eq!(
        map_operational_state(1, None, Ok(0)).status,
        HotspotStatus::On { clients: Some(0) }
    );
    assert_eq!(
        map_operational_state(1, None, Ok(7)).status,
        HotspotStatus::On { clients: Some(7) }
    );
    assert_eq!(
        map_operational_state(3, Some(HotspotTransitionHint::Starting), Ok(0)).status,
        HotspotStatus::Starting
    );
    assert_eq!(
        map_operational_state(3, Some(HotspotTransitionHint::Stopping), Ok(0)).status,
        HotspotStatus::Stopping
    );
}

#[test]
fn transition_unknown_direction_and_unknown_state_are_not_off() {
    let transition = map_operational_state(3, None, Ok(0));
    assert_eq!(
        transition.status,
        HotspotStatus::Failed {
            code: ErrorCode::Internal
        }
    );
    assert_eq!(
        transition.stable_code,
        "hotspot:transition_direction_unknown"
    );

    let unknown = map_operational_state(99, Some(HotspotTransitionHint::Starting), Ok(0));
    assert_eq!(
        unknown.status,
        HotspotStatus::Failed {
            code: ErrorCode::Internal
        }
    );
    assert_eq!(unknown.stable_code, "hotspot:operational_state_unknown");
}

#[test]
fn operation_status_maps_known_errors_and_requires_final_readback() {
    let success_mismatch = map_operation_status(0, true, Some(HotspotStatus::Off));
    assert_eq!(
        success_mismatch.outcome,
        dji4g_windows_platform::HotspotOperationOutcome::Failed {
            code: ErrorCode::VerificationFailed,
        }
    );
    assert_eq!(success_mismatch.stable_code, "hotspot:final_state_mismatch");

    let wifi_off = map_operation_status(3, true, None);
    assert_eq!(
        wifi_off.outcome,
        dji4g_windows_platform::HotspotOperationOutcome::Failed {
            code: ErrorCode::CapabilityUnavailable,
        }
    );
    assert_eq!(wifi_off.stable_code, "hotspot:no_wifi_adapter");

    let unknown = map_operation_status(0xffff, false, None);
    assert_eq!(
        unknown.outcome,
        dji4g_windows_platform::HotspotOperationOutcome::OutcomeUnknown {
            code: ErrorCode::Internal,
        }
    );
    assert_eq!(unknown.stable_code, "hotspot:operation_status_unknown");

    let already_on_for_stop =
        map_operation_status(9, false, Some(HotspotStatus::On { clients: Some(1) }));
    assert_eq!(
        already_on_for_stop.outcome,
        dji4g_windows_platform::HotspotOperationOutcome::Failed {
            code: ErrorCode::VerificationFailed,
        }
    );

    let entitlement_timeout = map_operation_status(4, true, None);
    assert_eq!(
        entitlement_timeout.outcome,
        dji4g_windows_platform::HotspotOperationOutcome::Failed {
            code: ErrorCode::Timeout,
        }
    );
    let in_progress = map_operation_status(6, true, None);
    assert_eq!(in_progress.stable_code, "hotspot:operation_in_progress");
}

#[test]
fn operation_status_has_a_stable_mapping_for_every_documented_failure() {
    for (raw, expected) in [
        (2, "hotspot:mobile_broadband_off"),
        (3, "hotspot:no_wifi_adapter"),
        (4, "hotspot:entitlement_timeout"),
        (5, "hotspot:entitlement_failure"),
        (6, "hotspot:operation_in_progress"),
        (7, "hotspot:bluetooth_device_off"),
        (8, "hotspot:network_limited_connectivity"),
        (10, "hotspot:radio_restriction"),
        (11, "hotspot:band_interference"),
    ] {
        assert_eq!(map_operation_status(raw, true, None).stable_code, expected);
    }
    assert_eq!(
        map_operation_status(9, true, Some(HotspotStatus::On { clients: None })).outcome,
        dji4g_windows_platform::HotspotOperationOutcome::Applied
    );
}

#[test]
fn hresult_mapping_distinguishes_no_package_from_access_denied() {
    let no_package = map_hresult(0x8007_3d54, "status");
    assert_eq!(no_package.code, "hotspot:missing_package_identity");
    assert_eq!(no_package.os_code, Some(0x8007_3d54));

    let access_denied = map_hresult(0x8007_0005, "status");
    assert_eq!(access_denied.code, "hotspot:access_denied");
    assert_eq!(access_denied.os_code, Some(0x8007_0005));

    let wrong_apartment = map_hresult(0x8001_010e, "status");
    assert_eq!(wrong_apartment.code, "hotspot:wrong_apartment");

    for raw in [120, 50, 0x8000_000f, 0x8004_0154, 0x8000_4001] {
        assert_eq!(map_hresult(raw, "status").code, "hotspot:unsupported_os");
    }
}

#[test]
fn msix_manifest_declares_hotspot_and_full_trust_contract() {
    let manifest = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/msix/Package.appxmanifest"
    ));
    for required in [
        "ProcessorArchitecture=\"x64\"",
        "Name=\"runFullTrust\"",
        "Name=\"wiFiControl\"",
        "Category=\"windows.startupTask\"",
        "Enabled=\"false\"",
        "Resources",
        "Logo>Assets\\StoreLogo.png</Logo>",
        "Square150x150Logo=\"Assets\\Square150x150Logo.png\"",
        "Square44x44Logo=\"Assets\\Square44x44Logo.png\"",
    ] {
        assert!(manifest.contains(required), "manifest missing {required}");
    }
    assert!(!manifest.contains("requireAdministrator"));
    assert!(!manifest.contains("allowElevation"));
}

#[test]
fn msix_build_script_has_safe_dry_run_and_unsigned_default() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/scripts/build-msix.ps1"
    ));
    for required in [
        "[switch]$DryRun",
        "KitsRoot10",
        "makeappx.exe",
        "x64",
        "unsigned-development-only",
        "/h SHA256",
        "/no",
    ] {
        assert!(script.contains(required), "build script missing {required}");
    }
    assert!(!script.contains("New-SelfSignedCertificate"));
    assert!(!script.contains("Add-AppxPackage"));

    let dry_run_test = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/tests/build-msix-dry-run.ps1"
    ));
    assert!(dry_run_test.contains("-DryRun"));
    assert!(dry_run_test.contains("ConvertFrom-Json"));
    assert!(dry_run_test.contains("Dji4GPanel-assets-test-"));
}

#[test]
fn generated_msix_assets_are_exact_size_and_have_a_deterministic_source() {
    fn assert_png_size(bytes: &[u8], expected: u32) {
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            expected
        );
        assert_eq!(
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
            expected
        );
    }
    assert_png_size(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/msix/Assets/StoreLogo.png"
        )),
        50,
    );
    assert_png_size(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/msix/Assets/Square44x44Logo.png"
        )),
        44,
    );
    assert_png_size(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/msix/Assets/Square150x150Logo.png"
        )),
        150,
    );
    let generator = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/scripts/generate-msix-assets.ps1"
    ));
    for required in [
        "System.Drawing",
        "-Size 50",
        "-Size 44",
        "-Size 150",
        "uses no font",
    ] {
        assert!(
            generator.contains(required),
            "asset generator missing {required}"
        );
    }
    assert!(!generator.contains("SystemFonts"));
}
