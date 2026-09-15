use std::{
    io::Read,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant, SystemTime},
};

#[cfg(test)]
use std::sync::{Arc, Mutex};

use dji4g_domain::{DefaultRouteOwner, DeviceEpoch};

use crate::{
    AdapterIdentity, AddressFamily, PlatformError, RouteObservation, adapter::is_usable_source,
};

const MAX_STAGE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const BLOCKING_TIMER_MARGIN: Duration = Duration::from_millis(25);

#[cfg(test)]
const IF_TYPE_ETHERNET_RAW: u32 = 6;
const IF_TYPE_PPP_RAW: u32 = 23;
const IF_TYPE_PROP_VIRTUAL_RAW: u32 = 53;
const IF_TYPE_TUNNEL_RAW: u32 = 131;
#[cfg(test)]
const NDIS_MEDIUM_802_3_RAW: i32 = 0;
const NDIS_MEDIUM_TUNNEL_RAW: i32 = 15;
const NDIS_MEDIUM_IP_RAW: i32 = 19;
const NDIS_PHYSICAL_MEDIUM_UNSPECIFIED_RAW: i32 = 0;
#[cfg(test)]
const NDIS_PHYSICAL_MEDIUM_802_3_RAW: i32 = 14;
const NET_IF_ACCESS_BROADCAST_RAW: i32 = 2;
const NET_IF_ACCESS_POINT_TO_POINT_RAW: i32 = 3;

#[derive(Clone, Copy, Debug)]
struct RawInterfaceKind {
    interface_type: u32,
    tunnel_type: i32,
    media_type: i32,
    physical_medium_type: i32,
    access_type: i32,
}

fn classify_global_interface(raw: RawInterfaceKind) -> DefaultRouteOwner {
    let typed_virtual = matches!(
        raw.interface_type,
        IF_TYPE_TUNNEL_RAW | IF_TYPE_PPP_RAW | IF_TYPE_PROP_VIRTUAL_RAW
    );
    let tunnel_medium = raw.tunnel_type != 0 || raw.media_type == NDIS_MEDIUM_TUNNEL_RAW;
    // NDIS medium IP has no physical link-layer framing. Combined with an unspecified physical
    // medium this is structural evidence for a software-created IP interface (for example Wintun
    // or Meta), independent of its user-changeable alias/friendly name.
    let software_ip = raw.media_type == NDIS_MEDIUM_IP_RAW
        && raw.physical_medium_type == NDIS_PHYSICAL_MEDIUM_UNSPECIFIED_RAW
        && matches!(
            raw.access_type,
            NET_IF_ACCESS_BROADCAST_RAW | NET_IF_ACCESS_POINT_TO_POINT_RAW
        );
    if typed_virtual || tunnel_medium || software_ip {
        DefaultRouteOwner::VpnOrTun
    } else {
        DefaultRouteOwner::Other
    }
}

