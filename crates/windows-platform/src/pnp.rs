use std::fmt;

const TARGET_VID: u16 = 0x2CA3;
const TARGET_PID: u16 = 0x4006;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UsbIdentity {
    vid: u16,
    pid: u16,
}

fn parse_usb_identity(value: &str) -> Option<UsbIdentity> {
    let mut tokens = value.split(['\\', '&']);
    if !tokens.next()?.eq_ignore_ascii_case("USB") {
        return None;
    }

    let mut vid = None;
    let mut pid = None;
    for token in tokens {
        let upper = token.to_ascii_uppercase();
        if let Some(raw) = upper.strip_prefix("VID_") {
            if raw.len() != 4 || vid.is_some() {
                return None;
            }
            vid = u16::from_str_radix(raw, 16).ok();
            vid?;
        } else if let Some(raw) = upper.strip_prefix("PID_") {
            if raw.len() != 4 || pid.is_some() {
                return None;
            }
            pid = u16::from_str_radix(raw, 16).ok();
            pid?;
        }
    }
    Some(UsbIdentity {
        vid: vid?,
        pid: pid?,
    })
}

fn is_exact_target_identity(value: &str) -> bool {
    parse_usb_identity(value)
        == Some(UsbIdentity {
            vid: TARGET_VID,
            pid: TARGET_PID,
        })
}

fn has_usb_interface_token(value: &str) -> bool {
    value.split(['\\', '&']).any(|token| {
        token
            .to_ascii_uppercase()
            .strip_prefix("MI_")
            .is_some_and(|raw| raw.len() == 2 && raw.bytes().all(|byte| byte.is_ascii_hexdigit()))
    })
}

