//! Capability-status state machine for optional module features (research document §8.1).

use dji4g_domain::{DeviceProfile, FeatureStatus};

/// The module features the collection layer can probe in this release.  Later phases extend this
/// enum (SMS, GNSS, …); the container only models what a collector can report today.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FeatureKey {
    /// AT+CNUM phone-number lookup.
    Cnum,
    /// AT+QCCID SIM-identity read.
    Qccid,
}

impl FeatureKey {
    /// Every feature key modeled in this release, in a stable order.
    pub const ALL: [Self; 2] = [Self::Cnum, Self::Qccid];
}

/// Capability verdicts scoped to one module context — the 型号 [`DeviceProfile`] (VID/PID, i.e.
/// the USB combination) plus the module firmware string — rather than to the application as a
/// whole.
///
/// The container is a pure state machine: it stores exactly what the collection layer reports and
/// never invents a verdict.  In particular an `Empty` result (e.g. CNUM answered `OK` with no
/// records) is recorded as `Empty` and is never promoted to [`FeatureStatus::UnsupportedConfirmed`];
/// only interpretable evidence (a stable error code the collector recognizes) becomes
/// `UnsupportedConfirmed`.  `TransportFailure`/`FormatMismatch` are likewise stored verbatim and
/// decided by the collection layer.  When the firmware string changes, the module cache is
/// invalidated and every feature returns to [`FeatureStatus::NotProbed`]; SIM/operator-dependent
/// availability is recorded separately and is not part of this cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureCapability {
    profile: DeviceProfile,
    firmware: Option<String>,
    cnum: FeatureStatus,
    qccid: FeatureStatus,
}

impl FeatureCapability {
    /// Anchor a fresh capability container at the given module context; every feature starts
    /// [`FeatureStatus::NotProbed`].
    pub(crate) fn new(profile: DeviceProfile, firmware: Option<String>) -> Self {
        Self {
            profile,
            firmware,
            cnum: FeatureStatus::NotProbed,
            qccid: FeatureStatus::NotProbed,
        }
    }

    /// The 型号 + USB combination (VID/PID) this capability set is scoped to.
    #[must_use]
    pub const fn profile(&self) -> DeviceProfile {
        self.profile
    }

    /// The firmware string this capability set is scoped to, if the module reported one.
    #[must_use]
    pub fn firmware(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    /// Current verdict for one feature; [`FeatureStatus::NotProbed`] before the first recorded
    /// observation.
    #[must_use]
    pub fn get(&self, key: FeatureKey) -> FeatureStatus {
        match key {
            FeatureKey::Cnum => self.cnum,
            FeatureKey::Qccid => self.qccid,
        }
    }

    /// Record the collector's verdict verbatim.
    pub(crate) fn set(&mut self, key: FeatureKey, status: FeatureStatus) {
        match key {
            FeatureKey::Cnum => self.cnum = status,
            FeatureKey::Qccid => self.qccid = status,
        }
    }

    /// Retarget the container after a firmware change and return every feature to `NotProbed`.
    /// Returns whether the firmware string actually changed.
    pub(crate) fn adopt_firmware(&mut self, firmware: Option<String>) -> bool {
        if self.firmware == firmware {
            return false;
        }
        self.firmware = firmware;
        self.cnum = FeatureStatus::NotProbed;
        self.qccid = FeatureStatus::NotProbed;
        true
    }
}