#[derive(Clone, Debug)]
pub struct ProbePolicy {
    pub dns_timeout: Duration,
    pub connect_timeout: Duration,
    pub tls_timeout: Duration,
    pub read_timeout: Duration,
    pub total_timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for ProbePolicy {
    fn default() -> Self {
        Self {
            dns_timeout: Duration::from_secs(3),
            connect_timeout: Duration::from_secs(3),
            tls_timeout: Duration::from_secs(3),
            read_timeout: Duration::from_secs(3),
            total_timeout: Duration::from_secs(10),
            max_response_bytes: 4096,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeStage<T> {
    Succeeded(T),
    Failed {
        code: &'static str,
        os_code: Option<u32>,
    },
    Unavailable {
        code: &'static str,
    },
    Unexecuted {
        code: &'static str,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimedStage<T> {
    pub outcome: ProbeStage<T>,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundRouteEvidence {
    pub luid: u64,
    pub source: IpAddr,
    pub route: RouteObservation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectEvidence {
    pub actual_source: IpAddr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsEvidence {
    pub validated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpEvidence {
    pub status: u16,
    pub response_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointAttempt {
    pub endpoint_id: &'static str,
    pub family: AddressFamily,
    pub destination: SocketAddr,
    pub route: TimedStage<BoundRouteEvidence>,
    pub connect: TimedStage<ConnectEvidence>,
    pub tls: TimedStage<TlsEvidence>,
    pub http: TimedStage<HttpEvidence>,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FamilyProbeResult {
    pub family: AddressFamily,
    pub interface_index: u32,
    pub selected_source: Option<IpAddr>,
    pub dns: TimedStage<Vec<IpAddr>>,
    pub endpoints: Vec<EndpointAttempt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalRouteEvidence {
    pub global_luid: Option<u64>,
    pub interface_index: Option<u32>,
    pub owner: DefaultRouteOwner,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalRouteComparison {
    pub family: AddressFamily,
    pub target_luid: u64,
    pub route: TimedStage<GlobalRouteEvidence>,
    pub explanation_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundProbeResult {
    pub epoch: DeviceEpoch,
    pub adapter_guid: String,
    pub chosen_family: Option<AddressFamily>,
    pub families: Vec<FamilyProbeResult>,
    pub global_routes: Vec<GlobalRouteComparison>,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub elapsed: Duration,
}

#[derive(Clone, Copy, Debug)]
struct Endpoint {
    id: &'static str,
    destination: SocketAddr,
    tls_name: &'static str,
}

/// Reachability endpoints, tried in order. Mainland-carrier networks routinely blackhole the
/// Cloudflare/Google anycast addresses while domestic anycast DNS endpoints answer in
/// milliseconds, so the domestic endpoints lead; they are globally anycast and remain valid
/// targets outside the mainland as well. Pass requires one endpoint with a validated TLS
/// handshake and any parseable HTTP response.
const V4_ENDPOINTS: [Endpoint; 4] = [
    Endpoint {
        id: "alidns-v4",
        destination: SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::new(223, 5, 5, 5)), 443),
        tls_name: "dns.alidns.com",
    },
    Endpoint {
        id: "dnspod-v4",
        destination: SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::new(1, 12, 12, 21)), 443),
        tls_name: "doh.pub",
    },
    Endpoint {
        id: "cloudflare-v4",
        destination: SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1)), 443),
        tls_name: "one.one.one.one",
    },
    Endpoint {
        id: "google-v4",
        destination: SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::new(8, 8, 8, 8)), 443),
        tls_name: "dns.google",
    },
];
const V6_ENDPOINTS: [Endpoint; 3] = [
    Endpoint {
        id: "alidns-v6",
        destination: SocketAddr::new(
            IpAddr::V6(std::net::Ipv6Addr::new(0x2400, 0x3200, 0, 0, 0, 0, 0, 0x1)),
            443,
        ),
        tls_name: "dns.alidns.com",
    },
    Endpoint {
        id: "cloudflare-v6",
        destination: SocketAddr::new(
            IpAddr::V6(std::net::Ipv6Addr::new(
                0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111,
            )),
            443,
        ),
        tls_name: "one.one.one.one",
    },
    Endpoint {
        id: "google-v6",
        destination: SocketAddr::new(
            IpAddr::V6(std::net::Ipv6Addr::new(
                0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888,
            )),
            443,
        ),
        tls_name: "dns.google",
    },
];

#[derive(Clone, Copy, Debug)]
struct BestRouteRequest {
    luid: u64,
    interface_index: u32,
    source: IpAddr,
    destination: SocketAddr,
}
#[derive(Clone, Copy, Debug)]
struct BestRouteReply {
    luid: u64,
    interface_index: u32,
    source: IpAddr,
    next_hop: IpAddr,
    route_metric: u32,
}
#[derive(Clone, Copy, Debug)]
struct GlobalRouteReply {
    luid: u64,
    interface_index: u32,
    owner: DefaultRouteOwner,
}
#[derive(Clone, Debug)]
struct BoundDnsRequest {
    family: AddressFamily,
    interface_index: u32,
    timeout: Duration,
    deadline: Instant,
}
#[derive(Clone, Debug)]
struct BoundHttpsRequest {
    family: AddressFamily,
    luid: u64,
    interface_index: u32,
    socket_interface_value: u32,
    source: IpAddr,
    destination: SocketAddr,
    tls_name: &'static str,
    connect_timeout: Duration,
    tls_timeout: Duration,
    read_timeout: Duration,
    deadline: Instant,
    max_response_bytes: usize,
}
#[derive(Clone, Copy, Debug)]
struct SocketEvidence {
    actual_source: IpAddr,
    tls_validated: bool,
    http_status: Option<u16>,
    response_bytes: usize,
}

trait ProbeBackend {
    fn best_route(&self, request: BestRouteRequest) -> Result<BestRouteReply, PlatformError>;
    fn dns(&self, request: BoundDnsRequest) -> Result<Vec<IpAddr>, PlatformError>;
    fn https(&self, request: BoundHttpsRequest) -> Result<SocketEvidence, PlatformError>;
    fn global_route(
        &self,
        family: AddressFamily,
        destination: SocketAddr,
    ) -> Result<GlobalRouteReply, PlatformError>;
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

fn validate_policy(policy: &ProbePolicy) -> Result<(), PlatformError> {
    let stages = [
        policy.dns_timeout,
        policy.connect_timeout,
        policy.tls_timeout,
        policy.read_timeout,
    ];
    if stages
        .into_iter()
        .any(|d| d.is_zero() || d > MAX_STAGE_TIMEOUT)
        || policy.total_timeout.is_zero()
        || policy.total_timeout > MAX_TOTAL_TIMEOUT
        || policy.max_response_bytes == 0
        || policy.max_response_bytes > MAX_RESPONSE_BYTES
    {
        return Err(PlatformError {
            code: "probe:policy_invalid",
            os_code: None,
        });
    }
    Ok(())
}

fn observe_now<B: ProbeBackend>(
    backend: &B,
    target: &AdapterIdentity,
    policy: &ProbePolicy,
) -> Result<BoundProbeResult, PlatformError> {
    validate_policy(policy)?;
    let started_at = backend.now();
    let started_mono = Instant::now();
    let mut families = Vec::new();
    let mut globals = Vec::new();
    for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
        // Each family gets the full budget: a blackholed IPv4 endpoint must not starve the
        // IPv6 family, which on carrier networks is often the only globally routed one.
        let deadline = started_mono + policy.total_timeout;
        let interface_index = match family {
            AddressFamily::Ipv4 => target.ipv4_index(),
            AddressFamily::Ipv6 => target.ipv6_index(),
        };
        let Some(interface_index) = interface_index else {
            continue;
        };
        if interface_index == 0 {
            return Err(PlatformError {
                code: "probe:route_identity_mismatch",
                os_code: None,
            });
        }
        let source = target
            .unicast_addresses()
            .iter()
            .copied()
            .find(|ip| is_usable_source(*ip, family));
        let dns_started_at = backend.now();
        let dns_started_mono = Instant::now();
        let dns = remaining(deadline, policy.dns_timeout).map_or_else(
            || {
                timed_stage(
                    backend,
                    dns_started_at,
                    dns_started_mono,
                    ProbeStage::Unexecuted {
                        code: "probe:total_timeout",
                    },
                )
            },
            |timeout| {
                let outcome = backend
                    .dns(BoundDnsRequest {
                        family,
                        interface_index,
                        timeout,
                        deadline,
                    })
                    .map(ProbeStage::Succeeded)
                    .unwrap_or_else(error_stage);
                timed_stage(backend, dns_started_at, dns_started_mono, outcome)
            },
        );
        let endpoints: &[Endpoint] = match family {
            AddressFamily::Ipv4 => &V4_ENDPOINTS,
            AddressFamily::Ipv6 => &V6_ENDPOINTS,
        };
        let mut endpoint_results = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            endpoint_results.push(observe_endpoint(
                backend,
                target,
                policy,
                deadline,
                family,
                interface_index,
                source,
                *endpoint,
            ));
        }
        let global_started_at = backend.now();
        let global_started_mono = Instant::now();
        let global_outcome = if remaining(deadline, policy.total_timeout).is_none() {
            ProbeStage::Unexecuted {
                code: "probe:total_timeout",
            }
        } else {
            backend
                .global_route(family, endpoints[0].destination)
                .map(|reply| {
                    let owner = if reply.luid == target.luid()
                        && reply.interface_index == interface_index
                    {
                        DefaultRouteOwner::TargetAdapter
                    } else {
                        reply.owner
                    };
                    ProbeStage::Succeeded(GlobalRouteEvidence {
                        global_luid: Some(reply.luid),
                        interface_index: Some(reply.interface_index),
                        owner,
                    })
                })
                .unwrap_or_else(error_stage)
        };
        globals.push(GlobalRouteComparison {
            family,
            target_luid: target.luid(),
            route: timed_stage(
                backend,
                global_started_at,
                global_started_mono,
                global_outcome,
            ),
            explanation_only: true,
        });
        families.push(FamilyProbeResult {
            family,
            interface_index,
            selected_source: source,
            dns,
            endpoints: endpoint_results,
        });
    }
    let chosen_family = families
        .iter()
        .find(|family| {
            family.endpoints.iter().any(|attempt| {
                matches!(attempt.route.outcome, ProbeStage::Succeeded(_))
                    && matches!(attempt.connect.outcome, ProbeStage::Succeeded(_))
                    && matches!(
                        attempt.tls.outcome,
                        ProbeStage::Succeeded(TlsEvidence { validated: true })
                    )
                    && matches!(attempt.http.outcome, ProbeStage::Succeeded(_))
            })
        })
        .map(|f| f.family);
    Ok(BoundProbeResult {
        epoch: target.epoch(),
        adapter_guid: target.guid_string(),
        chosen_family,
        families,
        global_routes: globals,
        started_at,
        finished_at: backend.now(),
        elapsed: started_mono.elapsed(),
    })
}

#[allow(clippy::too_many_arguments)]
fn observe_endpoint<B: ProbeBackend>(
    backend: &B,
    target: &AdapterIdentity,
    policy: &ProbePolicy,
    deadline: Instant,
    family: AddressFamily,
    interface_index: u32,
    source: Option<IpAddr>,
    endpoint: Endpoint,
) -> EndpointAttempt {
    let started_at = backend.now();
    let started_mono = Instant::now();
    let Some(total_timeout) = remaining(deadline, policy.total_timeout) else {
        return EndpointAttempt {
            endpoint_id: endpoint.id,
            family,
            destination: endpoint.destination,
            route: empty_timed(backend, started_at, "probe:total_timeout"),
            connect: empty_timed(backend, started_at, "probe:total_timeout"),
            tls: empty_timed(backend, started_at, "probe:total_timeout"),
            http: empty_timed(backend, started_at, "probe:total_timeout"),
            started_at,
            finished_at: backend.now(),
            elapsed: started_mono.elapsed(),
        };
    };
    let Some(source) = source else {
        let route = TimedStage {
            outcome: ProbeStage::Unavailable {
                code: "net:no_usable_address",
            },
            started_at,
            finished_at: backend.now(),
            elapsed: started_mono.elapsed(),
        };
        return EndpointAttempt {
            endpoint_id: endpoint.id,
            family,
            destination: endpoint.destination,
            route,
            connect: empty_timed(backend, started_at, "probe:dependency_unavailable"),
            tls: empty_timed(backend, started_at, "probe:dependency_unavailable"),
            http: empty_timed(backend, started_at, "probe:dependency_unavailable"),
            started_at,
            finished_at: backend.now(),
            elapsed: started_mono.elapsed(),
        };
    };

    let route_started_at = backend.now();
    let route_started_mono = Instant::now();
    let route_result = backend.best_route(BestRouteRequest {
        luid: target.luid(),
        interface_index,
        source,
        destination: endpoint.destination,
    });
    let route_outcome = match route_result {
        Ok(route)
            if route.luid == target.luid()
                && route.interface_index == interface_index
                && route.source == source
                && target.unicast_addresses().contains(&route.source) =>
        {
            ProbeStage::Succeeded(BoundRouteEvidence {
                luid: route.luid,
                source: route.source,
                route: RouteObservation {
                    family,
                    interface_index,
                    destination: endpoint.destination.ip(),
                    prefix_len: match family {
                        AddressFamily::Ipv4 => 32,
                        AddressFamily::Ipv6 => 128,
                    },
                    next_hop: route.next_hop,
                    route_metric: route.route_metric,
                    total_metric: route.route_metric,
                },
            })
        }
        Ok(_) => ProbeStage::Failed {
            code: "probe:route_identity_mismatch",
            os_code: None,
        },
        Err(error) => error_stage(error),
    };
    let route = timed_stage(backend, route_started_at, route_started_mono, route_outcome);
    if !matches!(route.outcome, ProbeStage::Succeeded(_)) {
        return EndpointAttempt {
            endpoint_id: endpoint.id,
            family,
            destination: endpoint.destination,
            route,
            connect: empty_timed(backend, started_at, "probe:dependency_unavailable"),
            tls: empty_timed(backend, started_at, "probe:dependency_unavailable"),
            http: empty_timed(backend, started_at, "probe:dependency_unavailable"),
            started_at,
            finished_at: backend.now(),
            elapsed: started_mono.elapsed(),
        };
    }

    let socket_interface_value = match family {
        AddressFamily::Ipv4 => interface_index.to_be(),
        AddressFamily::Ipv6 => interface_index,
    };
    let transport_started_at = backend.now();
    let transport_started_mono = Instant::now();
    let result = backend.https(BoundHttpsRequest {
        family,
        luid: target.luid(),
        interface_index,
        socket_interface_value,
        source,
        destination: endpoint.destination,
        tls_name: endpoint.tls_name,
        connect_timeout: policy.connect_timeout.min(total_timeout),
        tls_timeout: policy.tls_timeout.min(total_timeout),
        read_timeout: policy.read_timeout.min(total_timeout),
        deadline,
        max_response_bytes: policy.max_response_bytes,
    });
    let (connect, tls, http) = transport_stages(
        backend,
        transport_started_at,
        transport_started_mono,
        source,
        target,
        policy,
        result,
    );
    EndpointAttempt {
        endpoint_id: endpoint.id,
        family,
        destination: endpoint.destination,
        route,
        connect,
        tls,
        http,
        started_at,
        finished_at: backend.now(),
        elapsed: started_mono.elapsed(),
    }
}

#[allow(clippy::type_complexity)]
fn transport_stages<B: ProbeBackend>(
    backend: &B,
    started_at: SystemTime,
    started_mono: Instant,
    expected_source: IpAddr,
    target: &AdapterIdentity,
    policy: &ProbePolicy,
    result: Result<SocketEvidence, PlatformError>,
) -> (
    TimedStage<ConnectEvidence>,
    TimedStage<TlsEvidence>,
    TimedStage<HttpEvidence>,
) {
    let elapsed = started_mono.elapsed();
    let finished_at = backend.now();
    match result {
        Ok(socket)
            if socket.actual_source != expected_source
                || !target.unicast_addresses().contains(&socket.actual_source) =>
        {
            (
                fixed_timed_stage(
                    started_at,
                    finished_at,
                    elapsed,
                    ProbeStage::Failed {
                        code: "probe:source_mismatch",
                        os_code: None,
                    },
                ),
                fixed_timed_stage(
                    started_at,
                    finished_at,
                    elapsed,
                    ProbeStage::Unexecuted {
                        code: "probe:dependency_unavailable",
                    },
                ),
                fixed_timed_stage(
                    started_at,
                    finished_at,
                    elapsed,
                    ProbeStage::Unexecuted {
                        code: "probe:dependency_unavailable",
                    },
                ),
            )
        }
        Ok(socket) if !socket.tls_validated => (
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(ConnectEvidence {
                    actual_source: socket.actual_source,
                }),
            ),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Failed {
                    code: "probe:tls_failed",
                    os_code: None,
                },
            ),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
            ),
        ),
        Ok(socket)
            if socket.http_status.is_none()
                || socket.response_bytes > policy.max_response_bytes =>
        {
            (
                fixed_timed_stage(
                    started_at,
                    finished_at,
                    elapsed,
                    ProbeStage::Succeeded(ConnectEvidence {
                        actual_source: socket.actual_source,
                    }),
                ),
                fixed_timed_stage(
                    started_at,
                    finished_at,
                    elapsed,
                    ProbeStage::Succeeded(TlsEvidence { validated: true }),
                ),
                fixed_timed_stage(
                    started_at,
                    finished_at,
                    elapsed,
                    ProbeStage::Failed {
                        code: if socket.response_bytes > policy.max_response_bytes {
                            "probe:http_too_large"
                        } else {
                            "probe:http_invalid"
                        },
                        os_code: None,
                    },
                ),
            )
        }
        Ok(socket) => (
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(ConnectEvidence {
                    actual_source: socket.actual_source,
                }),
            ),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(TlsEvidence { validated: true }),
            ),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(HttpEvidence {
                    status: socket.http_status.expect("validated above"),
                    response_bytes: socket.response_bytes,
                }),
            ),
        ),
        Err(error) if error.code.starts_with("probe:tls_") => (
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(ConnectEvidence {
                    actual_source: expected_source,
                }),
            ),
            fixed_timed_stage(started_at, finished_at, elapsed, error_stage(error)),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
            ),
        ),
        Err(error) if error.code.starts_with("probe:http_") => (
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(ConnectEvidence {
                    actual_source: expected_source,
                }),
            ),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Succeeded(TlsEvidence { validated: true }),
            ),
            fixed_timed_stage(started_at, finished_at, elapsed, error_stage(error)),
        ),
        Err(error) => (
            fixed_timed_stage(started_at, finished_at, elapsed, error_stage(error)),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
            ),
            fixed_timed_stage(
                started_at,
                finished_at,
                elapsed,
                ProbeStage::Unexecuted {
                    code: "probe:dependency_unavailable",
                },
            ),
        ),
    }
}

