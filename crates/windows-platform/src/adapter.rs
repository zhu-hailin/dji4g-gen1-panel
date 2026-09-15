use std::{collections::BTreeSet, net::IpAddr, str::FromStr};

use dji4g_domain::DeviceEpoch;

use crate::{DjiDevice, PlatformError};

const GAA_INCLUDE_ALL_INTERFACES: u32 = 0x0100;

fn adapter_enumeration_flags() -> u32 {
    // Keep this platform-independent seam covered by tests: down adapters and adapters with
    // neither address family bound must still be discoverable by their authoritative GUID.
    GAA_INCLUDE_ALL_INTERFACES
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AdapterGuid([u8; 16]);

impl AdapterGuid {
    #[must_use]
    pub fn canonical(self) -> String {
        let b = self.0;
        format!(
            "{{{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

impl std::fmt::Debug for AdapterGuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl FromStr for AdapterGuid {
    type Err = PlatformError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value
            .strip_prefix('{')
            .and_then(|v| v.strip_suffix('}'))
            .ok_or(PlatformError {
                code: "net:netcfg_id_invalid",
                os_code: None,
            })?;
        if value.len() != 36
            || ![8, 13, 18, 23]
                .into_iter()
                .all(|i| value.as_bytes()[i] == b'-')
        {
            return Err(PlatformError {
                code: "net:netcfg_id_invalid",
                os_code: None,
            });
        }
        let hex: String = value.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(PlatformError {
                code: "net:netcfg_id_invalid",
                os_code: None,
            });
        }
        let mut bytes = [0_u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| PlatformError {
                code: "net:netcfg_id_invalid",
                os_code: None,
            })?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// An epoch-scoped adapter identity produced by WindowsAdapterResolver.
///
/// Its fields are private so callers cannot direct production probes at an arbitrary interface,
/// source address, or stale epoch.
///
/// ```compile_fail
/// use dji4g_domain::DeviceEpoch;
/// use dji4g_windows_platform::AdapterIdentity;
/// let _forged = AdapterIdentity {
///     guid: "{00000000-0000-0000-0000-000000000000}".parse().unwrap(),
///     luid: 1,
///     ipv4_index: Some(1),
///     ipv6_index: None,
///     epoch: DeviceEpoch(1),
///     unicast_addresses: vec!["127.0.0.1".parse().unwrap()],
/// };
/// ```
pub struct AdapterIdentity {
    guid: AdapterGuid,
    luid: u64,
    ipv4_index: Option<u32>,
    ipv6_index: Option<u32>,
    epoch: DeviceEpoch,
    unicast_addresses: Vec<IpAddr>,
}

impl AdapterIdentity {
    #[must_use]
    pub fn guid_string(&self) -> String {
        self.guid.canonical()
    }
    #[must_use]
    pub const fn luid(&self) -> u64 {
        self.luid
    }
    #[must_use]
    pub const fn ipv4_index(&self) -> Option<u32> {
        self.ipv4_index
    }
    #[must_use]
    pub const fn ipv6_index(&self) -> Option<u32> {
        self.ipv6_index
    }
    #[must_use]
    pub const fn epoch(&self) -> DeviceEpoch {
        self.epoch
    }
    #[must_use]
    pub fn unicast_addresses(&self) -> &[IpAddr] {
        &self.unicast_addresses
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteObservation {
    pub family: AddressFamily,
    pub interface_index: u32,
    pub destination: IpAddr,
    pub prefix_len: u8,
    pub next_hop: IpAddr,
    pub route_metric: u32,
    pub total_metric: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterObservation {
    pub identity: AdapterIdentity,
    pub oper_up: bool,
    pub dhcp_v4: bool,
    pub dhcp_v6: bool,
    pub gateways: Vec<IpAddr>,
    pub dns_servers: Vec<IpAddr>,
    pub routes: Vec<RouteObservation>,
    pub usable_families: Vec<AddressFamily>,
    /// Monotonic interface byte counters (both families combined), sampled for the rate
    /// display.  `None` means the native query failed this cycle — an honest gap, never a
    /// fabricated zero.
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RawAdapter {
    guid: AdapterGuid,
    luid: u64,
    friendly_name: String,
    ipv4_index: u32,
    ipv6_index: u32,
    oper_up: bool,
    dhcp_v4: bool,
    dhcp_v6: bool,
    unicast: Vec<IpAddr>,
    gateways: Vec<IpAddr>,
    dns_servers: Vec<IpAddr>,
    ipv4_metric: u32,
    ipv6_metric: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RawRoute {
    luid: u64,
    interface_index: u32,
    family: AddressFamily,
    destination: IpAddr,
    prefix_len: u8,
    next_hop: IpAddr,
    route_metric: u32,
}

fn resolve_observed_adapter(
    epoch: DeviceEpoch,
    netcfg_ids: &[String],
    adapters: &[RawAdapter],
    routes: &[RawRoute],
) -> Result<AdapterObservation, PlatformError> {
    if netcfg_ids.is_empty() {
        return Err(PlatformError {
            code: "net:netcfg_id_missing",
            os_code: None,
        });
    }
    let ids: Result<BTreeSet<_>, _> = netcfg_ids
        .iter()
        .map(|id| id.parse::<AdapterGuid>())
        .collect();
    let ids = ids?;
    if netcfg_ids.len() != ids.len() {
        return Err(PlatformError {
            code: "net:adapter_identity_mismatch",
            os_code: None,
        });
    }
    if ids.len() != 1 {
        return Err(PlatformError {
            code: "net:adapter_ambiguous",
            os_code: None,
        });
    }
    let guid = *ids.iter().next().expect("one unique NetCfgInstanceId");
    let matching: Vec<_> = adapters.iter().filter(|row| row.guid == guid).collect();
    let row = match matching.as_slice() {
        [] => {
            return Err(PlatformError {
                code: "net:adapter_not_found",
                os_code: None,
            });
        }
        [row] => *row,
        _ => {
            return Err(PlatformError {
                code: "net:adapter_identity_mismatch",
                os_code: None,
            });
        }
    };
    let ipv4_index = (row.ipv4_index != 0).then_some(row.ipv4_index);
    let ipv6_index = (row.ipv6_index != 0).then_some(row.ipv6_index);
    let identity = AdapterIdentity {
        guid,
        luid: row.luid,
        ipv4_index,
        ipv6_index,
        epoch,
        unicast_addresses: row.unicast.clone(),
    };
    let target_routes: Vec<_> = routes
        .iter()
        .filter(|route| route.luid == row.luid)
        .map(|route| {
            let interface_metric = match route.family {
                AddressFamily::Ipv4 => row.ipv4_metric,
                AddressFamily::Ipv6 => row.ipv6_metric,
            };
            RouteObservation {
                family: route.family,
                interface_index: route.interface_index,
                destination: route.destination,
                prefix_len: route.prefix_len,
                next_hop: route.next_hop,
                route_metric: route.route_metric,
                total_metric: route.route_metric.saturating_add(interface_metric),
            }
        })
        .collect();
    let mut usable_families = Vec::new();
    if row.oper_up {
        for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
            let has_source = row
                .unicast
                .iter()
                .copied()
                .any(|ip| is_usable_source(ip, family));
            let expected_index = match family {
                AddressFamily::Ipv4 => ipv4_index,
                AddressFamily::Ipv6 => ipv6_index,
            };
            let has_route = target_routes.iter().any(|route| {
                route.family == family
                    && Some(route.interface_index) == expected_index
                    && route.prefix_len == 0
            });
            if has_source && has_route {
                usable_families.push(family);
            }
        }
    }
    Ok(AdapterObservation {
        identity,
        oper_up: row.oper_up,
        dhcp_v4: row.dhcp_v4,
        dhcp_v6: row.dhcp_v6,
        gateways: row.gateways.clone(),
        dns_servers: row.dns_servers.clone(),
        routes: target_routes,
        usable_families,
        rx_bytes: None,
        tx_bytes: None,
    })
}

pub(crate) fn is_usable_source(ip: IpAddr, family: AddressFamily) -> bool {
    if matches!(
        (family, ip),
        (AddressFamily::Ipv4, IpAddr::V6(_)) | (AddressFamily::Ipv6, IpAddr::V4(_))
    ) {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_unspecified() && !v4.is_loopback() && !v4.is_link_local() && !v4.is_multicast()
        }
        IpAddr::V6(v6) => {
            !v6.is_unspecified()
                && !v6.is_loopback()
                && !v6.is_unicast_link_local()
                && !v6.is_multicast()
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsAdapterResolver;

impl WindowsAdapterResolver {
    pub fn resolve(
        &self,
        device: &DjiDevice,
        epoch: DeviceEpoch,
    ) -> Result<AdapterObservation, PlatformError> {
        let netcfg_ids = checked_netcfg_ids(
            device
                .net_candidates()
                .iter()
                .map(|candidate| candidate.net_cfg_instance_id()),
        )?;
        let (adapters, routes) = collect_platform_network()?;
        let mut observation = resolve_observed_adapter(epoch, &netcfg_ids, &adapters, &routes)?;
        let (rx_bytes, tx_bytes) = byte_counters(observation.identity.luid()).unzip();
        observation.rx_bytes = rx_bytes;
        observation.tx_bytes = tx_bytes;
        Ok(observation)
    }

    /// Read-only byte counters for one already-bound adapter GUID.
    ///
    /// Backs the application's 1 s rates-only tick: it reads *only* the monotonic interface octet
    /// counters (`GetIfEntry2`) for the exact adapter the reducer already resolved and bound, keyed
    /// by its authoritative GUID. It never enumerates adapters, never inspects routes, and never
    /// touches the AT or repair paths. `None` means the GUID is malformed or the native query failed
    /// this tick — an honest gap, never a fabricated or stale number.
    #[must_use]
    pub fn read_byte_counters(&self, adapter_guid: &str) -> Option<(u64, u64)> {
        let guid: AdapterGuid = adapter_guid.parse().ok()?;
        byte_counters_by_guid(guid)
    }
}

/// One interface's MIB-II counters, exactly as `GetIfEntry2` reports them for a single LUID.
///
/// The octet, error and discard counters are monotonic per-interface totals; the link speeds are
/// the interface's negotiated bit rates, not measured internet throughput. A failed read is an
/// honest gap (`Err`), never a fabricated zero, and never a silent switch to another adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InterfaceMetrics {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub in_errors: u64,
    pub out_errors: u64,
    pub in_discards: u64,
    pub out_discards: u64,
    pub link_rx_bits_per_second: u64,
    pub link_tx_bits_per_second: u64,
}

/// Read the extended counters for exactly one already-bound interface, addressed by its LUID.
///
/// `luid == 0` never reaches the native layer. There is no fallback to interface enumeration: when
/// the exact interface cannot be queried, the caller gets an error and must record a measurement
/// gap (research appendix B / §7.1).
pub fn read_interface_metrics(luid: u64) -> Result<InterfaceMetrics, PlatformError> {
    if luid == 0 {
        return Err(PlatformError {
            code: "net:interface_luid_invalid",
            os_code: None,
        });
    }
    #[cfg(windows)]
    {
        native::read_interface_metrics(luid)
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError {
            code: "net:unsupported_platform",
            os_code: None,
        })
    }
}

fn checked_netcfg_ids<'a>(
    values: impl IntoIterator<Item = Option<&'a str>>,
) -> Result<Vec<String>, PlatformError> {
    let mut ids = Vec::new();
    for value in values {
        let value = value.ok_or(PlatformError {
            code: "net:netcfg_id_missing",
            os_code: None,
        })?;
        ids.push(value.to_owned());
    }
    if ids.is_empty() {
        return Err(PlatformError {
            code: "net:netcfg_id_missing",
            os_code: None,
        });
    }
    Ok(ids)
}

#[cfg(windows)]
use native::{byte_counters, byte_counters_by_guid};
#[cfg(not(windows))]
pub(crate) fn byte_counters(_luid: u64) -> Option<(u64, u64)> {
    None
}
#[cfg(not(windows))]
fn byte_counters_by_guid(_guid: AdapterGuid) -> Option<(u64, u64)> {
    None
}

#[cfg(not(windows))]
fn collect_platform_network() -> Result<(Vec<RawAdapter>, Vec<RawRoute>), PlatformError> {
    Err(PlatformError {
        code: "net:unsupported_platform",
        os_code: None,
    })
}

#[cfg(windows)]
fn collect_platform_network() -> Result<(Vec<RawAdapter>, Vec<RawRoute>), PlatformError> {
    native::collect()
}

#[cfg(windows)]
mod native {
    /// One `GetIfEntry2` row per LUID covers both address families' octets.
    pub(crate) fn byte_counters(luid: u64) -> Option<(u64, u64)> {
        let mut row = MIB_IF_ROW2 {
            InterfaceLuid: NET_LUID_LH { Value: luid },
            ..MIB_IF_ROW2::default()
        };
        // SAFETY: row identifies the interface and row is writable.
        if unsafe { GetIfEntry2(&mut row) } != NO_ERROR {
            return None;
        }
        Some((row.InOctets, row.OutOctets))
    }

    /// Read-only counters for one adapter addressed by its authoritative GUID.
    ///
    /// The exact inverse of [`guid_from_windows`]: rebuild the `GUID`, map it to the interface LUID,
    /// then read that single interface's octet counters. This never enumerates adapters.
    pub(crate) fn byte_counters_by_guid(guid: AdapterGuid) -> Option<(u64, u64)> {
        let b = guid.0;
        let windows_guid = GUID {
            data1: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            data2: u16::from_be_bytes([b[4], b[5]]),
            data3: u16::from_be_bytes([b[6], b[7]]),
            data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        };
        let mut luid = NET_LUID_LH::default();
        // SAFETY: windows_guid is a valid GUID and luid is a writable out-parameter.
        if unsafe { ConvertInterfaceGuidToLuid(&windows_guid, &mut luid) } != NO_ERROR {
            return None;
        }
        // SAFETY: luid.Value was initialized by ConvertInterfaceGuidToLuid on success.
        byte_counters(unsafe { luid.Value })
    }

    /// Read one interface's extended `MIB_IF_ROW2` counters by LUID.
    ///
    /// The query is exact: a failed `GetIfEntry2` is returned to the caller. This function never
    /// enumerates interfaces, so it cannot substitute a different adapter for a missing target.
    pub(crate) fn read_interface_metrics(luid: u64) -> Result<InterfaceMetrics, PlatformError> {
        let mut row = MIB_IF_ROW2 {
            InterfaceLuid: NET_LUID_LH { Value: luid },
            ..MIB_IF_ROW2::default()
        };
        // SAFETY: row identifies the interface and row is writable.
        let status = unsafe { GetIfEntry2(&mut row) };
        if status != NO_ERROR {
            return Err(os_error("net:interface_metrics_failed", status));
        }
        Ok(metrics_from_row(&row))
    }

    /// Field-for-field mapping from the native row; split out so the mapping is testable without I/O.
    pub(super) fn metrics_from_row(row: &MIB_IF_ROW2) -> InterfaceMetrics {
        InterfaceMetrics {
            rx_bytes: row.InOctets,
            tx_bytes: row.OutOctets,
            in_errors: row.InErrors,
            out_errors: row.OutErrors,
            in_discards: row.InDiscards,
            out_discards: row.OutDiscards,
            link_rx_bits_per_second: row.ReceiveLinkSpeed,
            link_tx_bits_per_second: row.TransmitLinkSpeed,
        }
    }

    use super::*;
    use std::{
        mem::size_of,
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
        ptr::{null, null_mut},
        slice,
    };
    use windows_sys::{
        Win32::{
            Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR},
            NetworkManagement::{
                IpHelper::{
                    ConvertInterfaceGuidToLuid, ConvertInterfaceLuidToGuid, FreeMibTable,
                    GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_INCLUDE_PREFIX, GetAdaptersAddresses,
                    GetIfEntry2, GetIpForwardTable2, IP_ADAPTER_ADDRESSES_LH,
                    IP_ADAPTER_DHCP_ENABLED, MIB_IF_ROW2, MIB_IPFORWARD_TABLE2,
                },
                Ndis::{IfOperStatusUp, NET_LUID_LH},
            },
            Networking::WinSock::{
                AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6,
            },
        },
        core::GUID,
    };

    const MAX_BUFFER: u32 = 4 * 1024 * 1024;

    struct MibTable(*mut MIB_IPFORWARD_TABLE2);
    impl Drop for MibTable {
        fn drop(&mut self) {
            if !self.0.is_null() {
                /* SAFETY: returned by GetIpForwardTable2 and freed once here. */
                unsafe { FreeMibTable(self.0.cast()) };
            }
        }
    }

    pub(super) fn collect() -> Result<(Vec<RawAdapter>, Vec<RawRoute>), PlatformError> {
        let adapters = adapters()?;
        let routes = routes()?;
        Ok((adapters, routes))
    }

    fn adapters() -> Result<Vec<RawAdapter>, PlatformError> {
        let mut size = 15 * 1024_u32;
        for _ in 0..4 {
            if size > MAX_BUFFER {
                break;
            }
            let words = (size as usize).div_ceil(size_of::<usize>());
            let mut buffer = vec![0_usize; words];
            let mut buffer_bytes =
                u32::try_from(buffer.len() * size_of::<usize>()).map_err(|_| PlatformError {
                    code: "net:adapter_enumeration_failed",
                    os_code: None,
                })?;
            // SAFETY: buffer is aligned and writable for buffer_bytes bytes; API writes a linked
            // list wholly contained in it.
            let status = unsafe {
                GetAdaptersAddresses(
                    AF_UNSPEC as u32,
                    GAA_FLAG_INCLUDE_GATEWAYS
                        | GAA_FLAG_INCLUDE_PREFIX
                        | adapter_enumeration_flags(),
                    null(),
                    buffer.as_mut_ptr().cast(),
                    &mut buffer_bytes,
                )
            };
            if status == ERROR_BUFFER_OVERFLOW {
                size = buffer_bytes;
                continue;
            }
            if status != NO_ERROR {
                return Err(os_error("net:adapter_enumeration_failed", status));
            }
            let mut result = Vec::new();
            let mut current = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            while !current.is_null() {
                // SAFETY: current points into the API-owned buffer whose lifetime spans this loop.
                let row = unsafe { &*current };
                // SAFETY: union fields are initialized by GetAdaptersAddresses for this structure version.
                let if_index = unsafe { row.Anonymous1.Anonymous.IfIndex };
                // SAFETY: same initialized union invariant.
                let flags = unsafe { row.Anonymous2.Flags };
                // SAFETY: NET_LUID Value is the stable scalar representation initialized by the API.
                let luid = unsafe { row.Luid.Value };
                let mut guid = GUID::default();
                // SAFETY: row LUID is live and guid is writable.
                let guid_status = unsafe { ConvertInterfaceLuidToGuid(&row.Luid, &mut guid) };
                if guid_status != NO_ERROR {
                    return Err(os_error("net:adapter_identity_mismatch", guid_status));
                }
                let parsed_guid = guid_from_windows(guid);
                let mut roundtrip = NET_LUID_LH::default();
                // SAFETY: guid and roundtrip are valid pointers.
                let back_status = unsafe { ConvertInterfaceGuidToLuid(&guid, &mut roundtrip) };
                // SAFETY: both union values were initialized by conversion APIs.
                if back_status != NO_ERROR || unsafe { roundtrip.Value } != luid {
                    return Err(os_error("net:adapter_identity_mismatch", back_status));
                }
                result.push(RawAdapter {
                    guid: parsed_guid,
                    luid,
                    friendly_name: String::new(),
                    ipv4_index: if_index,
                    ipv6_index: row.Ipv6IfIndex,
                    oper_up: row.OperStatus == IfOperStatusUp,
                    dhcp_v4: flags & IP_ADAPTER_DHCP_ENABLED != 0,
                    dhcp_v6: row.Dhcpv6Iaid != 0,
                    unicast: sockaddr_list_unicast(row.FirstUnicastAddress),
                    gateways: sockaddr_list_gateway(row.FirstGatewayAddress),
                    dns_servers: sockaddr_list_dns(row.FirstDnsServerAddress),
                    ipv4_metric: row.Ipv4Metric,
                    ipv6_metric: row.Ipv6Metric,
                });
                current = row.Next;
            }
            return Ok(result);
        }
        Err(PlatformError {
            code: "net:adapter_enumeration_failed",
            os_code: None,
        })
    }

    fn routes() -> Result<Vec<RawRoute>, PlatformError> {
        let mut raw = null_mut();
        // SAFETY: output pointer is valid and receives an IP Helper allocation.
        let status = unsafe { GetIpForwardTable2(AF_UNSPEC, &mut raw) };
        if status != NO_ERROR {
            return Err(os_error("net:route_enumeration_failed", status));
        }
        let guard = MibTable(raw);
        if guard.0.is_null() {
            return Err(PlatformError {
                code: "net:route_enumeration_failed",
                os_code: None,
            });
        }
        // SAFETY: table pointer is live and NumEntries rows follow the header per API contract.
        let table = unsafe { &*guard.0 };
        // SAFETY: Table is a flexible array with exactly NumEntries initialized rows.
        let rows =
            unsafe { slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize) };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(destination) = sockaddr_inet(&row.DestinationPrefix.Prefix) else {
                continue;
            };
            let Some(next_hop) = sockaddr_inet(&row.NextHop) else {
                continue;
            };
            let family = match destination {
                IpAddr::V4(_) => AddressFamily::Ipv4,
                IpAddr::V6(_) => AddressFamily::Ipv6,
            };
            // SAFETY: LUID Value was initialized by GetIpForwardTable2.
            out.push(RawRoute {
                luid: unsafe { row.InterfaceLuid.Value },
                interface_index: row.InterfaceIndex,
                family,
                destination,
                prefix_len: row.DestinationPrefix.PrefixLength,
                next_hop,
                route_metric: row.Metric,
            });
        }
        Ok(out)
    }

    fn guid_from_windows(guid: GUID) -> AdapterGuid {
        let mut b = [0_u8; 16];
        b[..4].copy_from_slice(&guid.data1.to_be_bytes());
        b[4..6].copy_from_slice(&guid.data2.to_be_bytes());
        b[6..8].copy_from_slice(&guid.data3.to_be_bytes());
        b[8..].copy_from_slice(&guid.data4);
        AdapterGuid(b)
    }

    fn sockaddr(ptr: *const SOCKADDR) -> Option<IpAddr> {
        if ptr.is_null() {
            return None;
        } /* SAFETY: caller receives pointers from IP Helper lists with matching family layouts. */
        unsafe {
            match (*ptr).sa_family {
                AF_INET => {
                    let v = &*(ptr.cast::<SOCKADDR_IN>());
                    let b = v.sin_addr.S_un.S_un_b;
                    Some(IpAddr::V4(Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4)))
                }
                AF_INET6 => {
                    let v = &*(ptr.cast::<SOCKADDR_IN6>());
                    Some(IpAddr::V6(Ipv6Addr::from(v.sin6_addr.u.Byte)))
                }
                _ => None,
            }
        }
    }
    fn sockaddr_inet(
        value: &windows_sys::Win32::Networking::WinSock::SOCKADDR_INET,
    ) -> Option<IpAddr> {
        /* SAFETY: family discriminates the initialized union arm. */
        unsafe {
            match value.si_family {
                AF_INET => {
                    let v = value.Ipv4;
                    Some(IpAddr::V4(Ipv4Addr::new(
                        v.sin_addr.S_un.S_un_b.s_b1,
                        v.sin_addr.S_un.S_un_b.s_b2,
                        v.sin_addr.S_un.S_un_b.s_b3,
                        v.sin_addr.S_un.S_un_b.s_b4,
                    )))
                }
                AF_INET6 => Some(IpAddr::V6(Ipv6Addr::from(value.Ipv6.sin6_addr.u.Byte))),
                _ => None,
            }
        }
    }
    fn sockaddr_list_unicast(
        mut p: *mut windows_sys::Win32::NetworkManagement::IpHelper::IP_ADAPTER_UNICAST_ADDRESS_LH,
    ) -> Vec<IpAddr> {
        let mut v = vec![];
        while !p.is_null() {
            /* SAFETY: API-linked list remains inside live buffer. */
            let r = unsafe { &*p };
            if let Some(ip) = sockaddr(r.Address.lpSockaddr) {
                v.push(ip)
            }
            p = r.Next;
        }
        v
    }
    fn sockaddr_list_gateway(
        mut p: *mut windows_sys::Win32::NetworkManagement::IpHelper::IP_ADAPTER_GATEWAY_ADDRESS_LH,
    ) -> Vec<IpAddr> {
        let mut v = vec![];
        while !p.is_null() {
            /* SAFETY: API-linked list remains inside live buffer. */
            let r = unsafe { &*p };
            if let Some(ip) = sockaddr(r.Address.lpSockaddr) {
                v.push(ip)
            }
            p = r.Next;
        }
        v
    }
    fn sockaddr_list_dns(
        mut p: *mut windows_sys::Win32::NetworkManagement::IpHelper::IP_ADAPTER_DNS_SERVER_ADDRESS_XP,
    ) -> Vec<IpAddr> {
        let mut v = vec![];
        while !p.is_null() {
            /* SAFETY: API-linked list remains inside live buffer. */
            let r = unsafe { &*p };
            if let Some(ip) = sockaddr(r.Address.lpSockaddr) {
                v.push(ip)
            }
            p = r.Next;
        }
        v
    }
    fn os_error(code: &'static str, status: u32) -> PlatformError {
        PlatformError {
            code: if status == 5 {
                "net:permission_denied"
            } else {
                code
            },
            os_code: Some(status),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    pub(crate) fn fixture_identity(
        epoch: DeviceEpoch,
        luid: u64,
        v4_index: u32,
        source: &str,
    ) -> AdapterIdentity {
        AdapterIdentity {
            guid: "{8BDD4C57-901D-4A19-B85B-970F32B4C41A}".parse().unwrap(),
            luid,
            ipv4_index: Some(v4_index),
            ipv6_index: None,
            epoch,
            unicast_addresses: vec![source.parse().unwrap()],
        }
    }
    pub(crate) fn fixture_identity_v6(
        epoch: DeviceEpoch,
        luid: u64,
        v6_index: u32,
        source: &str,
    ) -> AdapterIdentity {
        AdapterIdentity {
            guid: "{8BDD4C57-901D-4A19-B85B-970F32B4C41A}".parse().unwrap(),
            luid,
            ipv4_index: None,
            ipv6_index: Some(v6_index),
            epoch,
            unicast_addresses: vec![source.parse().unwrap()],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;
    use dji4g_domain::DeviceEpoch;

    #[cfg(windows)]
    use windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2;

    const DJI_GUID: &str = "{8BDD4C57-901D-4A19-B85B-970F32B4C41A}";
    const OTHER_GUID: &str = "{11111111-2222-3333-4444-555555555555}";

    fn adapter(guid: &str, luid: u64, name: &str, v4_index: u32) -> RawAdapter {
        RawAdapter {
            guid: guid.parse().unwrap(),
            luid,
            friendly_name: name.to_owned(),
            ipv4_index: v4_index,
            ipv6_index: v4_index + 100,
            oper_up: true,
            dhcp_v4: true,
            dhcp_v6: false,
            unicast: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 225, 2))],
            gateways: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 225, 1))],
            dns_servers: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 225, 1))],
            ipv4_metric: 25,
            ipv6_metric: 35,
        }
    }

    fn default_route(luid: u64, index: u32, metric: u32) -> RawRoute {
        RawRoute {
            luid,
            interface_index: index,
            family: AddressFamily::Ipv4,
            destination: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            prefix_len: 0,
            next_hop: IpAddr::V4(Ipv4Addr::new(192, 168, 225, 1)),
            route_metric: metric,
        }
    }

    #[test]
    fn guid_not_friendly_name_selects_the_inventory_proven_adapter() {
        let rows = vec![
            adapter(OTHER_GUID, 22, "以太网 5", 7),
            adapter(DJI_GUID, 11, "任意重命名", 42),
        ];
        let routes = vec![default_route(11, 42, 5), default_route(22, 7, 1)];

        let resolved =
            resolve_observed_adapter(DeviceEpoch(9), &[DJI_GUID.to_owned()], &rows, &routes)
                .unwrap();

        assert_eq!(
            resolved.identity.guid_string(),
            DJI_GUID.to_ascii_lowercase()
        );
        assert_eq!(resolved.identity.luid(), 11);
        assert_eq!(resolved.identity.ipv4_index(), Some(42));
        assert_eq!(resolved.routes.len(), 1);
    }

    #[test]
    fn rediscovery_returns_the_current_ifindex_for_the_same_guid() {
        let first = resolve_observed_adapter(
            DeviceEpoch(3),
            &[DJI_GUID.to_owned()],
            &[adapter(DJI_GUID, 11, "RNDIS", 12)],
            &[default_route(11, 12, 7)],
        )
        .unwrap();
        let second = resolve_observed_adapter(
            DeviceEpoch(4),
            &[DJI_GUID.to_owned()],
            &[adapter(DJI_GUID, 11, "RNDIS #2", 77)],
            &[default_route(11, 77, 7)],
        )
        .unwrap();

        assert_eq!(first.identity.ipv4_index(), Some(12));
        assert_eq!(second.identity.ipv4_index(), Some(77));
        assert_eq!(second.identity.epoch(), DeviceEpoch(4));
    }

    #[test]
    fn malformed_missing_and_multiple_netcfg_ids_fail_closed() {
        let rows = [adapter(DJI_GUID, 11, "RNDIS", 42)];
        let routes = [default_route(11, 42, 1)];
        assert_eq!(
            resolve_observed_adapter(DeviceEpoch(1), &[], &rows, &routes)
                .unwrap_err()
                .code,
            "net:netcfg_id_missing"
        );
        assert_eq!(
            resolve_observed_adapter(DeviceEpoch(1), &["not-a-guid".into()], &rows, &routes)
                .unwrap_err()
                .code,
            "net:netcfg_id_invalid"
        );
        assert_eq!(
            resolve_observed_adapter(
                DeviceEpoch(1),
                &[DJI_GUID.into(), OTHER_GUID.into()],
                &rows,
                &routes,
            )
            .unwrap_err()
            .code,
            "net:adapter_ambiguous"
        );
        assert_eq!(
            resolve_observed_adapter(
                DeviceEpoch(1),
                &[DJI_GUID.into(), DJI_GUID.to_ascii_lowercase()],
                &rows,
                &routes,
            )
            .unwrap_err()
            .code,
            "net:adapter_identity_mismatch"
        );
    }

    #[test]
    fn every_target_net_candidate_must_have_a_netcfg_identity() {
        let candidates = [Some(DJI_GUID), None];
        assert_eq!(
            checked_netcfg_ids(candidates.into_iter()).unwrap_err().code,
            "net:netcfg_id_missing"
        );
    }

    #[test]
    fn adapter_enumeration_requests_all_ndis_interfaces() {
        assert_ne!(adapter_enumeration_flags() & GAA_INCLUDE_ALL_INTERFACES, 0);
    }

    #[test]
    fn down_or_family_unbound_adapter_still_maps_by_guid_without_becoming_usable() {
        let mut row = adapter(DJI_GUID, 11, "RNDIS", 0);
        row.ipv6_index = 0;
        row.oper_up = false;
        row.unicast.clear();
        let resolved =
            resolve_observed_adapter(DeviceEpoch(20), &[DJI_GUID.into()], &[row], &[]).unwrap();

        assert_eq!(resolved.identity.luid(), 11);
        assert_eq!(resolved.identity.ipv4_index(), None);
        assert_eq!(resolved.identity.ipv6_index(), None);
        assert!(resolved.usable_families.is_empty());
    }

    #[test]
    fn apipa_link_local_and_down_adapters_map_but_are_not_usable() {
        let mut row = adapter(DJI_GUID, 11, "RNDIS", 42);
        row.unicast = vec![
            IpAddr::V4(Ipv4Addr::new(169, 254, 1, 9)),
            IpAddr::V6("fe80::1".parse::<Ipv6Addr>().unwrap()),
        ];
        let resolved = resolve_observed_adapter(
            DeviceEpoch(1),
            &[DJI_GUID.into()],
            &[row.clone()],
            &[default_route(11, 42, 1)],
        )
        .unwrap();
        assert!(resolved.usable_families.is_empty());

        row.oper_up = false;
        row.unicast = vec![IpAddr::V4(Ipv4Addr::new(192, 168, 225, 2))];
        let down = resolve_observed_adapter(
            DeviceEpoch(2),
            &[DJI_GUID.into()],
            &[row],
            &[default_route(11, 42, 1)],
        )
        .unwrap();
        assert!(down.usable_families.is_empty());
    }

    #[test]
    fn route_metric_addition_saturates_and_non_target_routes_are_excluded() {
        let mut row = adapter(DJI_GUID, 11, "RNDIS", 42);
        row.ipv4_metric = u32::MAX;
        let result = resolve_observed_adapter(
            DeviceEpoch(1),
            &[DJI_GUID.into()],
            &[row],
            &[default_route(11, 42, 10), default_route(999, 9, 0)],
        )
        .unwrap();
        assert_eq!(result.routes.len(), 1);
        assert_eq!(result.routes[0].total_metric, u32::MAX);
    }

    #[test]
    fn zero_luid_interface_metrics_are_rejected() {
        let error = read_interface_metrics(0).unwrap_err();

        assert_eq!(error.code, "net:interface_luid_invalid");
        assert_eq!(error.os_code, None);
    }

    #[cfg(windows)]
    #[test]
    fn interface_metrics_map_every_mib_row_field() {
        let row = MIB_IF_ROW2 {
            InOctets: 11,
            OutOctets: 22,
            InErrors: 33,
            OutErrors: 44,
            InDiscards: 55,
            OutDiscards: 66,
            ReceiveLinkSpeed: 1_000_000_000,
            TransmitLinkSpeed: 2_000_000_000,
            ..MIB_IF_ROW2::default()
        };

        assert_eq!(
            native::metrics_from_row(&row),
            InterfaceMetrics {
                rx_bytes: 11,
                tx_bytes: 22,
                in_errors: 33,
                out_errors: 44,
                in_discards: 55,
                out_discards: 66,
                link_rx_bits_per_second: 1_000_000_000,
                link_tx_bits_per_second: 2_000_000_000,
            }
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_interface_metrics_are_unsupported() {
        let error = read_interface_metrics(7).unwrap_err();

        assert_eq!(error.code, "net:unsupported_platform");
        assert_eq!(error.os_code, None);
    }
}
