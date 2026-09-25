use std::time::{Duration, SystemTime};

use dji4g_domain::{
    HostAdapter, HostNetworkFinding, HostNetworkObservation, HostProxyMode, ProxyBinding,
    ProxyClient, classify_host_network, host_observation_is_fresh,
};

fn observed(alias: &str, up: bool, binding: &str) -> HostNetworkObservation {
    HostNetworkObservation {
        adapters: vec![HostAdapter {
            guid: "{abc}".into(),
            luid: 1,
            alias: alias.into(),
            up,
        }],
        default_routes: vec![],
        system_proxy: HostProxyMode::Manual,
        binding: Some(ProxyBinding {
            client: ProxyClient::ClashVergeRev,
            version: Some("2.5.5".into()),
            interface_alias: binding.into(),
            repairable: true,
        }),
        proxy_inspection_complete: true,
        proxy_error_code: None,
        observed_at: SystemTime::UNIX_EPOCH,
    }
}

#[test]
fn missing_alias_is_distinct_from_a_present_but_down_adapter() {
    assert_eq!(
        classify_host_network(&observed("Wi-Fi", true, "Ethernet 5")),
        HostNetworkFinding::MissingBoundInterface
    );
    assert_eq!(
        classify_host_network(&observed("Ethernet 5", false, "Ethernet 5")),
        HostNetworkFinding::BoundInterfaceDown
    );
    assert_eq!(
        classify_host_network(&observed("以太网 5", true, "以太网 5")),
        HostNetworkFinding::NoKnownProblem
    );
}

#[test]
fn duplicate_alias_and_age_never_authorize_a_repair() {
    let mut observation = observed("Wi-Fi", true, "Wi-Fi");
    observation.adapters.push(HostAdapter {
        guid: "{def}".into(),
        luid: 2,
        alias: "wi-fi".into(),
        up: true,
    });
    assert_eq!(
        classify_host_network(&observation),
        HostNetworkFinding::BindingAmbiguous
    );
    assert!(!host_observation_is_fresh(
        observation.observed_at,
        observation.observed_at + Duration::from_secs(31)
    ));
}