fn fixed_timed_stage<T>(
    started_at: SystemTime,
    finished_at: SystemTime,
    elapsed: Duration,
    outcome: ProbeStage<T>,
) -> TimedStage<T> {
    if matches!(outcome, ProbeStage::Unexecuted { .. }) {
        return TimedStage {
            outcome,
            started_at: finished_at,
            finished_at,
            elapsed: Duration::ZERO,
        };
    }
    TimedStage {
        outcome,
        started_at,
        finished_at,
        elapsed,
    }
}

fn timed_stage<T, B: ProbeBackend>(
    backend: &B,
    started_at: SystemTime,
    started_mono: Instant,
    outcome: ProbeStage<T>,
) -> TimedStage<T> {
    TimedStage {
        outcome,
        started_at,
        finished_at: backend.now(),
        elapsed: started_mono.elapsed(),
    }
}

fn empty_timed<T, B: ProbeBackend>(
    backend: &B,
    started_at: SystemTime,
    code: &'static str,
) -> TimedStage<T> {
    TimedStage {
        outcome: ProbeStage::Unexecuted { code },
        started_at,
        finished_at: backend.now(),
        elapsed: Duration::ZERO,
    }
}

fn error_stage<T>(error: PlatformError) -> ProbeStage<T> {
    ProbeStage::Failed {
        code: error.code,
        os_code: error.os_code,
    }
}
fn remaining(deadline: Instant, cap: Duration) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .map(|value| value.min(cap))
}

#[derive(Clone, Copy, Debug)]
struct MonotonicDeadline {
    hard: Instant,
}

impl MonotonicDeadline {
    #[cfg(test)]
    fn with_budget(budget: Duration) -> Self {
        Self {
            hard: Instant::now() + budget,
        }
    }

    fn remaining(self, stage_started: Instant, stage_cap: Duration) -> Option<Duration> {
        let now = Instant::now();
        let total = self.hard.checked_duration_since(now)?;
        let stage_end = stage_started + stage_cap;
        let stage = stage_end.checked_duration_since(now)?;
        let value = total.min(stage);
        (!value.is_zero()).then_some(value)
    }
}

trait DeadlineRead: Read {
    fn set_deadline_read_timeout(&mut self, timeout: Duration) -> std::io::Result<()>;
}

fn read_http_headers_bounded<R: DeadlineRead>(
    reader: &mut R,
    deadline: MonotonicDeadline,
    stage_cap: Duration,
    max_response_bytes: usize,
) -> Result<Vec<u8>, PlatformError> {
    let stage_started = Instant::now();
    let mut response = Vec::with_capacity(max_response_bytes.min(512));
    loop {
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(response);
        }
        let remaining_capacity = max_response_bytes.saturating_sub(response.len());
        if remaining_capacity == 0 {
            return Err(PlatformError {
                code: "probe:http_too_large",
                os_code: None,
            });
        }
        let timeout = deadline
            .remaining(stage_started, stage_cap)
            .ok_or(PlatformError {
                code: "probe:http_timeout",
                os_code: None,
            })?;
        reader
            .set_deadline_read_timeout(timeout)
            .map_err(|error| io_platform_error("probe:http_timeout", &error))?;
        let mut chunk = [0_u8; 512];
        let requested = chunk.len().min(remaining_capacity);
        match reader.read(&mut chunk[..requested]) {
            Ok(0) => return Ok(response),
            Ok(count) => {
                response.extend_from_slice(&chunk[..count]);
                if deadline.remaining(stage_started, stage_cap).is_none()
                    && !response.windows(4).any(|window| window == b"\r\n\r\n")
                {
                    return Err(PlatformError {
                        code: "probe:http_timeout",
                        os_code: None,
                    });
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                if deadline.remaining(stage_started, stage_cap).is_none() {
                    return Err(io_platform_error("probe:http_timeout", &error));
                }
            }
            Err(error) => return Err(io_platform_error("probe:http_invalid", &error)),
        }
    }
}

fn io_platform_error(code: &'static str, error: &std::io::Error) -> PlatformError {
    PlatformError {
        code,
        os_code: error
            .raw_os_error()
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn classify_tls_io_error(error: &std::io::Error) -> PlatformError {
    io_platform_error(
        if matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ) {
            "probe:tls_timeout"
        } else {
            "probe:tls_failed"
        },
        error,
    )
}

#[cfg(test)]
#[derive(Default)]
struct DnsPendingGate {
    pending: Arc<Mutex<bool>>,
}

#[cfg(test)]
impl DnsPendingGate {
    fn try_begin(&self) -> Result<DnsPendingLease, PlatformError> {
        let mut pending = self.pending.lock().map_err(|_| PlatformError {
            code: "probe:dns_cancelled",
            os_code: None,
        })?;
        if *pending {
            return Err(PlatformError {
                code: "probe:dns_cancelled",
                os_code: None,
            });
        }
        *pending = true;
        Ok(DnsPendingLease {
            pending: Arc::clone(&self.pending),
            completed: false,
        })
    }
}

#[cfg(test)]
#[derive(Debug)]
struct DnsPendingLease {
    pending: Arc<Mutex<bool>>,
    completed: bool,
}

#[cfg(test)]
impl DnsPendingLease {
    fn complete(mut self) {
        self.release();
    }

    fn release(&mut self) {
        if !self.completed {
            if let Ok(mut pending) = self.pending.lock() {
                *pending = false;
            }
            self.completed = true;
        }
    }
}

#[cfg(test)]
impl Drop for DnsPendingLease {
    fn drop(&mut self) {
        // A production DNS worker deliberately forgets this lease after caller timeout; the
        // delayed completion callback path owns and eventually drops it. Ordinary paths release.
        self.release();
    }
}

#[cfg(test)]
fn finish_dns_completion<R>(records: R, status: i32) -> Result<R, PlatformError> {
    if status != 0 {
        drop(records);
        return Err(PlatformError {
            code: "probe:dns_failed",
            os_code: u32::try_from(status).ok(),
        });
    }
    Ok(records)
}

#[cfg(test)]
type TestDeadline = MonotonicDeadline;

#[cfg(test)]
struct SlowFragmentReader {
    fragments: std::collections::VecDeque<Vec<u8>>,
    delay: Duration,
    bytes_returned: usize,
    max_requested: usize,
    timeout: Option<Duration>,
}

#[cfg(test)]
impl SlowFragmentReader {
    fn new(fragments: Vec<Vec<u8>>) -> Self {
        Self::with_delay(fragments, Duration::ZERO)
    }
    fn with_delay(fragments: Vec<Vec<u8>>, delay: Duration) -> Self {
        Self {
            fragments: fragments.into(),
            delay,
            bytes_returned: 0,
            max_requested: 0,
            timeout: None,
        }
    }
    fn bytes_returned(&self) -> usize {
        self.bytes_returned
    }
    fn max_requested(&self) -> usize {
        self.max_requested
    }
}

#[cfg(test)]
impl Read for SlowFragmentReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.max_requested = self.max_requested.max(buffer.len());
        if !self.delay.is_zero() {
            if self.timeout.is_some_and(|timeout| timeout < self.delay) {
                std::thread::sleep(self.timeout.expect("checked above"));
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "deadline",
                ));
            }
            std::thread::sleep(self.delay);
        }
        let Some(mut fragment) = self.fragments.pop_front() else {
            return Ok(0);
        };
        let count = fragment.len().min(buffer.len());
        buffer[..count].copy_from_slice(&fragment[..count]);
        if count < fragment.len() {
            fragment.drain(..count);
            self.fragments.push_front(fragment);
        }
        self.bytes_returned += count;
        Ok(count)
    }
}

