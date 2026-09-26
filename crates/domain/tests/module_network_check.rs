use dji4g_domain::{
    ModuleNetworkEvidence, ModuleNetworkVerdict, NetworkEvidenceState as S, classify_module_network,
};

#[test]
fn only_bound_public_and_dns_pass_prove_module_connectivity() {
    let mut e = ModuleNetworkEvidence::default();
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::Inconclusive
    );
    e.device = S::Passed;
    e.adapter = S::Passed;
    e.address_route = S::Passed;
    e.gateway = S::Passed;
    e.public = S::Passed;
    e.dns = S::Passed;
    assert_eq!(classify_module_network(&e), ModuleNetworkVerdict::Usable);
    e.dns = S::Failed;
    assert_eq!(classify_module_network(&e), ModuleNetworkVerdict::DnsIssue);
    e.public = S::Unavailable;
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::Inconclusive
    );
}

#[test]
fn unexecuted_or_stale_probe_never_proves_failure_or_success() {
    let mut e = ModuleNetworkEvidence {
        device: S::Passed,
        adapter: S::Passed,
        address_route: S::Passed,
        ..Default::default()
    };
    for state in [S::NotRun, S::Running, S::Unavailable, S::Stale] {
        e.public = state;
        e.dns = state;
        assert_eq!(
            classify_module_network(&e),
            ModuleNetworkVerdict::Inconclusive
        );
    }
}

#[test]
fn local_faults_are_separate_from_public_probe_failure() {
    let mut e = ModuleNetworkEvidence {
        device: S::Failed,
        ..Default::default()
    };
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::DeviceMissing
    );
    e.device = S::Passed;
    e.adapter = S::Failed;
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::AdapterIssue
    );
    e.adapter = S::Passed;
    e.address_route = S::Failed;
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::AddressRouteIssue
    );
}

#[test]
fn disconnected_link_and_gateway_failure_are_specific_not_driver_claims() {
    let mut e = ModuleNetworkEvidence {
        device: S::Passed,
        adapter: S::Passed,
        link: S::Failed,
        address_route: S::Failed,
        ..Default::default()
    };
    assert_eq!(classify_module_network(&e), ModuleNetworkVerdict::LinkDown);
    e.link = S::Passed;
    e.address_route = S::Passed;
    e.gateway = S::Failed;
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::GatewayIssue
    );
    e.gateway = S::Unavailable;
    assert_eq!(
        classify_module_network(&e),
        ModuleNetworkVerdict::Inconclusive
    );
}
