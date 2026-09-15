use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub vid: u16,
    pub pid: u16,
}

impl DeviceProfile {
    pub const DJI_GEN1: Self = DJI_GEN1;

    #[must_use]
    pub const fn matches(self, vid: u16, pid: u16) -> bool {
        self.vid == vid && self.pid == pid
    }
}

pub const DJI_GEN1: DeviceProfile = DeviceProfile {
    vid: 0x2CA3,
    pid: 0x4006,
};

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct DeviceEpoch(pub u64);

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StableDeviceIdentity {
    pub container_id: String,
    pub device_instance_id: String,
    pub vid: u16,
    pub pid: u16,
}

impl StableDeviceIdentity {
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.parsed_usb_vid_pid().is_some_and(|(vid, pid)| {
            DJI_GEN1.matches(vid, pid) && (vid, pid) == (self.vid, self.pid)
        })
    }

    fn parsed_usb_vid_pid(&self) -> Option<(u16, u16)> {
        let mut segments = self.device_instance_id.split('\\');
        let bus = segments.next()?;
        let hardware_id = segments.next()?;
        let instance = segments.next()?;
        if !bus.eq_ignore_ascii_case("USB") || instance.is_empty() || segments.next().is_some() {
            return None;
        }

        let mut vid = None;
        let mut pid = None;
        for component in hardware_id.split('&') {
            if let Some(value) = parse_prefixed_hex_u16(component, "VID_") {
                if vid.replace(value).is_some() {
                    return None;
                }
            } else if let Some(value) = parse_prefixed_hex_u16(component, "PID_") {
                if pid.replace(value).is_some() {
                    return None;
                }
            }
        }

        Some((vid?, pid?))
    }
}

fn parse_prefixed_hex_u16(component: &str, prefix: &str) -> Option<u16> {
    if component.len() != prefix.len() + 4
        || !component.get(..prefix.len())?.eq_ignore_ascii_case(prefix)
    {
        return None;
    }

    u16::from_str_radix(component.get(prefix.len()..)?, 16).ok()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EvidenceSource {
    Pnp,
    AtControl,
    WindowsAdapter,
    BoundGatewayProbe,
    BoundDnsProbe,
    BoundPublicProbe,
    GlobalRoute,
    GlobalConnectivity,
    Hotspot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence<T> {
    pub epoch: DeviceEpoch,
    pub observed_at: SystemTime,
    pub ttl: Duration,
    pub source: EvidenceSource,
    pub value: T,
}

impl<T> Evidence<T> {
    #[must_use]
    pub fn is_fresh_for(&self, epoch: DeviceEpoch, now: SystemTime) -> bool {
        if self.epoch != epoch {
            return false;
        }

        now.duration_since(self.observed_at)
            .is_ok_and(|age| age <= self.ttl)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DevicePresence {
    Supported(DeviceProfile),
    NotDetected,
    Unsupported { vid: u16, pid: u16 },
    PermissionDenied,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSnapshot {
    pub epoch: DeviceEpoch,
    pub identity: StableDeviceIdentity,
    pub problem_code: Option<u32>,
    pub at_port: Option<String>,
    pub adapter_id: Option<String>,
}