#[cfg(test)]
impl DeadlineRead for SlowFragmentReader {
    fn set_deadline_read_timeout(&mut self, timeout: Duration) -> std::io::Result<()> {
        self.timeout = Some(timeout);
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug)]
struct DropSpy(Arc<std::sync::atomic::AtomicUsize>);

#[cfg(test)]
impl DropSpy {
    fn new(counter: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        Self(counter)
    }
}

#[cfg(test)]
impl Drop for DropSpy {
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[allow(async_fn_in_trait)]
pub trait NetworkProbe {
    async fn observe(
        &self,
        target: &AdapterIdentity,
        policy: &ProbePolicy,
    ) -> Result<BoundProbeResult, PlatformError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsNetworkProbe;

impl NetworkProbe for WindowsNetworkProbe {
    async fn observe(
        &self,
        target: &AdapterIdentity,
        policy: &ProbePolicy,
    ) -> Result<BoundProbeResult, PlatformError> {
        #[cfg(windows)]
        {
            observe_now(&native::NativeProbeBackend, target, policy)
        }
        #[cfg(not(windows))]
        {
            let _ = (target, policy);
            Err(PlatformError {
                code: "net:unsupported_platform",
                os_code: None,
            })
        }
    }
}

impl WindowsNetworkProbe {
    pub fn observe_now(
        &self,
        target: &AdapterIdentity,
        policy: &ProbePolicy,
    ) -> Result<BoundProbeResult, PlatformError> {
        #[cfg(windows)]
        {
            observe_now(&native::NativeProbeBackend, target, policy)
        }
        #[cfg(not(windows))]
        {
            let _ = (target, policy);
            Err(PlatformError {
                code: "net:unsupported_platform",
                os_code: None,
            })
        }
    }
}

#[cfg(test)]
struct FakeProbeBackend {
    best_routes: Vec<Result<BestRouteReply, PlatformError>>,
    dns: Vec<Result<Vec<IpAddr>, PlatformError>>,
    https: Vec<Result<SocketEvidence, PlatformError>>,
    global_routes: Vec<Result<GlobalRouteReply, PlatformError>>,
    now: SystemTime,
    best_cursor: std::sync::atomic::AtomicUsize,
    dns_cursor: std::sync::atomic::AtomicUsize,
    https_cursor: std::sync::atomic::AtomicUsize,
    global_cursor: std::sync::atomic::AtomicUsize,
    best_seen: std::sync::Mutex<Vec<BestRouteRequest>>,
    dns_seen: std::sync::Mutex<Vec<BoundDnsRequest>>,
    https_seen: std::sync::Mutex<Vec<BoundHttpsRequest>>,
}

#[cfg(test)]
impl Default for FakeProbeBackend {
    fn default() -> Self {
        Self {
            best_routes: Vec::new(),
            dns: Vec::new(),
            https: Vec::new(),
            global_routes: Vec::new(),
            now: SystemTime::UNIX_EPOCH,
            best_cursor: Default::default(),
            dns_cursor: Default::default(),
            https_cursor: Default::default(),
            global_cursor: Default::default(),
            best_seen: Default::default(),
            dns_seen: Default::default(),
            https_seen: Default::default(),
        }
    }
}

#[cfg(test)]
impl FakeProbeBackend {
    fn item<T: Clone>(
        items: &[Result<T, PlatformError>],
        cursor: &std::sync::atomic::AtomicUsize,
        default: &'static str,
    ) -> Result<T, PlatformError> {
        let i = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        items.get(i).cloned().unwrap_or(Err(PlatformError {
            code: default,
            os_code: None,
        }))
    }
    fn best_route_requests(&self) -> Vec<BestRouteRequest> {
        self.best_seen.lock().unwrap().clone()
    }
    fn dns_requests(&self) -> Vec<BoundDnsRequest> {
        self.dns_seen.lock().unwrap().clone()
    }
    fn https_requests(&self) -> Vec<BoundHttpsRequest> {
        self.https_seen.lock().unwrap().clone()
    }
}
#[cfg(test)]
impl ProbeBackend for FakeProbeBackend {
    fn best_route(&self, r: BestRouteRequest) -> Result<BestRouteReply, PlatformError> {
        self.best_seen.lock().unwrap().push(r);
        Self::item(
            &self.best_routes,
            &self.best_cursor,
            "probe:route_unavailable",
        )
    }
    fn dns(&self, r: BoundDnsRequest) -> Result<Vec<IpAddr>, PlatformError> {
        self.dns_seen.lock().unwrap().push(r);
        Self::item(&self.dns, &self.dns_cursor, "probe:dns_failed")
    }
    fn https(&self, r: BoundHttpsRequest) -> Result<SocketEvidence, PlatformError> {
        self.https_seen.lock().unwrap().push(r);
        Self::item(&self.https, &self.https_cursor, "probe:connect_failed")
    }
    fn global_route(
        &self,
        _: AddressFamily,
        _: SocketAddr,
    ) -> Result<GlobalRouteReply, PlatformError> {
        Self::item(
            &self.global_routes,
            &self.global_cursor,
            "probe:route_unavailable",
        )
    }
    fn now(&self) -> SystemTime {
        self.now
    }
}

#[cfg(windows)]
mod native {
    use std::{
        ffi::c_void,
        io::Write,
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
        os::windows::io::AsRawSocket,
        ptr::{null, null_mut},
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
    };

    use native_tls::TlsConnector;
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};
    use windows_sys::Win32::{
        Foundation::NO_ERROR,
        NetworkManagement::{
            Dns::{
                DNS_QUERY_BYPASS_CACHE, DNS_QUERY_CANCEL, DNS_QUERY_NO_HOSTS_FILE,
                DNS_QUERY_NO_MULTICAST, DNS_QUERY_NO_NETBT, DNS_QUERY_REQUEST,
                DNS_QUERY_REQUEST_VERSION1, DNS_QUERY_RESULT, DNS_QUERY_RESULTS_VERSION1,
                DNS_RECORDA, DNS_TYPE_A, DNS_TYPE_AAAA, DnsCancelQuery, DnsQueryEx,
            },
            IpHelper::{GetBestRoute2, GetIfEntry2, MIB_IF_ROW2, MIB_IPFORWARD_ROW2},
            Ndis::NET_LUID_LH,
        },
        Networking::WinSock::{
            AF_INET, AF_INET6, IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF,
            SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_INET, setsockopt,
        },
    };

    use super::*;

    const DNS_REQUEST_PENDING: i32 = 9506;
    static DNS_NATIVE_PENDING: AtomicBool = AtomicBool::new(false);
    pub(super) struct NativeProbeBackend;

    impl ProbeBackend for NativeProbeBackend {
        fn best_route(&self, request: BestRouteRequest) -> Result<BestRouteReply, PlatformError> {
            let luid = NET_LUID_LH {
                Value: request.luid,
            };
            let source = sockaddr_inet(SocketAddr::new(request.source, 0));
            let destination = sockaddr_inet(request.destination);
            let mut row = MIB_IPFORWARD_ROW2::default();
            let mut best_source = SOCKADDR_INET::default();
            // SAFETY: all input structures are initialized for their address families and both
            // output structures are writable for the entire call.
            let status = unsafe {
                GetBestRoute2(
                    &luid,
                    request.interface_index,
                    &source,
                    &destination,
                    0,
                    &mut row,
                    &mut best_source,
                )
            };
            if status != NO_ERROR {
                return Err(os_error("probe:route_unavailable", status));
            }
            Ok(BestRouteReply {
                // SAFETY: the API initialized the returned LUID scalar and source union.
                luid: unsafe { row.InterfaceLuid.Value },
                interface_index: row.InterfaceIndex,
                source: ip_from_inet(&best_source).ok_or(PlatformError {
                    code: "probe:route_identity_mismatch",
                    os_code: None,
                })?,
                next_hop: ip_from_inet(&row.NextHop).ok_or(PlatformError {
                    code: "probe:route_identity_mismatch",
                    os_code: None,
                })?,
                route_metric: row.Metric,
            })
        }