fn is_proven_root(node: &PnpNode) -> bool {
    is_exact_target_identity(&node.instance_id)
        && !has_usb_interface_token(&node.instance_id)
        && !node.hardware_ids.is_empty()
        && node
            .hardware_ids
            .iter()
            .all(|id| is_exact_target_identity(id) && !has_usb_interface_token(id))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FunctionRole {
    DedicatedAt,
    Modem,
    DmDiag,
    Nmea,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PnpNode {
    instance_id: String,
    hardware_ids: Vec<String>,
    ancestry: Vec<String>,
    container_id: Option<String>,
    problem_code: Option<u32>,
    native_devnode: Option<u32>,
    port_name: Option<String>,
    com_interface_path: Option<String>,
    net_interface_path: Option<String>,
    net_cfg_instance_id: Option<String>,
    role: FunctionRole,
    verified_modem: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A read-only summary of an inventory-owned COM function.
///
/// ```compile_fail
/// use dji4g_windows_platform::{ComCandidate, FunctionRole};
/// let _forged = ComCandidate {
///     port_name: "COM1".to_owned(),
///     interface_path: r"\\?\USB#forged".to_owned(),
///     container_id: None,
///     instance_id: r"USB\VID_2CA3&PID_4006&MI_02\AT".to_owned(),
///     hardware_ids: vec![],
///     ancestry: vec![r"USB\VID_2CA3&PID_4006\ROOT".to_owned()],
///     role: FunctionRole::DedicatedAt,
///     verified_modem: false,
///     problem_code: None,
/// };
/// ```
pub struct ComCandidate {
    port_name: String,
    interface_path: String,
    container_id: Option<String>,
    instance_id: String,
    hardware_ids: Vec<String>,
    ancestry: Vec<String>,
    role: FunctionRole,
    verified_modem: bool,
    problem_code: Option<u32>,
}

impl ComCandidate {
    #[must_use]
    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    #[must_use]
    pub fn interface_path(&self) -> &str {
        &self.interface_path
    }

    #[must_use]
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref()
    }

    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    #[must_use]
    pub const fn role(&self) -> FunctionRole {
        self.role
    }

    #[must_use]
    pub const fn problem_code(&self) -> Option<u32> {
        self.problem_code
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetCandidate {
    interface_path: String,
    net_cfg_instance_id: Option<String>,
    container_id: Option<String>,
    instance_id: String,
    hardware_ids: Vec<String>,
    ancestry: Vec<String>,
    problem_code: Option<u32>,
    native_devnode: Option<u32>,
}

impl NetCandidate {
    #[must_use]
    pub fn interface_path(&self) -> &str {
        &self.interface_path
    }

    #[must_use]
    pub fn net_cfg_instance_id(&self) -> Option<&str> {
        self.net_cfg_instance_id.as_deref()
    }

    #[must_use]
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref()
    }

    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    #[must_use]
    pub const fn problem_code(&self) -> Option<u32> {
        self.problem_code
    }

    /// Returns the inventory-owned devnode token for the current Windows scan.  The token is
    /// crate-private so callers cannot direct a state change at an arbitrary devnode.
    #[cfg(windows)]
    pub(crate) const fn native_devnode(&self) -> Option<u32> {
        self.native_devnode
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DjiDevice {
    root_instance_id: String,
    container_id: Option<String>,
    problem_code: Option<u32>,
    com_candidates: Vec<ComCandidate>,
    net_candidates: Vec<NetCandidate>,
    native_root_devnode: Option<u32>,
}

impl DjiDevice {
    #[must_use]
    pub fn root_instance_id(&self) -> &str {
        &self.root_instance_id
    }

    #[must_use]
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref()
    }

    #[must_use]
    pub const fn problem_code(&self) -> Option<u32> {
        self.problem_code
    }

    #[must_use]
    pub fn com_candidates(&self) -> &[ComCandidate] {
        &self.com_candidates
    }

    #[must_use]
    pub fn net_candidates(&self) -> &[NetCandidate] {
        &self.net_candidates
    }

    pub fn select_at_port(&self) -> Result<SelectedPort, PortSelectionError> {
        select_at_candidate(&self.com_candidates)
    }

    /// Returns the inventory-owned root devnode token for the current Windows scan.  This is
    /// crate-private; re-enumeration must first revalidate the exact VID/PID root and ancestry.
    #[cfg(windows)]
    pub(crate) const fn native_root_devnode(&self) -> Option<u32> {
        self.native_root_devnode
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// A snapshot produced only by platform enumeration/correlation.
///
/// Raw proof selection is intentionally not part of the public API.
///
/// ```compile_fail
/// use dji4g_windows_platform::select_at_candidate;
/// ```
pub struct InventorySnapshot {
    devices: Vec<DjiDevice>,
}

impl InventorySnapshot {
    #[must_use]
    pub fn devices(&self) -> &[DjiDevice] {
        &self.devices
    }
}

#[must_use]
fn correlate_topology(nodes: Vec<PnpNode>) -> InventorySnapshot {
    use std::collections::{BTreeMap, HashMap};

    let proven_roots: HashMap<_, _> = nodes
        .iter()
        .filter(|node| is_proven_root(node))
        .map(|node| (node.instance_id.to_ascii_uppercase(), node))
        .collect();
    let mut devices: BTreeMap<String, DjiDevice> = proven_roots
        .values()
        .map(|root| {
            (
                root.instance_id.clone(),
                DjiDevice {
                    root_instance_id: root.instance_id.clone(),
                    container_id: root.container_id.clone(),
                    problem_code: root.problem_code,
                    com_candidates: Vec::new(),
                    net_candidates: Vec::new(),
                    native_root_devnode: root.native_devnode,
                },
            )
        })
        .collect();
    for node in &nodes {
        if is_proven_root(node) {
            continue;
        }
        let Some(root_instance_id) = node
            .ancestry
            .iter()
            .rev()
            .find(|value| {
                !value.eq_ignore_ascii_case(&node.instance_id)
                    && proven_roots.contains_key(&value.to_ascii_uppercase())
            })
            .cloned()
        else {
            continue;
        };
        let root = proven_roots
            .get(&root_instance_id.to_ascii_uppercase())
            .expect("root was resolved from proven_roots");
        if root
            .container_id
            .as_deref()
            .zip(node.container_id.as_deref())
            .is_some_and(|(root_id, child_id)| !root_id.eq_ignore_ascii_case(child_id))
        {
            continue;
        }
        let device = devices
            .get_mut(&root.instance_id)
            .expect("every proven root has a device entry");
        if let Some(port_name) = &node.port_name {
            device.com_candidates.push(ComCandidate {
                port_name: port_name.clone(),
                interface_path: node.com_interface_path.clone().unwrap_or_default(),
                container_id: node.container_id.clone(),
                instance_id: node.instance_id.clone(),
                hardware_ids: node.hardware_ids.clone(),
                ancestry: node.ancestry.clone(),
                role: node.role,
                verified_modem: node.verified_modem,
                problem_code: node.problem_code,
            });
        }
        if let Some(interface_path) = &node.net_interface_path {
            device.net_candidates.push(NetCandidate {
                net_cfg_instance_id: node.net_cfg_instance_id.clone(),
                interface_path: interface_path.clone(),
                container_id: node.container_id.clone(),
                instance_id: node.instance_id.clone(),
                hardware_ids: node.hardware_ids.clone(),
                ancestry: node.ancestry.clone(),
                problem_code: node.problem_code,
                native_devnode: node.native_devnode,
            });
        }
    }
    InventorySnapshot {
        devices: devices.into_values().collect(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectedPortKind {
    DedicatedAt,
    VerifiedModem,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A port that passed topology and role selection.
///
/// Fields are intentionally private so serial ownership can only start from
/// [`select_at_candidate`], never from a caller-forged DM/NMEA or COM value.
///
/// ```compile_fail
/// use dji4g_windows_platform::{SelectedPort, SelectedPortKind};
/// let _forged = SelectedPort {
///     port_name: "COM1".to_owned(),
///     interface_path: String::new(),
///     container_id: None,
///     problem_code: None,
///     kind: SelectedPortKind::DedicatedAt,
/// };
/// ```
pub struct SelectedPort {
    port_name: String,
    interface_path: String,
    container_id: Option<String>,
    problem_code: Option<u32>,
    kind: SelectedPortKind,
}

impl SelectedPort {
    #[must_use]
    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    #[must_use]
    pub fn interface_path(&self) -> &str {
        &self.interface_path
    }

    #[must_use]
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref()
    }

    #[must_use]
    pub const fn problem_code(&self) -> Option<u32> {
        self.problem_code
    }

    #[must_use]
    pub const fn kind(&self) -> SelectedPortKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortSelectionError {
    NoSafePort,
    AmbiguousPort { count: usize },
}

impl fmt::Display for PortSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoSafePort => "pnp:no_safe_at_port",
            Self::AmbiguousPort { .. } => "pnp:ambiguous_at_port",
        })
    }
}

impl std::error::Error for PortSelectionError {}

fn select_at_candidate(candidates: &[ComCandidate]) -> Result<SelectedPort, PortSelectionError> {
    let dedicated: Vec<_> = candidates
        .iter()
        .filter(|candidate| is_target(candidate) && candidate.role == FunctionRole::DedicatedAt)
        .collect();
    if !dedicated.is_empty() {
        return select_unique(&dedicated, SelectedPortKind::DedicatedAt);
    }

    let modem: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            is_target(candidate)
                && candidate.role == FunctionRole::Modem
                && candidate.verified_modem
        })
        .collect();
    if !modem.is_empty() {
        return select_unique(&modem, SelectedPortKind::VerifiedModem);
    }

    // Last resort: every candidate here is already scoped to the proven exact-identity root
    // (container + ancestry correlation), so a single unclassified COM function is trusted as
    // the AT port — the AT session actor still verifies the port actually speaks AT before any
    // use. With several unclassified ports the roles are genuinely ambiguous and none is tried.
    let residue: Vec<_> = candidates
        .iter()
        .filter(|candidate| is_target(candidate) && candidate.role == FunctionRole::Unknown)
        .collect();
    select_unique(&residue, SelectedPortKind::DedicatedAt)
}

/// How a selected AT port earned trust.  `RoleClassified` ports were named by their USB
/// descriptors; `HandshakeVerified` ports proved they speak the AT protocol through the bounded
/// read-only handshake after generic driver names left every candidate unclassified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortProvenance {
    RoleClassified,
    HandshakeVerified,
}

/// The result of [`select_at_port_verified`]: either an already-trusted port, or the
/// deterministic, bounded candidate list the caller must handshake-probe in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AtPortSelection {
    Classified(SelectedPort),
    HandshakeCandidates(Vec<SelectedPort>),
}

/// Classified tiers first (identical to [`select_at_candidate`], including the single-residue
/// rule); then, when generic driver names leave several unclassified candidates, hand the caller
/// a deterministic probe list instead of refusing outright.  The pool never widens: every
/// candidate is still scoped to the proven exact-identity root, and probing itself is the
/// caller's bounded read-only handshake (`serial::probe_at_port`).  Candidates whose interface
/// path already failed the handshake this epoch are skipped; when nothing remains within the
/// budget the ambiguity error stands.
pub fn select_at_port_verified(
    candidates: &[ComCandidate],
    already_failed: &[String],
    budget: usize,
) -> Result<AtPortSelection, PortSelectionError> {
    let dedicated: Vec<_> = candidates
        .iter()
        .filter(|candidate| is_target(candidate) && candidate.role == FunctionRole::DedicatedAt)
        .collect();
    let modem: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            is_target(candidate)
                && candidate.role == FunctionRole::Modem
                && candidate.verified_modem
        })
        .collect();

    // A single classified candidate is trusted outright.  SEVERAL classified candidates (real
    // hardware exposes two "AT Port"-named functions) are exactly the case the handshake was
    // built to disambiguate: hand them to the caller for probing instead of refusing.
    if let [only] = dedicated.as_slice() {
        return Ok(AtPortSelection::Classified(select_unique(
            std::slice::from_ref(only),
            SelectedPortKind::DedicatedAt,
        )?));
    }
    if let [only] = modem.as_slice() {
        return Ok(AtPortSelection::Classified(select_unique(
            std::slice::from_ref(only),
            SelectedPortKind::VerifiedModem,
        )?));
    }

    let residue: Vec<_> = candidates
        .iter()
        .filter(|candidate| is_target(candidate) && candidate.role == FunctionRole::Unknown)
        .collect();
    // The single-residue rule is a classification, but a residue that already failed the
    // handshake this epoch is known-bad, not trusted: it must fall through to the ambiguity
    // error instead of being re-selected every cycle.
    if let [only] = residue.as_slice() {
        let already_failed_residue = already_failed
            .iter()
            .any(|failed| failed == &only.interface_path);
        if !already_failed_residue {
            return Ok(AtPortSelection::Classified(select_unique(
                std::slice::from_ref(only),
                SelectedPortKind::DedicatedAt,
            )?));
        }
    }

    // Probe order: classified AT ports first (best prior), then verified modems, then
    // unclassified residue.  Within a tier the order is deterministic by interface path — COM
    // numbers renumber across replugs, the USB interface path does not.
    let mut ordered: Vec<&ComCandidate> = Vec::new();
    ordered.extend(dedicated);
    ordered.extend(modem);
    ordered.extend(residue);
    ordered.sort_by(|left, right| left.interface_path.cmp(&right.interface_path));
    let probed: Vec<SelectedPort> = ordered
        .iter()
        .filter(|candidate| {
            !already_failed
                .iter()
                .any(|failed| failed == &candidate.interface_path)
        })
        .take(budget)
        .map(|candidate| SelectedPort {
            port_name: candidate.port_name.clone(),
            interface_path: candidate.interface_path.clone(),
            container_id: candidate.container_id.clone(),
            problem_code: candidate.problem_code,
            kind: SelectedPortKind::DedicatedAt,
        })
        .collect();
    if probed.is_empty() {
        return Err(PortSelectionError::AmbiguousPort {
            count: ordered.len(),
        });
    }
    Ok(AtPortSelection::HandshakeCandidates(probed))
}

fn is_target(candidate: &ComCandidate) -> bool {
    !candidate.interface_path.is_empty()
        && candidate
            .ancestry
            .iter()
            .any(|value| is_exact_target_identity(value) && !has_usb_interface_token(value))
}

#[cfg(test)]
pub(crate) fn selected_test_port() -> SelectedPort {
    SelectedPort {
        port_name: "COM22".to_owned(),
        interface_path: r"\\?\USB#fixture#at".to_owned(),
        container_id: Some("{fixture-container}".to_owned()),
        problem_code: None,
        kind: SelectedPortKind::DedicatedAt,
    }
}

