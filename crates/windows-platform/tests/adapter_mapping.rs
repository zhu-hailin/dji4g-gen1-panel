use std::str::FromStr;

use dji4g_windows_platform::AdapterGuid;

#[test]
fn adapter_guid_is_strict_and_canonical() {
    let guid = AdapterGuid::from_str("{8BDD4C57-901D-4A19-B85B-970F32B4C41A}").unwrap();
    assert_eq!(guid.canonical(), "{8bdd4c57-901d-4a19-b85b-970f32b4c41a}");
    assert!(AdapterGuid::from_str("8BDD4C57-901D-4A19-B85B-970F32B4C41A").is_err());
    assert!(AdapterGuid::from_str("{8BDD4C57-901D-4A19-B85B-970F32B4C41Z}").is_err());
}