        fn dns(&self, request: BoundDnsRequest) -> Result<Vec<IpAddr>, PlatformError> {
            if request.interface_index == 0 {
                return Err(PlatformError {
                    code: "probe:dns_api_failed",
                    os_code: None,
                });
            }
            let hard_remaining = request
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or(PlatformError {
                    code: "probe:dns_timeout",
                    os_code: None,
                })?;
            let query_timeout = request.timeout.min(hard_remaining);
            let lease = NativeDnsLease::acquire()?;
            let shared = Arc::new(NativeDnsShared::default());
            let worker_shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("dji4g-bound-dns".to_owned())
                .spawn(move || dns_worker(request, worker_shared, lease))
                .map_err(|_| PlatformError {
                    code: "probe:dns_api_failed",
                    os_code: None,
                })?;
            let mut result = shared.result.lock().map_err(|_| PlatformError {
                code: "probe:dns_cancelled",
                os_code: None,
            })?;
            let wait_started = Instant::now();
            while result.is_none() {
                let Some(remaining) = query_timeout.checked_sub(wait_started.elapsed()) else {
                    break;
                };
                if remaining.is_zero() {
                    break;
                }
                let waited =
                    shared
                        .ready
                        .wait_timeout(result, remaining)
                        .map_err(|_| PlatformError {
                            code: "probe:dns_cancelled",
                            os_code: None,
                        })?;
                result = waited.0;
                if waited.1.timed_out() {
                    break;
                }
            }
            if let Some(result) = result.clone() {
                return result;
            }
            drop(result);
            let cancel_status = {
                let cancel = shared.cancel.lock().map_err(|_| PlatformError {
                    code: "probe:dns_cancelled",
                    os_code: None,
                })?;
                cancel.map(|pointer| {
                    // SAFETY: the worker clears this pointer under the same mutex before dropping
                    // PendingDns. Holding the mutex therefore keeps the pointed-to cancel handle live.
                    unsafe { DnsCancelQuery(pointer as *const DNS_QUERY_CANCEL) }
                })
            };
            Err(PlatformError {
                code: if cancel_status.is_none_or(|status| status == 0) {
                    "probe:dns_timeout"
                } else {
                    "probe:dns_cancelled"
                },
                os_code: cancel_status.and_then(|status| u32::try_from(status).ok()),
            })
        }

        fn https(&self, request: BoundHttpsRequest) -> Result<SocketEvidence, PlatformError> {
            let deadline = MonotonicDeadline {
                hard: request.deadline,
            };
            revalidate_interface(request.luid, request.interface_index)?;
            let domain = match request.family {
                AddressFamily::Ipv4 => Domain::IPV4,
                AddressFamily::Ipv6 => Domain::IPV6,
            };
            let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
                .map_err(|e| io_error("probe:connect_failed", &e))?;
            let raw = socket.as_raw_socket();
            let value = request.socket_interface_value;
            let (level, option) = match request.family {
                AddressFamily::Ipv4 => (IPPROTO_IP, IP_UNICAST_IF),
                AddressFamily::Ipv6 => (IPPROTO_IPV6, IPV6_UNICAST_IF),
            };
            // SAFETY: raw is a live socket; value points to exactly one u32 for the documented
            // family-specific interface option (IPv4 network order, IPv6 host order).
            let status = unsafe {
                setsockopt(
                    raw as usize,
                    level,
                    option,
                    (&value as *const u32).cast(),
                    size_of::<u32>() as i32,
                )
            };
            if status != 0 {
                return Err(last_socket_error("probe:socket_option_failed"));
            }
            socket
                .bind(&SockAddr::from(SocketAddr::new(request.source, 0)))
                .map_err(|e| io_error("probe:bind_failed", &e))?;
            let connect_started = Instant::now();
            let connect_timeout = deadline
                .remaining(connect_started, request.connect_timeout)
                .and_then(|remaining| remaining.checked_sub(BLOCKING_TIMER_MARGIN))
                .ok_or(PlatformError {
                    code: "probe:connect_timeout",
                    os_code: None,
                })?;
            socket
                .connect_timeout(&SockAddr::from(request.destination), connect_timeout)
                .map_err(|e| {
                    io_error(
                        if e.kind() == std::io::ErrorKind::TimedOut {
                            "probe:connect_timeout"
                        } else {
                            "probe:connect_failed"
                        },
                        &e,
                    )
                })?;
            let stream: TcpStream = socket.into();
            let actual_source = stream
                .local_addr()
                .map_err(|e| io_error("probe:source_mismatch", &e))?
                .ip();
            if actual_source != request.source {
                return Err(PlatformError {
                    code: "probe:source_mismatch",
                    os_code: None,
                });
            }
            stream
                .set_nonblocking(true)
                .map_err(|error| classify_tls_io_error(&error))?;
            let connector = TlsConnector::builder().build().map_err(|_| PlatformError {
                code: "probe:tls_failed",
                os_code: None,
            })?;
            let tls_started = Instant::now();
            let mut tls = match connector.connect(request.tls_name, stream) {
                Ok(stream) => stream,
                Err(native_tls::HandshakeError::Failure(error)) => {
                    return Err(classify_native_tls_error(
                        &error,
                        deadline
                            .remaining(tls_started, request.tls_timeout)
                            .is_none(),
                    ));
                }
                Err(native_tls::HandshakeError::WouldBlock(mut mid)) => loop {
                    let wait = deadline.remaining(tls_started, request.tls_timeout).ok_or(
                        PlatformError {
                            code: "probe:tls_timeout",
                            os_code: None,
                        },
                    )?;
                    match mid.handshake() {
                        Ok(stream) => break stream,
                        Err(native_tls::HandshakeError::WouldBlock(next)) => {
                            mid = next;
                            std::thread::sleep(wait.min(Duration::from_millis(2)));
                        }
                        Err(native_tls::HandshakeError::Failure(error)) => {
                            return Err(classify_native_tls_error(
                                &error,
                                deadline
                                    .remaining(tls_started, request.tls_timeout)
                                    .is_none(),
                            ));
                        }
                    }
                },
            };
            tls.get_ref()
                .set_nonblocking(false)
                .map_err(|error| classify_tls_io_error(&error))?;
            let request_text = format!(
                "HEAD / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: DJI4GPanel/0.1\r\n\r\n",
                request.tls_name
            );
            let http_started = Instant::now();
            write_http_request(
                &mut tls,
                request_text.as_bytes(),
                deadline,
                http_started,
                request.read_timeout,
            )?;
            let response = read_http_headers_bounded(
                &mut tls,
                deadline,
                request.read_timeout,
                request.max_response_bytes,
            )?;
            let line_end = response
                .windows(2)
                .position(|window| window == b"\r\n")
                .ok_or(PlatformError {
                    code: "probe:http_invalid",
                    os_code: None,
                })?;
            let line = std::str::from_utf8(&response[..line_end]).map_err(|_| PlatformError {
                code: "probe:http_invalid",
                os_code: None,
            })?;
            let mut parts = line.split_ascii_whitespace();
            if parts.next() != Some("HTTP/1.1") && !line.starts_with("HTTP/1.0 ") {
                return Err(PlatformError {
                    code: "probe:http_invalid",
                    os_code: None,
                });
            }
            let status = parts
                .next()
                .and_then(|part| part.parse::<u16>().ok())
                .filter(|code| (100..600).contains(code))
                .ok_or(PlatformError {
                    code: "probe:http_invalid",
                    os_code: None,
                })?;
            Ok(SocketEvidence {
                actual_source,
                tls_validated: true,
                http_status: Some(status),
                response_bytes: response.len(),
            })
        }

        fn global_route(
            &self,
            _family: AddressFamily,
            destination: SocketAddr,
        ) -> Result<GlobalRouteReply, PlatformError> {
            let destination = sockaddr_inet(destination);
            let mut row = MIB_IPFORWARD_ROW2::default();
            let mut source = SOCKADDR_INET::default();
            // SAFETY: null/zero requests the global best route and output buffers are writable.
            let status =
                unsafe { GetBestRoute2(null(), 0, null(), &destination, 0, &mut row, &mut source) };
            if status != NO_ERROR {
                return Err(os_error("probe:route_unavailable", status));
            }
            let mut interface = MIB_IF_ROW2 {
                InterfaceLuid: row.InterfaceLuid,
                InterfaceIndex: row.InterfaceIndex,
                ..MIB_IF_ROW2::default()
            };
            // SAFETY: row identifies the interface and interface is writable.
            let if_status = unsafe { GetIfEntry2(&mut interface) };
            if if_status != NO_ERROR {
                return Err(os_error("probe:route_unavailable", if_status));
            }
            let owner = classify_global_interface(RawInterfaceKind {
                interface_type: interface.Type,
                tunnel_type: interface.TunnelType,
                media_type: interface.MediaType,
                physical_medium_type: interface.PhysicalMediumType,
                access_type: interface.AccessType,
            });
            // SAFETY: route LUID was initialized by GetBestRoute2.
            Ok(GlobalRouteReply {
                luid: unsafe { row.InterfaceLuid.Value },
                interface_index: row.InterfaceIndex,
                owner,
            })
        }
    }

    struct NativeDnsLease;

    impl NativeDnsLease {
        fn acquire() -> Result<Self, PlatformError> {
            DNS_NATIVE_PENDING
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map(|_| Self)
                .map_err(|_| PlatformError {
                    code: "probe:dns_cancelled",
                    os_code: None,
                })
        }
    }

