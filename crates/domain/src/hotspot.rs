use serde::{Deserialize, Serialize};

use crate::ErrorCode;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HotspotUnsupportedReason {
    MissingPackageIdentity,
    MissingWifiControlCapability,
    NoWifiAdapter,
    PolicyDisabled,
    UnsupportedOperatingSystem,
    SourceProfileUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HotspotStatus {
    Unsupported(HotspotUnsupportedReason),
    Off,
    Starting,
    On { clients: Option<u32> },
    Stopping,
    Failed { code: ErrorCode },
}