fn select_unique(
    candidates: &[&ComCandidate],
    kind: SelectedPortKind,
) -> Result<SelectedPort, PortSelectionError> {
    match candidates {
        [] => Err(PortSelectionError::NoSafePort),
        [candidate] => Ok(SelectedPort {
            port_name: candidate.port_name.clone(),
            interface_path: candidate.interface_path.clone(),
            container_id: candidate.container_id.clone(),
            problem_code: candidate.problem_code,
            kind,
        }),
        many => Err(PortSelectionError::AmbiguousPort { count: many.len() }),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformError {
    pub code: &'static str,
    pub os_code: Option<u32>,
}

pub type InventoryError = PlatformError;

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for PlatformError {}

pub type InventoryResult<T> = std::result::Result<T, PlatformError>;

#[allow(async_fn_in_trait)]
pub trait DeviceInventory {
    async fn scan(&self) -> InventoryResult<InventorySnapshot>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsDeviceInventory;

impl WindowsDeviceInventory {
    pub fn scan_now(&self) -> InventoryResult<InventorySnapshot> {
        scan_platform()
    }
}

impl DeviceInventory for WindowsDeviceInventory {
    async fn scan(&self) -> InventoryResult<InventorySnapshot> {
        self.scan_now()
    }
}

#[cfg(windows)]
fn scan_platform() -> InventoryResult<InventorySnapshot> {
    native::scan()
}

/// Result of one exact NET devnode disable/enable transaction.  The repair executor maps the
/// variants to stable Failed/rollback outcomes without exposing ConfigMgr status text.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetDevnodeRestart {
    Applied,
    DisableFailed { os_code: u32 },
    EnableFailed { os_code: u32, recovery: bool },
}

/// Re-enumerate the root devnode selected by a fresh inventory snapshot.  This wrapper is kept
/// inside the platform crate so callers cannot pass an arbitrary instance id or devnode handle.
#[cfg(windows)]
pub(crate) fn reenumerate_exact_dji(device: &DjiDevice) -> InventoryResult<()> {
    native::reenumerate_exact_dji(device)
}

/// Restart the NET devnode selected by a fresh inventory snapshot.  The implementation verifies
/// the stored devnode's current instance id before every ConfigMgr call.
#[cfg(windows)]
pub(crate) fn restart_exact_net(device: &DjiDevice) -> InventoryResult<NetDevnodeRestart> {
    native::restart_exact_net(device)
}

#[cfg(not(windows))]
fn scan_platform() -> InventoryResult<InventorySnapshot> {
    Err(PlatformError {
        code: "pnp:unsupported_platform",
        os_code: None,
    })
}

#[cfg(windows)]
mod native {
    use std::{
        collections::HashMap,
        ffi::c_void,
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
    };

    use super::{
        DjiDevice, FunctionRole, InventoryResult, InventorySnapshot, NetDevnodeRestart,
        PlatformError, PnpNode, correlate_topology,
    };

    type Bool = i32;
    type Dword = u32;
    type Devinst = u32;
    type Hdevinfo = *mut c_void;
    type Hkey = *mut c_void;

    const DIGCF_PRESENT: Dword = 0x0000_0002;
    const DIGCF_ALLCLASSES: Dword = 0x0000_0004;
    const DIGCF_DEVICEINTERFACE: Dword = 0x0000_0010;
    const ERROR_NO_MORE_ITEMS: i32 = 259;
    const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_INVALID_DATA: i32 = 13;
    const ERROR_MORE_DATA: i32 = 234;
    const ERROR_NOT_FOUND: i32 = 1168;
    const ERROR_KEY_DOES_NOT_EXIST: i32 = 0xE000_0204_u32 as i32;
    const DEVPROP_TYPE_UINT32: Dword = 0x0000_0007;
    const DEVPROP_TYPE_GUID: Dword = 0x0000_000D;
    const DEVPROP_TYPE_STRING: Dword = 0x0000_0012;
    const DEVPROP_TYPE_STRING_LIST: Dword = 0x0000_2012;
    const CR_SUCCESS: Dword = 0;
    const CR_NO_SUCH_DEVNODE: Dword = 0x0000_000D;
    const CR_ACCESS_DENIED: Dword = 0x0000_0033;
    const CM_REENUMERATE_SYNCHRONOUS: Dword = 0x0000_0001;
    const CM_LOCATE_DEVNODE_NORMAL: Dword = 0;
    const MAX_DEVICE_ID_LEN: usize = 200;
    const MAX_ANCESTRY_DEPTH: usize = 32;
    const MAX_PROPERTY_ATTEMPTS: usize = 4;
    const MAX_REGISTRY_STRING_BYTES: usize = 64 * 1024;

    fn checked_registry_capacity(size: usize) -> InventoryResult<usize> {
        if size == 0 || size > MAX_REGISTRY_STRING_BYTES || size % 2 != 0 {
            return Err(PlatformError {
                code: "pnp:registry_buffer_failed",
                os_code: None,
            });
        }
        Ok(size)
    }

    fn checked_registry_growth(current: usize, requested: usize) -> InventoryResult<usize> {
        if requested <= current {
            return Err(PlatformError {
                code: "pnp:registry_buffer_failed",
                os_code: None,
            });
        }
        checked_registry_capacity(requested)
    }
    const DICS_FLAG_GLOBAL: Dword = 1;
    const DIREG_DEV: Dword = 1;
    const DIREG_DRV: Dword = 2;
    const KEY_QUERY_VALUE: Dword = 1;
    const REG_SZ: Dword = 1;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    struct Devpropkey {
        fmtid: Guid,
        pid: Dword,
    }

    #[repr(C)]
    struct SpDevinfoData {
        cb_size: Dword,
        class_guid: Guid,
        dev_inst: Devinst,
        reserved: usize,
    }

    #[repr(C)]
    struct SpDeviceInterfaceData {
        cb_size: Dword,
        interface_class_guid: Guid,
        flags: Dword,
        reserved: usize,
    }

    const DEVICE_PROPERTY_FMTID: Guid = Guid {
        data1: 0xa45c254e,
        data2: 0xdf1c,
        data3: 0x4efd,
        data4: [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    };
    const DEVICE_STATUS_FMTID: Guid = Guid {
        data1: 0x4340a6c5,
        data2: 0x93fa,
        data3: 0x4706,
        data4: [0x97, 0x2c, 0x7b, 0x64, 0x80, 0x08, 0xa5, 0xa7],
    };
    const CONTAINER_FMTID: Guid = Guid {
        data1: 0x8c7ed206,
        data2: 0x3f8a,
        data3: 0x4827,
        data4: [0xb3, 0xab, 0xae, 0x9e, 0x1f, 0xae, 0xfc, 0x6c],
    };
    const DEVPKEY_DEVICE_HARDWARE_IDS: Devpropkey = Devpropkey {
        fmtid: DEVICE_PROPERTY_FMTID,
        pid: 3,
    };
    const DEVPKEY_DEVICE_CLASS: Devpropkey = Devpropkey {
        fmtid: DEVICE_PROPERTY_FMTID,
        pid: 9,
    };
    const DEVPKEY_DEVICE_FRIENDLY_NAME: Devpropkey = Devpropkey {
        fmtid: DEVICE_PROPERTY_FMTID,
        pid: 14,
    };
    const DEVPKEY_DEVICE_INSTANCE_ID: Devpropkey = Devpropkey {
        fmtid: DEVICE_PROPERTY_FMTID,
        pid: 256,
    };
    const DEVPKEY_DEVICE_BUS_REPORTED_DESC: Devpropkey = Devpropkey {
        fmtid: Guid {
            data1: 0x540b947e,
            data2: 0x8b40,
            data3: 0x45bc,
            data4: [0xa8, 0xa2, 0x6a, 0x0b, 0x89, 0x4c, 0xbd, 0xa2],
        },
        pid: 4,
    };
    const DEVPKEY_DEVICE_PROBLEM_CODE: Devpropkey = Devpropkey {
        fmtid: DEVICE_STATUS_FMTID,
        pid: 3,
    };
    const DEVPKEY_DEVICE_CONTAINER_ID: Devpropkey = Devpropkey {
        fmtid: CONTAINER_FMTID,
        pid: 2,
    };
    const GUID_DEVINTERFACE_COMPORT: Guid = Guid {
        data1: 0x86e0d1e0,
        data2: 0x8089,
        data3: 0x11d0,
        data4: [0x9c, 0xe4, 0x08, 0x00, 0x3e, 0x30, 0x1f, 0x73],
    };
    const GUID_DEVINTERFACE_NET: Guid = Guid {
        data1: 0xcac88484,
        data2: 0x7515,
        data3: 0x4c03,
        data4: [0x82, 0xe6, 0x71, 0xa8, 0x7a, 0xba, 0xc3, 0x61],
    };

    #[derive(Debug)]
    struct DeviceRecord {
        dev_inst: Devinst,
        instance_id: String,
        hardware_ids: Vec<String>,
        ancestry: Vec<String>,
        container_id: Option<String>,
        class: Option<String>,
        friendly_name: Option<String>,
        bus_reported_desc: Option<String>,
        problem_code: Option<u32>,
        port_name: Option<String>,
        net_cfg_instance_id: Option<String>,
    }

    struct PropertyValue {
        property_type: Dword,
        bytes: Vec<u8>,
    }

    struct DeviceInfoSet(Hdevinfo);

    impl Drop for DeviceInfoSet {
        fn drop(&mut self) {
            // SAFETY: `self.0` is the still-owned handle returned by SetupDiGetClassDevsW.
            unsafe { SetupDiDestroyDeviceInfoList(self.0) };
        }
    }

    #[derive(Clone, Copy)]
    enum OptionalWin32Api {
        DeviceProperty,
        RegistryKey,
        RegistryValue,
    }

    fn classify_optional_win32(api: OptionalWin32Api, status: i32) -> InventoryResult<Option<()>> {
        if status == 0 {
            return Ok(Some(()));
        }
        let absent = match api {
            OptionalWin32Api::DeviceProperty => status == ERROR_NOT_FOUND,
            OptionalWin32Api::RegistryKey => {
                status == ERROR_FILE_NOT_FOUND || status == ERROR_KEY_DOES_NOT_EXIST
            }
            OptionalWin32Api::RegistryValue => status == ERROR_FILE_NOT_FOUND,
        };
        if absent {
            return Ok(None);
        }
        Err(os_error(
            if status == ERROR_ACCESS_DENIED {
                "pnp:permission_denied"
            } else {
                match api {
                    OptionalWin32Api::DeviceProperty if status == ERROR_INVALID_DATA => {
                        "pnp:property_invalid"
                    }
                    OptionalWin32Api::DeviceProperty => "pnp:property_failed",
                    OptionalWin32Api::RegistryKey | OptionalWin32Api::RegistryValue => {
                        "pnp:registry_failed"
                    }
                }
            },
            status,
        ))
    }

    fn classify_parent_status(status: Dword) -> InventoryResult<bool> {
        match status {
            CR_SUCCESS => Ok(true),
            CR_NO_SUCH_DEVNODE => Ok(false),
            CR_ACCESS_DENIED => Err(PlatformError {
                code: "pnp:permission_denied",
                os_code: Some(status),
            }),
            _ => Err(PlatformError {
                code: "pnp:cm_parent_failed",
                os_code: Some(status),
            }),
        }
    }

    fn interface_error(status: i32, operation_code: &'static str) -> PlatformError {
        os_error(
            if status == ERROR_ACCESS_DENIED {
                "pnp:permission_denied"
            } else {
                operation_code
            },
            status,
        )
    }

    pub(super) fn scan() -> InventoryResult<InventorySnapshot> {
        let com_paths = enumerate_interface_paths(&GUID_DEVINTERFACE_COMPORT)?;
        let net_paths = enumerate_interface_paths(&GUID_DEVINTERFACE_NET)?;
        let records = enumerate_devices(&net_paths)?;
        Ok(build_snapshot(records, &com_paths, &net_paths))
    }

    pub(super) fn reenumerate_exact_dji(device: &DjiDevice) -> InventoryResult<()> {
        let devnode = device.native_root_devnode().ok_or(PlatformError {
            code: "pnp:root_devnode_missing",
            os_code: None,
        })?;
        verify_devnode_id(
            devnode,
            device.root_instance_id(),
            "pnp:root_identity_changed",
        )?;
        let status = unsafe { CM_Reenumerate_DevNode(devnode, CM_REENUMERATE_SYNCHRONOUS) };
        match status {
            CR_SUCCESS => Ok(()),
            CR_NO_SUCH_DEVNODE => Err(PlatformError {
                code: "pnp:target_removed",
                os_code: Some(status),
            }),
            CR_ACCESS_DENIED => Err(PlatformError {
                code: "pnp:permission_denied",
                os_code: Some(status),
            }),
            _ => Err(PlatformError {
                code: "pnp:reenumeration_failed",
                os_code: Some(status),
            }),
        }
    }

    pub(super) fn restart_exact_net(device: &DjiDevice) -> InventoryResult<NetDevnodeRestart> {
        let root_devnode = device.native_root_devnode().ok_or(PlatformError {
            code: "pnp:root_devnode_missing",
            os_code: None,
        })?;
        verify_devnode_id(
            root_devnode,
            device.root_instance_id(),
            "pnp:root_identity_changed",
        )?;
        let candidates: Vec<_> = device
            .net_candidates()
            .iter()
            .filter_map(|candidate| candidate.native_devnode())
            .collect();
        let [devnode] = candidates.as_slice() else {
            return Err(PlatformError {
                code: "pnp:net_devnode_ambiguous",
                os_code: None,
            });
        };
        let candidate = device
            .net_candidates()
            .iter()
            .find(|candidate| candidate.native_devnode() == Some(*devnode))
            .ok_or(PlatformError {
                code: "pnp:net_devnode_missing",
                os_code: None,
            })?;
        verify_devnode_id(
            *devnode,
            candidate.instance_id(),
            "pnp:net_identity_changed",
        )?;

        verify_devnode_id(
            root_devnode,
            device.root_instance_id(),
            "pnp:root_identity_changed",
        )?;
        let disabled = unsafe { CM_Disable_DevNode(*devnode, 0) };
        if disabled != CR_SUCCESS {
            return Ok(NetDevnodeRestart::DisableFailed { os_code: disabled });
        }
        if verify_devnode_id(
            root_devnode,
            device.root_instance_id(),
            "pnp:root_identity_changed",
        )
        .is_err()
            || verify_devnode_id(
                *devnode,
                candidate.instance_id(),
                "pnp:net_identity_changed",
            )
            .is_err()
        {
            return Ok(NetDevnodeRestart::EnableFailed {
                os_code: CR_NO_SUCH_DEVNODE,
                recovery: false,
            });
        }
        let enabled = unsafe { CM_Enable_DevNode(*devnode, 0) };
        if enabled == CR_SUCCESS {
            return Ok(NetDevnodeRestart::Applied);
        }
        // A disable can leave the adapter down even when the first enable call reports a
        // transient ConfigMgr error.  One explicit recovery attempt is part of this transaction;
        // it is not an automatic retry of the user's requested operation.
        let recovery = verify_devnode_id(
            root_devnode,
            device.root_instance_id(),
            "pnp:root_identity_changed",
        )
        .is_ok()
            && verify_devnode_id(
                *devnode,
                candidate.instance_id(),
                "pnp:net_identity_changed",
            )
            .is_ok()
            && unsafe { CM_Enable_DevNode(*devnode, 0) } == CR_SUCCESS;
        Ok(NetDevnodeRestart::EnableFailed {
            os_code: enabled,
            recovery,
        })
    }

    fn verify_devnode_id(
        devnode: Devinst,
        expected: &str,
        error_code: &'static str,
    ) -> InventoryResult<()> {
        match cm_device_id(devnode)? {
            Some(actual) if actual.eq_ignore_ascii_case(expected) => Ok(()),
            Some(_) => Err(PlatformError {
                code: error_code,
                os_code: None,
            }),
            None => Err(PlatformError {
                code: "pnp:instance_id_missing",
                os_code: None,
            }),
        }
    }

    fn enumerate_devices(
        net_paths: &HashMap<Devinst, String>,
    ) -> InventoryResult<Vec<DeviceRecord>> {
        // SAFETY: null class/enumerator/window is valid with DIGCF_ALLCLASSES; the returned handle
        // is wrapped immediately and destroyed exactly once by DeviceInfoSet.
        let raw = unsafe {
            SetupDiGetClassDevsW(null(), null(), null_mut(), DIGCF_ALLCLASSES | DIGCF_PRESENT)
        };
        let set = checked_set(raw)?;
        let mut records = Vec::new();
        for index in 0..u32::MAX {
            // SAFETY: a zeroed SP_DEVINFO_DATA is valid once cb_size is initialized as required.
            let mut info: SpDevinfoData = unsafe { zeroed() };
            info.cb_size = size_of::<SpDevinfoData>() as u32;
            // SAFETY: `set` is live and `info` points to writable initialized storage.
            if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut info) } == 0 {
                let code = last_error();
                if code == ERROR_NO_MORE_ITEMS {
                    break;
                }
                return Err(interface_error(code, "pnp:enumerate_failed"));
            }
            let instance_id = match property_string(set.0, &info, &DEVPKEY_DEVICE_INSTANCE_ID)? {
                Some(value) => value,
                None => cm_device_id(info.dev_inst)?.ok_or(PlatformError {
                    code: "pnp:instance_id_missing",
                    os_code: None,
                })?,
            };
            let ancestry = ancestry(info.dev_inst, &instance_id)?;
            let class = property_string(set.0, &info, &DEVPKEY_DEVICE_CLASS)?;
            let net_cfg_instance_id = if class
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("net"))
                && net_paths.contains_key(&info.dev_inst)
                && ancestry
                    .iter()
                    .any(|value| super::is_exact_target_identity(value))
            {
                registry_net_cfg_instance_id(set.0, &info)?
            } else {
                None
            };
            records.push(DeviceRecord {
                dev_inst: info.dev_inst,
                ancestry,
                instance_id,
                hardware_ids: property_multi_string(set.0, &info, &DEVPKEY_DEVICE_HARDWARE_IDS)?,
                container_id: property_guid(set.0, &info, &DEVPKEY_DEVICE_CONTAINER_ID)?,
                class,
                friendly_name: property_string(set.0, &info, &DEVPKEY_DEVICE_FRIENDLY_NAME)?,
                bus_reported_desc: property_string(
                    set.0,
                    &info,
                    &DEVPKEY_DEVICE_BUS_REPORTED_DESC,
                )?,
                problem_code: property_u32(set.0, &info, &DEVPKEY_DEVICE_PROBLEM_CODE)?,
                port_name: registry_port_name(set.0, &info)?,
                net_cfg_instance_id,
            });
        }
        Ok(records)
    }

    fn enumerate_interface_paths(class_guid: &Guid) -> InventoryResult<HashMap<Devinst, String>> {
        // SAFETY: `class_guid` is a valid static interface GUID and null optional parameters are
        // accepted; the returned handle is immediately wrapped for single destruction.
        let raw = unsafe {
            SetupDiGetClassDevsW(
                class_guid,
                null(),
                null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        };
        let set = checked_set(raw)?;
        let mut paths = HashMap::new();
        for index in 0..u32::MAX {
            // SAFETY: the C structure permits zero initialization before cb_size is assigned.
            let mut interface: SpDeviceInterfaceData = unsafe { zeroed() };
            interface.cb_size = size_of::<SpDeviceInterfaceData>() as u32;
            // SAFETY: the live set and writable interface structure satisfy SetupAPI's contract.
            if unsafe {
                SetupDiEnumDeviceInterfaces(set.0, null_mut(), class_guid, index, &mut interface)
            } == 0
            {
                let code = last_error();
                if code == ERROR_NO_MORE_ITEMS {
                    break;
                }
                return Err(interface_error(code, "pnp:interface_enumerate_failed"));
            }

            let mut required = 0;
            // SAFETY: this documented sizing call intentionally passes a null detail buffer.
            let sizing_ok = unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    set.0,
                    &interface,
                    null_mut(),
                    0,
                    &mut required,
                    null_mut(),
                )
            };
            let sizing_error = last_error();
            if sizing_ok != 0 || sizing_error != ERROR_INSUFFICIENT_BUFFER || required < 8 {
                return Err(interface_error(sizing_error, "pnp:interface_detail_failed"));
            }
            let mut detail = vec![0_u8; required as usize];
            let cb_size: u32 = if size_of::<usize>() == 8 { 8 } else { 6 };
            detail[..4].copy_from_slice(&cb_size.to_ne_bytes());
            // SAFETY: the C structure permits zero initialization before cb_size is assigned.
            let mut info: SpDevinfoData = unsafe { zeroed() };
            info.cb_size = size_of::<SpDevinfoData>() as u32;
            // SAFETY: `detail` is writable for exactly `required` bytes, begins with the required
            // cbSize, and both output structures live through the call.
            if unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    set.0,
                    &interface,
                    detail.as_mut_ptr().cast(),
                    required,
                    &mut required,
                    &mut info,
                )
            } == 0
            {
                return Err(interface_error(last_error(), "pnp:interface_detail_failed"));
            }
            let path = detail_path(&detail).ok_or(PlatformError {
                code: "pnp:interface_detail_invalid",
                os_code: None,
            })?;
            paths.insert(info.dev_inst, path);
        }
        Ok(paths)
    }

    fn detail_path(detail: &[u8]) -> Option<String> {
        if detail.len() < 6 {
            return None;
        }
        let mut wide = Vec::new();
        for offset in (4..detail.len().saturating_sub(1)).step_by(2) {
            let unit = u16::from_le_bytes([detail[offset], detail[offset + 1]]);
            if unit == 0 {
                break;
            }
            wide.push(unit);
        }
        (!wide.is_empty()).then(|| String::from_utf16_lossy(&wide))
    }

    fn build_snapshot(
        records: Vec<DeviceRecord>,
        com_paths: &HashMap<Devinst, String>,
        net_paths: &HashMap<Devinst, String>,
    ) -> InventorySnapshot {
        let nodes = records
            .iter()
            .map(|record| PnpNode {
                instance_id: record.instance_id.clone(),
                hardware_ids: record.hardware_ids.clone(),
                ancestry: record.ancestry.clone(),
                container_id: record.container_id.clone(),
                problem_code: record.problem_code,
                native_devnode: Some(record.dev_inst),
                port_name: record.port_name.clone(),
                com_interface_path: com_paths.get(&record.dev_inst).cloned(),
                net_interface_path: net_paths.get(&record.dev_inst).cloned(),
                net_cfg_instance_id: record.net_cfg_instance_id.clone(),
                role: classify_role(record),
                verified_modem: record
                    .class
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("modem")),
            })
            .collect();
        correlate_topology(nodes)
    }

    fn classify_role(record: &DeviceRecord) -> FunctionRole {
        let friendly = record
            .friendly_name
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let bus_desc = record
            .bus_reported_desc
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let combined = format!("{friendly} {bus_desc}");
        if combined.contains("nmea") || combined.contains(" gps") {
            FunctionRole::Nmea
        } else if combined.contains("diag")
            || combined.contains(" dm ")
            || combined.contains("dm port")
        {
            FunctionRole::DmDiag
        } else if has_at_role(&friendly) || has_at_role(&bus_desc) {
            FunctionRole::DedicatedAt
        } else if record
            .class
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("modem"))
        {
            FunctionRole::Modem
        } else {
            FunctionRole::Unknown
        }
    }

    /// Either descriptive string carrying the AT signal is enough: real machines frequently
    /// pair a generic FriendlyName ("USB Serial Device (COM7)") with a vendor-specific
    /// bus-reported description, and demanding both strings match leaves such ports untrusted
    /// forever. NMEA and DM/diag tokens above still steal the role first.
    fn has_at_role(value: &str) -> bool {
        value.contains("at port")
            || value.contains("usb at")
            || value.contains("at command")
            || value.contains("at cmd")
            || contains_word(value, "at")
    }

    /// ASCII word-boundary containment: the token "at" in "USB AT" matches, "flat"/"format" do
    /// not. Multi-byte UTF-8 sequences never contain ASCII bytes, so byte windows are safe.
    fn contains_word(value: &str, word: &str) -> bool {
        let bytes = value.as_bytes();
        let word = word.as_bytes();
        if word.is_empty() {
            return false;
        }
        bytes
            .windows(word.len())
            .enumerate()
            .filter(|(_, window)| *window == word)
            .any(|(index, _)| {
                let before_ok = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
                let after_index = index + word.len();
                let after_ok =
                    after_index == bytes.len() || !bytes[after_index].is_ascii_alphanumeric();
                before_ok && after_ok
            })
    }

    fn property_bytes(
        set: Hdevinfo,
        info: &SpDevinfoData,
        key: &Devpropkey,
    ) -> InventoryResult<Option<PropertyValue>> {
        let mut capacity = 256_usize;
        for _ in 0..MAX_PROPERTY_ATTEMPTS {
            let mut bytes = vec![0_u8; capacity];
            let mut property_type = 0;
            let mut required = 0;
            // SAFETY: buffers and pointers are valid for the provided byte lengths; SetupAPI does
            // not retain them after this call.
            let ok = unsafe {
                SetupDiGetDevicePropertyW(
                    set,
                    info,
                    key,
                    &mut property_type,
                    bytes.as_mut_ptr(),
                    bytes.len() as u32,
                    &mut required,
                    0,
                )
            };
            if ok != 0 {
                bytes.truncate(required as usize);
                return Ok(Some(PropertyValue {
                    property_type,
                    bytes,
                }));
            }
            let status = last_error();
            if status == ERROR_INSUFFICIENT_BUFFER && required as usize > capacity {
                capacity = required as usize;
                continue;
            }
            return classify_optional_win32(OptionalWin32Api::DeviceProperty, status).map(|_| None);
        }
        Err(PlatformError {
            code: "pnp:property_buffer_failed",
            os_code: None,
        })
    }

    fn property_string(
        set: Hdevinfo,
        info: &SpDevinfoData,
        key: &Devpropkey,
    ) -> InventoryResult<Option<String>> {
        let Some(value) = property_bytes(set, info, key)? else {
            return Ok(None);
        };
        decode_property_string(value.property_type, &value.bytes).map(Some)
    }

    fn property_multi_string(
        set: Hdevinfo,
        info: &SpDevinfoData,
        key: &Devpropkey,
    ) -> InventoryResult<Vec<String>> {
        match property_bytes(set, info, key)? {
            Some(value) => decode_property_string_list(value.property_type, &value.bytes),
            None => Ok(Vec::new()),
        }
    }

    fn property_u32(
        set: Hdevinfo,
        info: &SpDevinfoData,
        key: &Devpropkey,
    ) -> InventoryResult<Option<u32>> {
        let Some(value) = property_bytes(set, info, key)? else {
            return Ok(None);
        };
        if value.property_type != DEVPROP_TYPE_UINT32 {
            return Err(PlatformError {
                code: "pnp:property_type_invalid",
                os_code: None,
            });
        }
        let array: [u8; 4] = value
            .bytes
            .get(..4)
            .and_then(|value| value.try_into().ok())
            .ok_or(PlatformError {
                code: "pnp:property_invalid",
                os_code: None,
            })?;
        Ok(Some(u32::from_ne_bytes(array)))
    }

    fn property_guid(
        set: Hdevinfo,
        info: &SpDevinfoData,
        key: &Devpropkey,
    ) -> InventoryResult<Option<String>> {
        let Some(value) = property_bytes(set, info, key)? else {
            return Ok(None);
        };
        if value.property_type != DEVPROP_TYPE_GUID {
            return Err(PlatformError {
                code: "pnp:property_type_invalid",
                os_code: None,
            });
        }
        if value.bytes.len() != size_of::<Guid>() {
            return Err(PlatformError {
                code: "pnp:property_invalid",
                os_code: None,
            });
        }
        // SAFETY: length was checked; read_unaligned accepts the byte buffer's alignment.
        let guid = unsafe { value.bytes.as_ptr().cast::<Guid>().read_unaligned() };
        Ok(Some(format_guid(guid)))
    }

    fn format_guid(guid: Guid) -> String {
        format!(
            "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7]
        )
    }

    fn utf16_strings(bytes: &[u8]) -> Vec<String> {
        let mut wide = Vec::with_capacity(bytes.len() / 2);
        for pair in bytes.chunks_exact(2) {
            wide.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
        wide.split(|unit| *unit == 0)
            .take_while(|value| !value.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }

    fn decode_property_string(property_type: Dword, bytes: &[u8]) -> InventoryResult<String> {
        if property_type != DEVPROP_TYPE_STRING {
            return Err(PlatformError {
                code: "pnp:property_type_invalid",
                os_code: None,
            });
        }
        let wide = strict_utf16_units(bytes)?;
        let Some((&0, content)) = wide.split_last() else {
            return Err(PlatformError {
                code: "pnp:property_invalid",
                os_code: None,
            });
        };
        if content.is_empty() || content.contains(&0) {
            return Err(PlatformError {
                code: "pnp:property_invalid",
                os_code: None,
            });
        }
        String::from_utf16(content).map_err(|_| PlatformError {
            code: "pnp:property_invalid",
            os_code: None,
        })
    }

    fn decode_property_string_list(
        property_type: Dword,
        bytes: &[u8],
    ) -> InventoryResult<Vec<String>> {
        if property_type != DEVPROP_TYPE_STRING_LIST {
            return Err(PlatformError {
                code: "pnp:property_type_invalid",
                os_code: None,
            });
        }
        let wide = strict_utf16_units(bytes)?;
        if wide.len() < 2 || wide[wide.len() - 2..] != [0, 0] {
            return Err(PlatformError {
                code: "pnp:property_invalid",
                os_code: None,
            });
        }
        let content = &wide[..wide.len() - 2];
        if content.is_empty() {
            return Ok(Vec::new());
        }
        content
            .split(|unit| *unit == 0)
            .map(|entry| {
                if entry.is_empty() {
                    return Err(PlatformError {
                        code: "pnp:property_invalid",
                        os_code: None,
                    });
                }
                String::from_utf16(entry).map_err(|_| PlatformError {
                    code: "pnp:property_invalid",
                    os_code: None,
                })
            })
            .collect()
    }

    fn strict_utf16_units(bytes: &[u8]) -> InventoryResult<Vec<u16>> {
        if bytes.len() % 2 != 0 {
            return Err(PlatformError {
                code: "pnp:property_invalid",
                os_code: None,
            });
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect())
    }

    fn ancestry(mut dev_inst: Devinst, instance_id: &str) -> InventoryResult<Vec<String>> {
        let mut values = vec![instance_id.to_owned()];
        for _ in 0..MAX_ANCESTRY_DEPTH {
            let mut parent = 0;
            // SAFETY: `parent` points to writable storage and `dev_inst` came from SetupAPI/CM.
            let status = unsafe { CM_Get_Parent(&mut parent, dev_inst, 0) };
            if !classify_parent_status(status)? {
                break;
            }
            if parent == dev_inst {
                break;
            }
            dev_inst = parent;
            if let Some(id) = cm_device_id(dev_inst)? {
                values.push(id);
            }
        }
        Ok(values)
    }

    fn cm_device_id(dev_inst: Devinst) -> InventoryResult<Option<String>> {
        let mut buffer = [0_u16; MAX_DEVICE_ID_LEN];
        // SAFETY: the fixed buffer is writable for MAX_DEVICE_ID_LEN UTF-16 units and the devnode
        // identifier came from Configuration Manager or SetupAPI.
        let status = unsafe {
            CM_Get_Device_IDW(
                dev_inst,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                CM_LOCATE_DEVNODE_NORMAL,
            )
        };
        if status == CR_NO_SUCH_DEVNODE {
            return Ok(None);
        }
        if status != CR_SUCCESS {
            return Err(PlatformError {
                code: if status == CR_ACCESS_DENIED {
                    "pnp:permission_denied"
                } else {
                    "pnp:cm_device_id_failed"
                },
                os_code: Some(status),
            });
        }
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .ok_or(PlatformError {
                code: "pnp:cm_device_id_invalid",
                os_code: None,
            })?;
        String::from_utf16(&buffer[..length])
            .map(Some)
            .map_err(|_| PlatformError {
                code: "pnp:cm_device_id_invalid",
                os_code: None,
            })
    }

    fn registry_port_name(set: Hdevinfo, info: &SpDevinfoData) -> InventoryResult<Option<String>> {
        // SAFETY: the live set and enumerated device data are valid; returned key is closed below.
        let key = unsafe {
            SetupDiOpenDevRegKey(set, info, DICS_FLAG_GLOBAL, 0, DIREG_DEV, KEY_QUERY_VALUE)
        };
        if key as isize == INVALID_HANDLE_VALUE {
            let status = last_error();
            return classify_optional_win32(OptionalWin32Api::RegistryKey, status).map(|_| None);
        }
        let name: Vec<u16> = "PortName\0".encode_utf16().collect();
        let result = (|| {
            let mut capacity = 128_usize;
            for _ in 0..MAX_PROPERTY_ATTEMPTS {
                let mut buffer = vec![0_u8; capacity];
                let mut byte_len = buffer.len() as u32;
                // SAFETY: the key is live; value name is NUL-terminated; output buffer size is exact.
                let status = unsafe {
                    RegQueryValueExW(
                        key,
                        name.as_ptr(),
                        null_mut(),
                        null_mut(),
                        buffer.as_mut_ptr(),
                        &mut byte_len,
                    )
                };
                if status == 0 {
                    buffer.truncate(byte_len as usize);
                    return utf16_strings(&buffer).into_iter().next().map(Some).ok_or(
                        PlatformError {
                            code: "pnp:registry_value_invalid",
                            os_code: None,
                        },
                    );
                }
                if status == ERROR_MORE_DATA && byte_len as usize > capacity {
                    capacity = checked_registry_growth(capacity, byte_len as usize)?;
                    continue;
                }
                return classify_optional_win32(OptionalWin32Api::RegistryValue, status)
                    .map(|_| None);
            }
            Err(PlatformError {
                code: "pnp:registry_buffer_failed",
                os_code: None,
            })
        })();
        // SAFETY: `key` is owned by this function and is closed exactly once.
        unsafe { RegCloseKey(key) };
        result
    }

    fn registry_net_cfg_instance_id(
        set: Hdevinfo,
        info: &SpDevinfoData,
    ) -> InventoryResult<Option<String>> {
        registry_strict_string(
            set,
            info,
            DIREG_DRV,
            "NetCfgInstanceId",
            "net:netcfg_id_invalid",
        )
    }

    fn registry_strict_string(
        set: Hdevinfo,
        info: &SpDevinfoData,
        key_type: Dword,
        value_name: &str,
        invalid_code: &'static str,
    ) -> InventoryResult<Option<String>> {
        // SAFETY: the live device info set and enumerated record satisfy SetupAPI's input
        // contract. The returned key is owned here and closed exactly once below.
        let key = unsafe {
            SetupDiOpenDevRegKey(set, info, DICS_FLAG_GLOBAL, 0, key_type, KEY_QUERY_VALUE)
        };
        if key as isize == INVALID_HANDLE_VALUE {
            let status = last_error();
            return classify_optional_win32(OptionalWin32Api::RegistryKey, status).map(|_| None);
        }
        let name: Vec<u16> = value_name.encode_utf16().chain([0]).collect();
        let result = (|| {
            let mut capacity = 128_usize;
            for _ in 0..MAX_PROPERTY_ATTEMPTS {
                let mut buffer = vec![0_u8; capacity];
                let mut byte_len = buffer.len() as u32;
                let mut value_type = 0;
                // SAFETY: the key is live, name is NUL-terminated, and the output slices are
                // writable for the sizes passed to RegQueryValueExW.
                let status = unsafe {
                    RegQueryValueExW(
                        key,
                        name.as_ptr(),
                        null_mut(),
                        &mut value_type,
                        buffer.as_mut_ptr(),
                        &mut byte_len,
                    )
                };
                if status == 0 {
                    if value_type != REG_SZ || byte_len as usize > buffer.len() {
                        return Err(PlatformError {
                            code: invalid_code,
                            os_code: None,
                        });
                    }
                    buffer.truncate(byte_len as usize);
                    return decode_registry_string(value_type, &buffer, invalid_code).map(Some);
                }
                if status == ERROR_MORE_DATA && byte_len as usize > capacity {
                    capacity = checked_registry_growth(capacity, byte_len as usize)?;
                    continue;
                }
                return classify_optional_win32(OptionalWin32Api::RegistryValue, status)
                    .map(|_| None);
            }
            Err(PlatformError {
                code: "pnp:registry_buffer_failed",
                os_code: None,
            })
        })();
        // SAFETY: `key` is owned by this function and has not otherwise been closed.
        unsafe { RegCloseKey(key) };
        result
    }

    fn decode_registry_string(
        value_type: Dword,
        bytes: &[u8],
        invalid_code: &'static str,
    ) -> InventoryResult<String> {
        if value_type != REG_SZ {
            return Err(PlatformError {
                code: invalid_code,
                os_code: None,
            });
        }
        let units = strict_utf16_units(bytes).map_err(|_| PlatformError {
            code: invalid_code,
            os_code: None,
        })?;
        let Some((&0, content)) = units.split_last() else {
            return Err(PlatformError {
                code: invalid_code,
                os_code: None,
            });
        };
        if content.is_empty() || content.contains(&0) {
            return Err(PlatformError {
                code: invalid_code,
                os_code: None,
            });
        }
        String::from_utf16(content).map_err(|_| PlatformError {
            code: invalid_code,
            os_code: None,
        })
    }

    #[cfg(test)]
    mod strict_registry_size_tests {
        use super::*;

        #[test]
        fn rejects_oversized_driver_key_string_before_allocating() {
            assert_eq!(
                checked_registry_capacity(MAX_REGISTRY_STRING_BYTES + 2)
                    .unwrap_err()
                    .code,
                "pnp:registry_buffer_failed"
            );
        }

        #[test]
        fn rejects_non_growing_and_odd_utf16_sizes() {
            assert!(checked_registry_growth(128, 128).is_err());
            assert!(checked_registry_growth(128, 129).is_err());
            assert_eq!(checked_registry_growth(128, 256).unwrap(), 256);
        }
    }

    fn checked_set(raw: Hdevinfo) -> InventoryResult<DeviceInfoSet> {
        if raw as isize == INVALID_HANDLE_VALUE {
            let code = last_error();
            Err(os_error(
                if code == ERROR_ACCESS_DENIED {
                    "pnp:permission_denied"
                } else {
                    "pnp:open_failed"
                },
                code,
            ))
        } else {
            Ok(DeviceInfoSet(raw))
        }
    }

    fn last_error() -> i32 {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default()
    }

    fn os_error(code: &'static str, os_code: i32) -> PlatformError {
        PlatformError {
            code,
            os_code: Some(os_code as u32),
        }
    }

    #[link(name = "setupapi")]
    unsafe extern "system" {
        fn SetupDiGetClassDevsW(
            class_guid: *const Guid,
            enumerator: *const u16,
            hwnd_parent: *mut c_void,
            flags: Dword,
        ) -> Hdevinfo;
        fn SetupDiEnumDeviceInfo(
            device_info_set: Hdevinfo,
            member_index: Dword,
            device_info_data: *mut SpDevinfoData,
        ) -> Bool;
        fn SetupDiEnumDeviceInterfaces(
            device_info_set: Hdevinfo,
            device_info_data: *mut SpDevinfoData,
            interface_class_guid: *const Guid,
            member_index: Dword,
            device_interface_data: *mut SpDeviceInterfaceData,
        ) -> Bool;
        fn SetupDiGetDeviceInterfaceDetailW(
            device_info_set: Hdevinfo,
            device_interface_data: *const SpDeviceInterfaceData,
            device_interface_detail_data: *mut c_void,
            device_interface_detail_data_size: Dword,
            required_size: *mut Dword,
            device_info_data: *mut SpDevinfoData,
        ) -> Bool;
        fn SetupDiGetDevicePropertyW(
            device_info_set: Hdevinfo,
            device_info_data: *const SpDevinfoData,
            property_key: *const Devpropkey,
            property_type: *mut Dword,
            property_buffer: *mut u8,
            property_buffer_size: Dword,
            required_size: *mut Dword,
            flags: Dword,
        ) -> Bool;
        fn SetupDiOpenDevRegKey(
            device_info_set: Hdevinfo,
            device_info_data: *const SpDevinfoData,
            scope: Dword,
            hw_profile: Dword,
            key_type: Dword,
            sam_desired: Dword,
        ) -> Hkey;
        fn SetupDiDestroyDeviceInfoList(device_info_set: Hdevinfo) -> Bool;
    }

    #[link(name = "cfgmgr32")]
    unsafe extern "system" {
        fn CM_Get_Parent(parent: *mut Devinst, dev_inst: Devinst, flags: Dword) -> Dword;
        fn CM_Disable_DevNode(dev_inst: Devinst, flags: Dword) -> Dword;
        fn CM_Enable_DevNode(dev_inst: Devinst, flags: Dword) -> Dword;
        fn CM_Reenumerate_DevNode(dev_inst: Devinst, flags: Dword) -> Dword;
        fn CM_Get_Device_IDW(
            dev_inst: Devinst,
            buffer: *mut u16,
            buffer_len: Dword,
            flags: Dword,
        ) -> Dword;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegQueryValueExW(
            key: Hkey,
            value_name: *const u16,
            reserved: *mut Dword,
            value_type: *mut Dword,
            data: *mut u8,
            data_len: *mut Dword,
        ) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn optional_property_absence_is_not_an_inventory_failure() {
            assert_eq!(
                classify_optional_win32(OptionalWin32Api::DeviceProperty, ERROR_NOT_FOUND).unwrap(),
                None
            );
            assert_eq!(
                classify_optional_win32(OptionalWin32Api::RegistryValue, ERROR_FILE_NOT_FOUND)
                    .unwrap(),
                None
            );
            assert_eq!(
                classify_optional_win32(OptionalWin32Api::RegistryKey, ERROR_KEY_DOES_NOT_EXIST,)
                    .unwrap(),
                None
            );
        }

        #[test]
        fn invalid_property_data_is_a_stable_failure_not_absence() {
            assert_eq!(
                classify_optional_win32(OptionalWin32Api::DeviceProperty, ERROR_INVALID_DATA)
                    .unwrap_err()
                    .code,
                "pnp:property_invalid"
            );
        }

        #[test]
        fn hardware_id_string_list_requires_type_termination_and_strict_utf16() {
            let mut valid = "USB\\VID_2CA3&PID_4006"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            valid.extend_from_slice(&[0, 0, 0, 0]);
            assert_eq!(
                decode_property_string_list(DEVPROP_TYPE_STRING_LIST, &valid).unwrap(),
                vec![r"USB\VID_2CA3&PID_4006"]
            );

            assert_eq!(
                decode_property_string_list(DEVPROP_TYPE_STRING, &valid)
                    .unwrap_err()
                    .code,
                "pnp:property_type_invalid"
            );
            assert_eq!(
                decode_property_string_list(DEVPROP_TYPE_STRING_LIST, &[0, 0, 0])
                    .unwrap_err()
                    .code,
                "pnp:property_invalid"
            );
            assert_eq!(
                decode_property_string_list(DEVPROP_TYPE_STRING_LIST, &[b'A', 0, 0, 0])
                    .unwrap_err()
                    .code,
                "pnp:property_invalid"
            );
            assert_eq!(
                decode_property_string_list(DEVPROP_TYPE_STRING_LIST, &[0x00, 0xD8, 0, 0, 0, 0])
                    .unwrap_err()
                    .code,
                "pnp:property_invalid"
            );
        }

        #[test]
        fn netcfg_registry_string_requires_reg_sz_termination_and_strict_utf16() {
            let mut valid = "{8BDD4C57-901D-4A19-B85B-970F32B4C41A}"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            valid.extend_from_slice(&[0, 0]);
            assert_eq!(
                decode_registry_string(REG_SZ, &valid, "net:netcfg_id_invalid").unwrap(),
                "{8BDD4C57-901D-4A19-B85B-970F32B4C41A}"
            );
            for (value_type, bytes) in [
                (7, valid.as_slice()),
                (REG_SZ, &valid[..valid.len() - 2]),
                (REG_SZ, &[0x00, 0xD8, 0, 0]),
            ] {
                assert_eq!(
                    decode_registry_string(value_type, bytes, "net:netcfg_id_invalid")
                        .unwrap_err()
                        .code,
                    "net:netcfg_id_invalid"
                );
            }
        }

        #[test]
        fn optional_property_permission_and_system_errors_are_not_silenced() {
            for api in [
                OptionalWin32Api::DeviceProperty,
                OptionalWin32Api::RegistryKey,
                OptionalWin32Api::RegistryValue,
            ] {
                let denied = classify_optional_win32(api, ERROR_ACCESS_DENIED).unwrap_err();
                assert_eq!(denied.code, "pnp:permission_denied");
                assert_eq!(denied.os_code, Some(ERROR_ACCESS_DENIED as u32));
            }

            assert_eq!(
                classify_optional_win32(OptionalWin32Api::DeviceProperty, 31)
                    .unwrap_err()
                    .code,
                "pnp:property_failed"
            );
            assert_eq!(
                classify_optional_win32(OptionalWin32Api::RegistryValue, 31)
                    .unwrap_err()
                    .code,
                "pnp:registry_failed"
            );
        }

        #[test]
        fn cm_parent_absence_is_distinct_from_permission_and_system_failures() {
            assert!(!classify_parent_status(CR_NO_SUCH_DEVNODE).unwrap());
            assert!(classify_parent_status(CR_SUCCESS).unwrap());
            assert_eq!(
                classify_parent_status(CR_ACCESS_DENIED).unwrap_err().code,
                "pnp:permission_denied"
            );
            assert_eq!(
                classify_parent_status(0xFFFF).unwrap_err().code,
                "pnp:cm_parent_failed"
            );
        }

        #[test]
        fn interface_errors_have_stable_permission_and_operation_codes() {
            assert_eq!(
                interface_error(ERROR_ACCESS_DENIED, "pnp:interface_enumerate_failed").code,
                "pnp:permission_denied"
            );
            assert_eq!(
                interface_error(31, "pnp:interface_detail_failed").code,
                "pnp:interface_detail_failed"
            );
        }

        fn role_record(
            friendly: Option<&str>,
            bus: Option<&str>,
            class: Option<&str>,
        ) -> DeviceRecord {
            DeviceRecord {
                dev_inst: 0,
                instance_id: r"USB\VID_2CA3&PID_4006&MI_02".to_owned(),
                hardware_ids: vec![r"USB\VID_2CA3&PID_4006&REV_0318".to_owned()],
                ancestry: vec![r"USB\VID_2CA3&PID_4006".to_owned()],
                container_id: Some("{container}".to_owned()),
                class: class.map(str::to_owned),
                friendly_name: friendly.map(str::to_owned),
                bus_reported_desc: bus.map(str::to_owned),
                problem_code: None,
                port_name: Some("COM7".to_owned()),
                net_cfg_instance_id: None,
            }
        }

        #[test]
        fn at_role_matches_either_description_string() {
            assert_eq!(
                classify_role(&role_record(
                    Some("USB Serial Device (COM7)"),
                    Some("Quectel USB AT Port"),
                    None,
                )),
                FunctionRole::DedicatedAt
            );
            assert_eq!(
                classify_role(&role_record(
                    Some("Quectel USB AT Port"),
                    Some("USB Serial Device (COM7)"),
                    None,
                )),
                FunctionRole::DedicatedAt
            );
        }

        #[test]
        fn generic_names_stay_unknown_and_modem_class_is_preserved() {
            assert_eq!(
                classify_role(&role_record(Some("USB Serial Device (COM7)"), None, None,)),
                FunctionRole::Unknown
            );
            assert_eq!(
                classify_role(&role_record(None, None, Some("Modem"),)),
                FunctionRole::Modem
            );
        }

        #[test]
        fn nmea_and_diag_tokens_still_steal_the_role() {
            assert_eq!(
                classify_role(&role_record(Some("Quectel USB NMEA Port"), None, None,)),
                FunctionRole::Nmea
            );
            assert_eq!(
                classify_role(&role_record(None, Some("Quectel USB DM Port"), None,)),
                FunctionRole::DmDiag
            );
        }

        #[test]
        fn standalone_at_token_matches_on_word_boundaries_only() {
            assert!(contains_word("dji usb at", "at"));
            assert!(contains_word("at command port", "at"));
            assert!(!contains_word("flat port", "at"));
            assert!(!contains_word("compatible", "at"));
        }
    }
}

#[cfg(test)]
#[path = "pnp_fixture_tests.rs"]
mod fixture_tests;