    impl Drop for NativeDnsLease {
        fn drop(&mut self) {
            DNS_NATIVE_PENDING.store(false, Ordering::Release);
        }
    }

    #[derive(Default)]
    struct NativeDnsShared {
        result: Mutex<Option<Result<Vec<IpAddr>, PlatformError>>>,
        ready: Condvar,
        cancel: Mutex<Option<usize>>,
    }

    fn dns_worker(request: BoundDnsRequest, shared: Arc<NativeDnsShared>, lease: NativeDnsLease) {
        let name: Vec<u16> = "example.com\0".encode_utf16().collect();
        let mut pending = Box::new(PendingDns {
            name,
            request: DNS_QUERY_REQUEST::default(),
            result: DNS_QUERY_RESULT {
                Version: DNS_QUERY_RESULTS_VERSION1,
                ..DNS_QUERY_RESULT::default()
            },
            cancel: DNS_QUERY_CANCEL::default(),
        });
        let (sender, receiver) = mpsc::sync_channel(1);
        let context = Arc::new(DnsCallback {
            sender: Mutex::new(Some(sender)),
        });
        let callback_ref = Arc::into_raw(Arc::clone(&context));
        pending.request = DNS_QUERY_REQUEST {
            Version: DNS_QUERY_REQUEST_VERSION1,
            QueryName: pending.name.as_ptr(),
            QueryType: match request.family {
                AddressFamily::Ipv4 => DNS_TYPE_A,
                AddressFamily::Ipv6 => DNS_TYPE_AAAA,
            },
            QueryOptions: u64::from(
                DNS_QUERY_BYPASS_CACHE
                    | DNS_QUERY_NO_HOSTS_FILE
                    | DNS_QUERY_NO_MULTICAST
                    | DNS_QUERY_NO_NETBT,
            ),
            pDnsServerList: null_mut(),
            InterfaceIndex: request.interface_index,
            pQueryCompletionCallback: Some(dns_completed),
            pQueryContext: callback_ref.cast_mut().cast(),
        };
        // SAFETY: the Box and name allocation never move while Windows may refer to the request,
        // result, cancel handle, query name or callback context. On timeout this worker remains
        // alive until the callback, so caller return cannot invalidate any of those pointers.
        let status =
            unsafe { DnsQueryEx(&pending.request, &mut pending.result, &mut pending.cancel) };
        let result = if status == DNS_REQUEST_PENDING {
            if let Ok(mut cancel) = shared.cancel.lock() {
                *cancel = Some((&pending.cancel as *const DNS_QUERY_CANCEL) as usize);
            }
            let callback_received = receiver.recv().is_ok();
            if let Ok(mut cancel) = shared.cancel.lock() {
                *cancel = None;
            }
            if callback_received {
                finish_native_dns(&mut pending, 0)
            } else {
                drop(take_dns_records(&mut pending));
                Err(PlatformError {
                    code: "probe:dns_cancelled",
                    os_code: None,
                })
            }
        } else {
            // SAFETY: a non-pending return completes synchronously and therefore does not
            // schedule the callback; reclaim the exact Arc reference passed to DnsQueryEx.
            drop(unsafe { Arc::from_raw(callback_ref) });
            finish_native_dns(&mut pending, status)
        };
        drop(lease);
        if let Ok(mut slot) = shared.result.lock() {
            *slot = Some(result);
            shared.ready.notify_all();
        }
    }

    fn take_dns_records(pending: &mut PendingDns) -> DnsRecords {
        let guard = DnsRecords(pending.result.pQueryRecords);
        pending.result.pQueryRecords = null_mut();
        guard
    }

    fn finish_native_dns(
        pending: &mut PendingDns,
        api_status: i32,
    ) -> Result<Vec<IpAddr>, PlatformError> {
        // Take ownership before inspecting either status: Windows may supply records on
        // synchronous errors and the list must be freed on every completed branch.
        let guard = take_dns_records(pending);
        if api_status != 0 {
            return Err(PlatformError {
                code: "probe:dns_api_failed",
                os_code: u32::try_from(api_status).ok(),
            });
        }
        if pending.result.QueryStatus != 0 {
            return Err(PlatformError {
                code: "probe:dns_failed",
                os_code: u32::try_from(pending.result.QueryStatus).ok(),
            });
        }
        let mut records = Vec::new();
        let mut current = guard.0;
        while !current.is_null() {
            // SAFETY: current traverses the record list returned by DnsQueryEx until null.
            let record = unsafe { &*current };
            if record.wType == DNS_TYPE_A {
                // SAFETY: wType discriminates the A union arm; bytes are in network order.
                let raw = unsafe { record.Data.A.IpAddress };
                records.push(IpAddr::V4(Ipv4Addr::from(raw.to_ne_bytes())));
            } else if record.wType == DNS_TYPE_AAAA {
                // SAFETY: wType discriminates the AAAA union arm.
                let raw = unsafe { record.Data.AAAA.Ip6Address.IP6Byte };
                records.push(IpAddr::V6(Ipv6Addr::from(raw)));
            }
            current = record.pNext;
        }
        Ok(records)
    }

    fn revalidate_interface(luid: u64, index: u32) -> Result<(), PlatformError> {
        let mut row = MIB_IF_ROW2 {
            InterfaceLuid: NET_LUID_LH { Value: luid },
            InterfaceIndex: index,
            ..MIB_IF_ROW2::default()
        };
        // SAFETY: row contains the intended identity and is writable for API output.
        let status = unsafe { GetIfEntry2(&mut row) };
        if status != NO_ERROR || row.InterfaceIndex != index {
            return Err(os_error("probe:adapter_removed", status));
        }
        Ok(())
    }

    fn sockaddr_inet(address: SocketAddr) -> SOCKADDR_INET {
        match address {
            SocketAddr::V4(value) => {
                let octets = value.ip().octets();
                let mut address = windows_sys::Win32::Networking::WinSock::IN_ADDR::default();
                address.S_un.S_addr = u32::from_ne_bytes(octets);
                let raw = SOCKADDR_IN {
                    sin_family: AF_INET,
                    sin_port: value.port().to_be(),
                    sin_addr: address,
                    ..SOCKADDR_IN::default()
                };
                SOCKADDR_INET { Ipv4: raw }
            }
            SocketAddr::V6(value) => {
                let mut address = windows_sys::Win32::Networking::WinSock::IN6_ADDR::default();
                address.u.Byte = value.ip().octets();
                let anonymous = windows_sys::Win32::Networking::WinSock::SOCKADDR_IN6_0 {
                    sin6_scope_id: value.scope_id(),
                };
                let raw = SOCKADDR_IN6 {
                    sin6_family: AF_INET6,
                    sin6_port: value.port().to_be(),
                    sin6_flowinfo: value.flowinfo(),
                    sin6_addr: address,
                    Anonymous: anonymous,
                };
                SOCKADDR_INET { Ipv6: raw }
            }
        }
    }
    fn ip_from_inet(value: &SOCKADDR_INET) -> Option<IpAddr> {
        /* SAFETY: si_family selects the initialized union arm returned by Windows. */
        unsafe {
            match value.si_family {
                AF_INET => {
                    let r = value.Ipv4;
                    Some(IpAddr::V4(Ipv4Addr::from(
                        r.sin_addr.S_un.S_addr.to_ne_bytes(),
                    )))
                }
                AF_INET6 => Some(IpAddr::V6(Ipv6Addr::from(value.Ipv6.sin6_addr.u.Byte))),
                _ => None,
            }
        }
    }
    fn io_error(code: &'static str, error: &std::io::Error) -> PlatformError {
        PlatformError {
            code,
            os_code: error.raw_os_error().and_then(|v| u32::try_from(v).ok()),
        }
    }

    impl DeadlineRead for native_tls::TlsStream<TcpStream> {
        fn set_deadline_read_timeout(&mut self, timeout: Duration) -> std::io::Result<()> {
            self.get_ref().set_read_timeout(Some(timeout))
        }
    }

    fn write_http_request(
        stream: &mut native_tls::TlsStream<TcpStream>,
        bytes: &[u8],
        deadline: MonotonicDeadline,
        stage_started: Instant,
        stage_cap: Duration,
    ) -> Result<(), PlatformError> {
        let mut written = 0;
        while written < bytes.len() {
            let timeout = deadline
                .remaining(stage_started, stage_cap)
                .ok_or(PlatformError {
                    code: "probe:http_timeout",
                    os_code: None,
                })?;
            stream
                .get_ref()
                .set_write_timeout(Some(timeout))
                .map_err(|error| io_error("probe:http_timeout", &error))?;
            match stream.write(&bytes[written..]) {
                Ok(0) => {
                    return Err(PlatformError {
                        code: "probe:http_invalid",
                        os_code: None,
                    });
                }
                Ok(count) => written += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if deadline.remaining(stage_started, stage_cap).is_none() {
                        return Err(io_error("probe:http_timeout", &error));
                    }
                }
                Err(error) => return Err(io_error("probe:http_invalid", &error)),
            }
        }
        Ok(())
    }

