//! Integration guards for the ordinary (non-demo) production startup path.
//!
//! These tests use only the public composition surface plus source-level guarantees.  The detailed
//! port-mapping and fail-closed behavior is exercised by the unit tests in `src/runtime.rs`; here we
//! pin the properties that must hold for a shipped executable: the ordinary path builds the real
//! composition, the privileged boundary refuses without a signed helper, and no test/demo fallback
//! or fabricated proof survives in the production sources.

use std::time::SystemTime;

use dji4g_application::DeviceEpoch;
use dji4g_panel::runtime::ProductionComposition;
use dji4g_windows_platform::TrustedHelper;

#[test]
fn ordinary_composition_exposes_all_real_monitor_ports() {
    let composition = ProductionComposition::new(SystemTime::UNIX_EPOCH);
    assert!(composition.has_all_real_ports());

    let (controller, ports) = composition.into_parts();
    // A fresh production controller starts at epoch 0 (no device evidence yet) and every monitor
    // port, including the optional hotspot port, is populated with a real backend.
    assert_eq!(controller.state().epoch(), DeviceEpoch(0));
    assert!(ports.hotspot.is_some());
}

#[test]
fn ordinary_main_never_constructs_the_fake_controller() {
    let source = include_str!("../src/main.rs");
    assert!(source.contains("ProductionComposition"));
    assert!(source.contains("with_ports"));
    assert!(!source.contains("Controller::for_test"));
    assert!(!source.contains("FakeActionExecutor"));
}

#[test]
fn runtime_wires_real_backends_without_placeholders() {
    let source = include_str!("../src/runtime.rs");

    // Every monitor/action port must delegate to a concrete Windows backend.
    for marker in [
        "WindowsDeviceInventory",
        "WindowsAdapterResolver",
        "WindowsNetworkProbe",
        "WindowsHotspotControl",
        "WindowsRepairExecutor",
        "AtSessionActor",
    ] {
        assert!(
            source.contains(marker),
            "runtime.rs must delegate to {marker}"
        );
    }

    // The privileged path must really go through the trusted-helper boundary.
    for marker in [
        "TrustedHelper::installed()",
        "launch_elevated_helper",
        "execute_via_helper",
        "build_helper_request",
    ] {
        assert!(
            source.contains(marker),
            "runtime.rs must wire the helper boundary via {marker}"
        );
    }

    // No fabricated proofs, demos, or unfinished code may remain in the production runtime or
    // the optional-probe module it drives.
    let sources = [
        ("runtime.rs", include_str!("../src/runtime.rs")),
        ("feature_probe.rs", include_str!("../src/feature_probe.rs")),
    ];
    for (file, source) in sources {
        for forbidden in [
            "[0; 32]",
            "Controller::for_test",
            "FakeActionExecutor",
            "todo!",
            "unimplemented!",
        ] {
            assert!(
                !source.contains(forbidden),
                "{file} must not contain the placeholder {forbidden:?}"
            );
        }
    }
}

#[test]
fn privileged_boundary_fails_closed_without_a_signed_helper() {
    // A development/test checkout has no installed, signature-verified sibling helper.  The
    // `TrustedHelper::installed()` gate itself still refuses — the dev-mode exemption lives one
    // layer above (in `execute_via_helper`, gated on this process being unsigned), so the strict
    // boundary cannot be downgraded by a signed installation.
    assert!(TrustedHelper::installed().is_err());
    // The dev-mode exemption is only reachable when this process is itself unsigned, which is
    // exactly the case in a development checkout.
    assert!(dji4g_windows_platform::is_dev_build());
}
