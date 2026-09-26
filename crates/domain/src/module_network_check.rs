//! Evidence-only classification. Host routes and proxy settings cannot prove or disprove the
//! module's bound connectivity. Missing AT telemetry cannot override a successful network test.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetworkEvidenceState {
    #[default]
    NotRun,
    Running,
    Passed,
    Failed,
    Unavailable,
    Stale,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleNetworkEvidence {
    pub device: NetworkEvidenceState,
    pub adapter: NetworkEvidenceState,
    pub address_route: NetworkEvidenceState,
    pub link: NetworkEvidenceState,
    pub gateway: NetworkEvidenceState,
    pub public: NetworkEvidenceState,
    pub dns: NetworkEvidenceState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ModuleNetworkVerdict {
    #[default]
    Inconclusive,
    Usable,
    DeviceMissing,
    AdapterIssue,
    AddressRouteIssue,
    LinkDown,
    GatewayIssue,
    PublicProbeFailed,
    DnsIssue,
}

#[must_use]
pub fn classify_module_network(e: &ModuleNetworkEvidence) -> ModuleNetworkVerdict {
    use ModuleNetworkVerdict as V;
    use NetworkEvidenceState as S;
    if e.device == S::Failed {
        return V::DeviceMissing;
    }
    if e.device != S::Passed {
        return V::Inconclusive;
    }
    if e.adapter == S::Failed {
        return V::AdapterIssue;
    }
    if e.adapter != S::Passed {
        return V::Inconclusive;
    }
    if e.link == S::Failed {
        return V::LinkDown;
    }
    if e.address_route == S::Failed {
        return V::AddressRouteIssue;
    }
    if e.address_route != S::Passed {
        return V::Inconclusive;
    }
    if e.gateway == S::Failed {
        return V::GatewayIssue;
    }
    if e.gateway != S::Passed {
        return V::Inconclusive;
    }
    match (e.public, e.dns) {
        (S::Passed, S::Passed) => V::Usable,
        (S::Passed, S::Failed) => V::DnsIssue,
        (S::Failed, _) => V::PublicProbeFailed,
        _ => V::Inconclusive,
    }
}