    fn classify_native_tls_error(
        error: &native_tls::Error,
        deadline_elapsed: bool,
    ) -> PlatformError {
        if deadline_elapsed {
            return PlatformError {
                code: "probe:tls_timeout",
                os_code: None,
            };
        }
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
        while let Some(current) = source {
            if let Some(io) = current.downcast_ref::<std::io::Error>()
                && matches!(
                    io.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
            {
                return classify_tls_io_error(io);
            }
            source = current.source();
        }
        PlatformError {
            code: "probe:tls_failed",
            os_code: None,
        }
    }
    fn os_error(code: &'static str, status: u32) -> PlatformError {
        PlatformError {
            code,
            os_code: Some(status),
        }
    }
    fn last_socket_error(code: &'static str) -> PlatformError {
        io_error(code, &std::io::Error::last_os_error())
    }

    struct DnsRecords(*mut DNS_RECORDA);
    impl Drop for DnsRecords {
        fn drop(&mut self) {
            if !self.0.is_null() {
                /* SAFETY: pointer was allocated by DnsQueryEx and is released exactly once with DnsFreeRecordList. */
                unsafe { DnsRecordListFree(self.0.cast(), 1) };
            }
        }
    }
    #[link(name = "dnsapi")]
    unsafe extern "system" {
        fn DnsRecordListFree(records: *mut c_void, free_type: i32);
    }

    struct PendingDns {
        name: Vec<u16>,
        request: DNS_QUERY_REQUEST,
        result: DNS_QUERY_RESULT,
        cancel: DNS_QUERY_CANCEL,
    }
    struct DnsCallback {
        sender: Mutex<Option<mpsc::SyncSender<()>>>,
    }
    unsafe extern "system" fn dns_completed(
        context: *const c_void,
        _results: *mut DNS_QUERY_RESULT,
    ) {
        if context.is_null() {
            return;
        }
        // SAFETY: context came from exactly one Arc::into_raw for this callback and is consumed
        // exactly once by the single documented DnsQueryEx completion callback.
        let callback = unsafe { Arc::from_raw(context.cast::<DnsCallback>()) };
        if let Ok(mut sender) = callback.sender.lock()
            && let Some(sender) = sender.take()
        {
            let _ = sender.try_send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        time::{Duration, SystemTime},
    };

    use super::*;
    use crate::adapter::tests_support::{fixture_identity, fixture_identity_v6};
    use dji4g_domain::{DefaultRouteOwner, DeviceEpoch};

    fn policy() -> ProbePolicy {
        ProbePolicy {
            dns_timeout: Duration::from_secs(1),
            connect_timeout: Duration::from_secs(1),
            tls_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(1),
            total_timeout: Duration::from_secs(5),
            max_response_bytes: 4096,
        }
    }

    #[test]
    fn target_route_and_source_are_mandatory_despite_phone_and_tun_connectivity() {
        let target = fixture_identity(DeviceEpoch(7), 55, 42, "192.168.225.2");
        let mut backend = FakeProbeBackend::default();
        backend.best_routes.push(Err(PlatformError {
            code: "probe:route_unavailable",
            os_code: None,
        }));
        backend.global_routes.push(Ok(GlobalRouteReply {
            luid: 900,
            interface_index: 9,
            owner: DefaultRouteOwner::VpnOrTun,
        }));

        let result = observe_now(&backend, &target, &policy()).unwrap();

        assert!(matches!(
            result.families[0].endpoints[0].route.outcome,
            ProbeStage::Failed {
                code: "probe:route_unavailable",
                ..
            }
        ));
        assert!(backend.https_requests().is_empty());
        assert!(result.global_routes[0].explanation_only);
    }

    #[test]
    fn apipa_source_never_reaches_route_or_socket_backends() {
        let target = fixture_identity(DeviceEpoch(14), 55, 42, "169.254.7.8");
        let backend = FakeProbeBackend::default();

        let result = observe_now(&backend, &target, &policy()).unwrap();

        assert!(result.families[0].endpoints.iter().all(|attempt| matches!(
            attempt.route.outcome,
            ProbeStage::Unavailable {
                code: "net:no_usable_address"
            }
        )));
        assert!(backend.best_route_requests().is_empty());
        assert!(backend.https_requests().is_empty());
    }

    #[test]
    fn dns_failure_and_bound_https_success_are_both_preserved() {
        let target = fixture_identity(DeviceEpoch(8), 55, 42, "192.168.225.2");
        let mut backend = FakeProbeBackend::default();
        backend.best_routes.extend((0..4).map(|_| {
            Ok(BestRouteReply {
                luid: 55,
                interface_index: 42,
                source: "192.168.225.2".parse().unwrap(),
                next_hop: "192.168.225.1".parse().unwrap(),
                route_metric: 5,
            })
        }));
        backend.dns.push(Err(PlatformError {
            code: "probe:dns_timeout",
            os_code: Some(1460),
        }));
        backend.https.extend((0..4).map(|_| {
            Ok(SocketEvidence {
                actual_source: "192.168.225.2".parse().unwrap(),
                tls_validated: true,
                http_status: Some(200),
                response_bytes: 128,
            })
        }));

        let result = observe_now(&backend, &target, &policy()).unwrap();
        let family = &result.families[0];
        assert!(matches!(
            family.dns.outcome,
            ProbeStage::Failed {
                code: "probe:dns_timeout",
                ..
            }
        ));
        assert_eq!(family.endpoints.len(), 4);
        assert!(family.endpoints.iter().all(|attempt| matches!(
            attempt.tls.outcome,
            ProbeStage::Succeeded(TlsEvidence { validated: true })
        ) && matches!(
            attempt.http.outcome,
            ProbeStage::Succeeded(HttpEvidence { status: 200, .. })
        )));
        assert!(
            backend
                .dns_requests()
                .iter()
                .all(|request| request.interface_index == 42)
        );
        assert!(
            backend
                .dns_requests()
                .iter()
                .all(|request| request.interface_index != 0)
        );
    }

    #[test]
    fn route_identity_and_getsockname_source_mismatches_fail_closed() {
        let target = fixture_identity(DeviceEpoch(9), 55, 42, "192.168.225.2");
        let mut wrong_route = FakeProbeBackend::default();
        wrong_route.best_routes.push(Ok(BestRouteReply {
            luid: 777,
            interface_index: 5,
            source: "172.20.10.2".parse().unwrap(),
            next_hop: "172.20.10.1".parse().unwrap(),
            route_metric: 1,
        }));
        let result = observe_now(&wrong_route, &target, &policy()).unwrap();
        assert!(matches!(
            result.families[0].endpoints[0].route.outcome,
            ProbeStage::Failed {
                code: "probe:route_identity_mismatch",
                ..
            }
        ));
        assert!(wrong_route.https_requests().is_empty());

        let mut wrong_source = FakeProbeBackend::default();
        wrong_source.best_routes.extend((0..4).map(|_| {
            Ok(BestRouteReply {
                luid: 55,
                interface_index: 42,
                source: "192.168.225.2".parse().unwrap(),
                next_hop: "192.168.225.1".parse().unwrap(),
                route_metric: 5,
            })
        }));
        wrong_source.https.extend((0..4).map(|_| {
            Ok(SocketEvidence {
                actual_source: "172.20.10.2".parse().unwrap(),
                tls_validated: true,
                http_status: Some(200),
                response_bytes: 64,
            })
        }));
        let result = observe_now(&wrong_source, &target, &policy()).unwrap();
        assert!(result.families[0].endpoints.iter().all(|attempt| matches!(
            attempt.connect.outcome,
            ProbeStage::Failed {
                code: "probe:source_mismatch",
                ..
            }
        )));
    }

    #[test]
    fn socket_interface_options_have_family_specific_byte_order() {
        let target = fixture_identity(DeviceEpoch(10), 55, 0x0102_0304, "192.168.225.2");
        let mut backend = FakeProbeBackend::default();
        backend.best_routes.extend((0..2).map(|_| {
            Ok(BestRouteReply {
                luid: 55,
                interface_index: 0x0102_0304,
                source: IpAddr::V4(Ipv4Addr::new(192, 168, 225, 2)),
                next_hop: IpAddr::V4(Ipv4Addr::new(192, 168, 225, 1)),
                route_metric: 1,
            })
        }));
        backend.https.extend((0..2).map(|_| {
            Err(PlatformError {
                code: "probe:connect_failed",
                os_code: None,
            })
        }));
        let _ = observe_now(&backend, &target, &policy()).unwrap();
        assert!(
            backend
                .https_requests()
                .iter()
                .all(|request| request.socket_interface_value == 0x0102_0304_u32.to_be())
        );

        let target_v6 = fixture_identity_v6(DeviceEpoch(10), 55, 0x0102_0304, "2001:db8::2");
        let mut backend_v6 = FakeProbeBackend::default();
        backend_v6.best_routes.extend((0..2).map(|_| {
            Ok(BestRouteReply {
                luid: 55,
                interface_index: 0x0102_0304,
                source: "2001:db8::2".parse().unwrap(),
                next_hop: "2001:db8::1".parse().unwrap(),
                route_metric: 1,
            })
        }));
        backend_v6.https.extend((0..2).map(|_| {
            Err(PlatformError {
                code: "probe:connect_failed",
                os_code: None,
            })
        }));
        let _ = observe_now(&backend_v6, &target_v6, &policy()).unwrap();
        assert!(
            backend_v6
                .https_requests()
                .iter()
                .all(|request| request.socket_interface_value == 0x0102_0304)
        );
    }

    #[test]
    fn endpoint_failure_and_success_remain_independent_of_global_route_owner() {
        let target = fixture_identity(DeviceEpoch(13), 55, 42, "192.168.225.2");
        let mut backend = FakeProbeBackend::default();
        backend.best_routes.extend((0..2).map(|_| {
            Ok(BestRouteReply {
                luid: 55,
                interface_index: 42,
                source: "192.168.225.2".parse().unwrap(),
                next_hop: "192.168.225.1".parse().unwrap(),
                route_metric: 5,
            })
        }));
        backend.https.push(Err(PlatformError {
            code: "probe:connect_timeout",
            os_code: None,
        }));
        backend.https.push(Ok(SocketEvidence {
            actual_source: "192.168.225.2".parse().unwrap(),
            tls_validated: true,
            http_status: Some(204),
            response_bytes: 80,
        }));
        backend.global_routes.push(Ok(GlobalRouteReply {
            luid: 900,
            interface_index: 9,
            owner: DefaultRouteOwner::VpnOrTun,
        }));

        let result = observe_now(&backend, &target, &policy()).unwrap();
        assert!(matches!(
            result.families[0].endpoints[0].connect.outcome,
            ProbeStage::Failed {
                code: "probe:connect_timeout",
                ..
            }
        ));
        assert!(matches!(
            result.families[0].endpoints[1].http.outcome,
            ProbeStage::Succeeded(_)
        ));
        assert_eq!(result.chosen_family, Some(AddressFamily::Ipv4));
        assert!(matches!(
            result.global_routes[0].route.outcome,
            ProbeStage::Succeeded(GlobalRouteEvidence {
                owner: DefaultRouteOwner::VpnOrTun,
                ..
            })
        ));
        assert!(result.global_routes[0].explanation_only);
    }

    #[test]
    fn policy_rejects_zero_or_unbounded_limits() {
        let target = fixture_identity(DeviceEpoch(11), 55, 42, "192.168.225.2");
        let backend = FakeProbeBackend::default();
        let mut invalid = policy();
        invalid.max_response_bytes = 0;
        assert_eq!(
            observe_now(&backend, &target, &invalid).unwrap_err().code,
            "probe:policy_invalid"
        );
        invalid = policy();
        invalid.total_timeout = Duration::from_secs(60);
        assert_eq!(
            observe_now(&backend, &target, &invalid).unwrap_err().code,
            "probe:policy_invalid"
        );
    }

    #[test]
    fn evidence_timestamps_are_ordered_and_endpoint_inputs_are_closed_constants() {
        let target = fixture_identity(DeviceEpoch(12), 55, 42, "192.168.225.2");
        let mut backend = FakeProbeBackend {
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(100),
            ..FakeProbeBackend::default()
        };
        backend.best_routes.extend((0..2).map(|_| {
            Err(PlatformError {
                code: "probe:route_unavailable",
                os_code: None,
            })
        }));
        let result = observe_now(&backend, &target, &policy()).unwrap();
        assert!(result.finished_at >= result.started_at);
        let destinations: Vec<SocketAddr> = backend
            .best_route_requests()
            .iter()
            .map(|r| r.destination)
            .collect();
        assert_eq!(
            destinations,
            vec![
                "223.5.5.5:443".parse().unwrap(),
                "1.12.12.21:443".parse().unwrap(),
                "1.1.1.1:443".parse().unwrap(),
                "8.8.8.8:443".parse().unwrap()
            ]
        );
    }

    #[test]
    fn route_evidence_survives_connect_failure_and_later_stages_are_unexecuted() {
        let target = fixture_identity(DeviceEpoch(21), 55, 42, "192.168.225.2");
        let mut backend = FakeProbeBackend::default();
        backend.best_routes.extend((0..2).map(|_| {
            Ok(BestRouteReply {
                luid: 55,
                interface_index: 42,
                source: "192.168.225.2".parse().unwrap(),
                next_hop: "192.168.225.1".parse().unwrap(),
                route_metric: 5,
            })
        }));
        backend.https.extend((0..2).map(|_| {
            Err(PlatformError {
                code: "probe:connect_timeout",
                os_code: Some(10060),
            })
        }));

        let result = observe_now(&backend, &target, &policy()).unwrap();
        let attempt = &result.families[0].endpoints[0];
        assert!(
            matches!(attempt.route.outcome, ProbeStage::Succeeded(ref route) if route.luid == 55 && route.source == "192.168.225.2".parse::<IpAddr>().unwrap())
        );
        assert!(matches!(
            attempt.connect.outcome,
            ProbeStage::Failed {
                code: "probe:connect_timeout",
                ..
            }
        ));
        assert!(matches!(attempt.tls.outcome, ProbeStage::Unexecuted { .. }));
        assert!(matches!(
            attempt.http.outcome,
            ProbeStage::Unexecuted { .. }
        ));
        assert!(attempt.finished_at >= attempt.started_at);
    }

    #[test]
    fn failed_global_route_remains_an_explanation_only_timed_error() {
        let target = fixture_identity(DeviceEpoch(22), 55, 42, "192.168.225.2");
        let mut backend = FakeProbeBackend::default();
        backend.global_routes.push(Err(PlatformError {
            code: "probe:route_unavailable",
            os_code: Some(1234),
        }));

        let result = observe_now(&backend, &target, &policy()).unwrap();
        assert!(result.global_routes[0].explanation_only);
        assert!(matches!(
            result.global_routes[0].route.outcome,
            ProbeStage::Failed {
                code: "probe:route_unavailable",
                os_code: Some(1234)
            }
        ));
    }

    #[test]
    fn raw_interface_classification_recognizes_only_structural_virtual_evidence() {
        let tunnel = RawInterfaceKind {
            interface_type: IF_TYPE_TUNNEL_RAW,
            tunnel_type: 0,
            media_type: NDIS_MEDIUM_802_3_RAW,
            physical_medium_type: NDIS_PHYSICAL_MEDIUM_802_3_RAW,
            access_type: NET_IF_ACCESS_BROADCAST_RAW,
        };
        let ppp = RawInterfaceKind {
            interface_type: IF_TYPE_PPP_RAW,
            ..tunnel
        };
        let prop_virtual = RawInterfaceKind {
            interface_type: IF_TYPE_PROP_VIRTUAL_RAW,
            ..tunnel
        };
        let meta_like = RawInterfaceKind {
            interface_type: IF_TYPE_ETHERNET_RAW,
            tunnel_type: 0,
            media_type: NDIS_MEDIUM_IP_RAW,
            physical_medium_type: NDIS_PHYSICAL_MEDIUM_UNSPECIFIED_RAW,
            access_type: NET_IF_ACCESS_POINT_TO_POINT_RAW,
        };
        let renamed_physical = RawInterfaceKind {
            interface_type: IF_TYPE_ETHERNET_RAW,
            ..tunnel
        };

        for raw in [tunnel, ppp, prop_virtual, meta_like] {
            assert_eq!(classify_global_interface(raw), DefaultRouteOwner::VpnOrTun);
        }
        assert_eq!(
            classify_global_interface(renamed_physical),
            DefaultRouteOwner::Other
        );
    }

    #[test]
    fn hard_deadline_caps_each_fragment_and_never_reads_past_four_kib() {
        let mut reader = SlowFragmentReader::new(vec![vec![b'x'; 3000], vec![b'y'; 3000]]);
        let result = read_http_headers_bounded(
            &mut reader,
            TestDeadline::with_budget(Duration::from_millis(50)),
            Duration::from_millis(25),
            4096,
        );
        assert_eq!(result.unwrap_err().code, "probe:http_too_large");
        assert_eq!(reader.bytes_returned(), 4096);
        assert!(reader.max_requested() <= 4096);
    }

    #[test]
    fn slow_fragmented_stream_exhausts_one_shared_deadline() {
        let mut reader = SlowFragmentReader::with_delay(
            vec![b"HTTP/1.1 200".to_vec(), b" OK\r\n".to_vec()],
            Duration::from_millis(20),
        );
        let started = Instant::now();
        let result = read_http_headers_bounded(
            &mut reader,
            TestDeadline::with_budget(Duration::from_millis(25)),
            Duration::from_secs(1),
            4096,
        );
        assert_eq!(result.unwrap_err().code, "probe:http_timeout");
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn timed_out_tls_io_is_classified_as_tls_timeout() {
        let nested = std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline");
        assert_eq!(classify_tls_io_error(&nested).code, "probe:tls_timeout");
    }

    #[test]
    fn dns_pending_gate_is_bounded_and_recovers_after_late_completion() {
        let gate = Arc::new(DnsPendingGate::default());
        let first = gate.try_begin().unwrap();
        assert_eq!(gate.try_begin().unwrap_err().code, "probe:dns_cancelled");
        let callback = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            first.complete();
        });
        assert_eq!(gate.try_begin().unwrap_err().code, "probe:dns_cancelled");
        callback.join().unwrap();
        let second = gate.try_begin().unwrap();
        second.complete();
    }

    #[test]
    fn dns_cancel_race_allows_only_one_bounded_pending_operation() {
        let gate = Arc::new(DnsPendingGate::default());
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let gate = Arc::clone(&gate);
            let barrier = Arc::clone(&barrier);
            let accepted = Arc::clone(&accepted);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                if let Ok(lease) = gate.try_begin() {
                    accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(5));
                    lease.complete();
                }
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
        gate.try_begin().unwrap().complete();
    }

    #[test]
    fn dns_completion_takes_record_ownership_before_inspecting_error_status() {
        let drops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let records = DropSpy::new(std::sync::Arc::clone(&drops));
        let result = finish_dns_completion(records, 9002);
        assert_eq!(result.unwrap_err().code, "probe:dns_failed");
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
