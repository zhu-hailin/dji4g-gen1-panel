use std::time::Duration;

use dji4g_windows_platform::ProbePolicy;

#[test]
fn default_probe_policy_is_small_and_bounded() {
    let policy = ProbePolicy::default();
    assert_eq!(policy.total_timeout, Duration::from_secs(10));
    assert!(policy.max_response_bytes <= 4096);
    assert!(policy.connect_timeout <= Duration::from_secs(3));
}
