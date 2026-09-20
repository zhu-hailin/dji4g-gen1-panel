//! Optional read-only AT probes: privacy helpers, outcome bookkeeping, and UI correlation.
//!
//! Each observation cycle the panel probes three *optional* features alongside the mandatory
//! reads: `AT+CNUM` (本机号码), `AT+QCCID` (SIM ICCID) and the serving-cell report
//! (research §8.1 / §10).  A probe may legitimately be empty, unsupported, mistimed or
//! malformed, and none of those outcomes may fail the whole observation or pollute the
//! availability verdict.  The runtime classifies each probe and records [`FeatureProbeState`]
//! together with the exact values it stored into the snapshot; the UI only uses the record while
//! [`probe_view`] proves it belongs to the snapshot being rendered, so the failure/status text
//! shown next to a row can never describe a different observation than the row itself.
//!
//! Privacy boundary: the plaintext ICCID never enters any snapshot, serialization, log or export
//! (only the mask and a fingerprint travel in [`SimIdentity`]).  The one plaintext copy kept for
//! an explicit user [显示] click lives only in this in-process record and is dropped on the next
//! observation; [`FeatureProbeView`] hands it to the UI exclusively while correlated.

use dji4g_application::DeviceEpoch;
use dji4g_at_protocol::SensorTemperature;
use dji4g_domain::{CellularSnapshot, FeatureStatus, NumberLookup, ServingCell, SimIdentity};

/// Masked ICCID display (research §4.3): first four and last four digits around an ellipsis.
///
/// An ICCID too short for two disjoint groups is fully masked: for ≤ 8 digits the two groups
/// would overlap and would reveal the whole value, so a short response degrades to a length-free
/// placeholder instead of a leaky mask.
#[must_use]
pub fn mask_iccid(iccid: &str) -> String {
    let digits = iccid
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<Vec<char>>();
    if digits.len() < 9 {
        return "****…****".to_owned();
    }
    let head = digits[..4].iter().collect::<String>();
    let tail = digits[digits.len() - 4..].iter().collect::<String>();
    format!("{head}…{tail}")
}

/// Stable fingerprint derived from the full ICCID: the first eight bytes of its SHA-256 digest.
///
/// The fingerprint is not a secret (anyone holding the ICCID can recompute it), but it is not
/// the plaintext either; it only ever travels inside [`SimIdentity`] and is used to detect SIM
/// changes.
#[must_use]
pub fn iccid_fingerprint(iccid: &str) -> [u8; 8] {
    let digest = sha256(iccid.as_bytes());
    let mut fingerprint = [0_u8; 8];
    fingerprint.copy_from_slice(&digest[..8]);
    fingerprint
}

/// Build the domain [`SimIdentity`] from a freshly parsed plaintext ICCID.
#[must_use]
pub fn sim_identity_for(iccid: &str) -> SimIdentity {
    SimIdentity {
        iccid_masked: mask_iccid(iccid),
        fingerprint: iccid_fingerprint(iccid),
    }
}

/// Pure SHA-256 over the input bytes (FIPS 180-4).  Crate-internal so the panel never needs a
/// digest dependency just to fingerprint an ICCID; used by no other path in this crate.
#[must_use]
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state = [
        0x6a09_e667_u32,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut blocks = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    blocks.push(0x80);
    while blocks.len() % 64 != 56 {
        blocks.push(0);
    }
    blocks.extend_from_slice(&bit_len.to_be_bytes());
    for block in blocks.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (index, &k) in K.iter().enumerate() {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k)
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state = [
            state[0].wrapping_add(a),
            state[1].wrapping_add(b),
            state[2].wrapping_add(c),
            state[3].wrapping_add(d),
            state[4].wrapping_add(e),
            state[5].wrapping_add(f),
            state[6].wrapping_add(g),
            state[7].wrapping_add(h),
        ];
    }
    let mut digest = [0_u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Per-probe bookkeeping of the most recent successful observation cycle.
///
/// The record mirrors the exact optional-feature values that were stored into the matching
/// [`CellularSnapshot`] plus the classified outcome of each probe.  The UI never trusts the
/// record blindly: [`probe_view`] re-checks the epoch and the captured values against the
/// snapshot being rendered first, so a record from an older or failed cycle can never describe
/// the current rows.
#[derive(Clone, PartialEq)]
pub struct FeatureProbeState {
    /// Device epoch of the observation cycle that produced this record.
    pub epoch: DeviceEpoch,
    /// Same values stored into the matching snapshot's `cellular`.
    pub numbers: Option<NumberLookup>,
    pub sim_identity: Option<SimIdentity>,
    pub serving_cell: Option<ServingCell>,
    /// Classified outcome of each optional probe.
    pub numbers_status: FeatureStatus,
    pub iccid_status: FeatureStatus,
    pub serving_cell_status: FeatureStatus,
    /// Plaintext ICCID of this cycle, kept only for an explicit user [显示] click.  Never
    /// serialized, logged or exported; replaced by the next observation.
    pub iccid_full: Option<String>,
    /// Raw `+QENG` serving-cell line as reported, for the row's hover/copy detail.
    pub serving_cell_raw: Option<String>,
    /// The module temperature stored into the matching snapshot, so the temperature rows can prove
    /// that the readings below belong to the value being displayed.
    pub temperature_celsius: Option<i16>,
    /// Every QTEMP reading of this cycle, in report order (§7.5).
    pub temperature_sensors: Vec<SensorTemperature>,
    /// Raw `+QTEMP:` line as reported, for the temperature rows' hover detail.
    pub temperature_raw: Option<String>,
}

impl std::fmt::Debug for FeatureProbeState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The plaintext ICCID and the raw serving-cell line stay out of every Debug path even
        // if a future log accidentally prints a record.
        formatter
            .debug_struct("FeatureProbeState")
            .field("epoch", &self.epoch)
            .field("numbers", &self.numbers)
            .field("sim_identity", &self.sim_identity)
            .field("serving_cell", &self.serving_cell)
            .field("numbers_status", &self.numbers_status)
            .field("iccid_status", &self.iccid_status)
            .field("serving_cell_status", &self.serving_cell_status)
            .field("iccid_full", &"[REDACTED]")
            .field("serving_cell_raw", &"[REDACTED]")
            .field("temperature_celsius", &self.temperature_celsius)
            .field("temperature_sensors", &self.temperature_sensors)
            .field("temperature_raw", &"[REDACTED]")
            .finish()
    }
}

impl Default for FeatureProbeState {
    fn default() -> Self {
        Self {
            epoch: DeviceEpoch(0),
            numbers: None,
            sim_identity: None,
            serving_cell: None,
            numbers_status: FeatureStatus::NotProbed,
            iccid_status: FeatureStatus::NotProbed,
            serving_cell_status: FeatureStatus::NotProbed,
            iccid_full: None,
            serving_cell_raw: None,
            temperature_celsius: None,
            temperature_sensors: Vec::new(),
            temperature_raw: None,
        }
    }
}

/// UI projection of one observation's optional probes.
///
/// [`captured`](Self::captured) is true only while the record demonstrably belongs to the
/// snapshot being rendered (same epoch and identical captured values).  Plaintext and the raw
/// serving-cell line are handed out exclusively in that case; a stale or mismatched record
/// yields neutral defaults and never any secret material.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureProbeView {
    pub captured: bool,
    pub numbers_status: FeatureStatus,
    pub iccid_status: FeatureStatus,
    pub serving_cell_status: FeatureStatus,
    pub iccid_full: Option<String>,
    pub serving_cell_raw: Option<String>,
    /// Every QTEMP reading of the correlated observation, in report order.
    pub temperature_sensors: Vec<SensorTemperature>,
    pub temperature_raw: Option<String>,
}

/// Correlate a probe record against the snapshot being rendered.
#[must_use]
pub fn probe_view(
    state: &FeatureProbeState,
    cellular: Option<&CellularSnapshot>,
    device_epoch: Option<DeviceEpoch>,
) -> FeatureProbeView {
    let values_match = cellular.is_some_and(|cell| {
        cell.numbers == state.numbers
            && cell.sim_identity == state.sim_identity
            && cell.serving_cell == state.serving_cell
            && cell.temperature_celsius == state.temperature_celsius
    });
    let captured = values_match && device_epoch.is_some_and(|epoch| epoch == state.epoch);
    if !captured {
        return FeatureProbeView {
            captured: false,
            numbers_status: FeatureStatus::NotProbed,
            iccid_status: FeatureStatus::NotProbed,
            serving_cell_status: FeatureStatus::NotProbed,
            iccid_full: None,
            serving_cell_raw: None,
            temperature_sensors: Vec::new(),
            temperature_raw: None,
        };
    }
    FeatureProbeView {
        captured: true,
        numbers_status: state.numbers_status,
        iccid_status: state.iccid_status,
        serving_cell_status: state.serving_cell_status,
        iccid_full: state.iccid_full.clone(),
        serving_cell_raw: state.serving_cell_raw.clone(),
        temperature_sensors: state.temperature_sensors.clone(),
        temperature_raw: state.temperature_raw.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FeatureProbeState, mask_iccid, probe_view, sha256};
    use dji4g_application::DeviceEpoch;
    use dji4g_at_protocol::SensorTemperature;
    use dji4g_domain::{
        AttachState, CellularSnapshot, FeatureStatus, NumberLookup, PhoneNumber, RegistrationState,
        SimIdentity, SimState,
    };

    fn cellular_with(numbers: Option<NumberLookup>) -> CellularSnapshot {
        CellularSnapshot {
            sim: SimState::Ready,
            registration: RegistrationState::RegisteredHome,
            attached: AttachState::Attached,
            carrier: None,
            radio_access_technology: None,
            signal_rssi_dbm: None,
            apn: None,
            pdp_address: None,
            pdp_state: None,
            firmware: None,
            serving_cell: None,
            sim_identity: None,
            numbers,
            temperature_celsius: None,
            temperature_status: FeatureStatus::NotProbed,
        }
    }

    #[test]
    fn iccid_mask_keeps_two_disjoint_groups() {
        assert_eq!(mask_iccid("89860123456789012345"), "8986…2345");
        assert_eq!(mask_iccid("123456789"), "1234…6789");
    }

    #[test]
    fn iccid_mask_fully_masks_short_values() {
        // Four groups of 4..=8 digits would overlap and leak the whole value, so short inputs
        // degrade to a length-free placeholder.
        assert_eq!(mask_iccid("123456"), "****…****");
        assert_eq!(mask_iccid("12345678"), "****…****");
        assert_eq!(mask_iccid(""), "****…****");
    }

    #[test]
    fn iccid_mask_ignores_nondigit_garnish() {
        assert_eq!(mask_iccid("8986 0123 4567 8901 2345"), "8986…2345");
    }

    #[test]
    fn sha256_matches_fips_180_4_vectors() {
        let empty = sha256(b"");
        assert_eq!(
            empty,
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55,
            ]
        );
        let abc = sha256(b"abc");
        assert_eq!(
            abc,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn fingerprint_is_stable_and_not_the_plaintext() {
        let iccid = "89860123456789012345";
        let first = super::iccid_fingerprint(iccid);
        let second = super::iccid_fingerprint(iccid);
        assert_eq!(first, second);
        let identity = super::sim_identity_for(iccid);
        assert_eq!(identity.fingerprint, first);
        assert_eq!(identity.iccid_masked, "8986…2345");
        let other = super::iccid_fingerprint("89860123456789012346");
        assert_ne!(
            first, other,
            "a one-digit difference must change the fingerprint"
        );
    }

    #[test]
    fn probe_view_is_captured_only_for_the_matching_snapshot() {
        let numbers = NumberLookup::Reported(vec![PhoneNumber::new("+8613800138000", 145)]);
        let cell = cellular_with(Some(numbers.clone()));
        let state = FeatureProbeState {
            epoch: DeviceEpoch(3),
            numbers: Some(numbers),
            sim_identity: None,
            serving_cell: None,
            numbers_status: FeatureStatus::Supported,
            iccid_status: FeatureStatus::UnsupportedConfirmed,
            serving_cell_status: FeatureStatus::NotProbed,
            iccid_full: None,
            serving_cell_raw: None,
            temperature_celsius: None,
            temperature_sensors: Vec::new(),
            temperature_raw: None,
        };
        let view = probe_view(&state, Some(&cell), Some(DeviceEpoch(3)));
        assert!(view.captured);
        assert_eq!(view.numbers_status, FeatureStatus::Supported);
        assert_eq!(view.iccid_status, FeatureStatus::UnsupportedConfirmed);
    }

    #[test]
    fn probe_view_stays_neutral_on_epoch_or_value_mismatch() {
        let numbers = NumberLookup::Reported(vec![PhoneNumber::new("+8613800138000", 145)]);
        let state = FeatureProbeState {
            epoch: DeviceEpoch(3),
            numbers: Some(numbers.clone()),
            sim_identity: Some(SimIdentity {
                iccid_masked: "8986…2345".to_owned(),
                fingerprint: [1; 8],
            }),
            serving_cell: None,
            numbers_status: FeatureStatus::Supported,
            iccid_status: FeatureStatus::Supported,
            serving_cell_status: FeatureStatus::NotProbed,
            iccid_full: Some("89860123456789012345".to_owned()),
            serving_cell_raw: Some("+QENG: raw".to_owned()),
            temperature_celsius: None,
            temperature_sensors: vec![SensorTemperature {
                name: None,
                celsius: 57,
            }],
            temperature_raw: Some("+QTEMP: 57,51,51".to_owned()),
        };
        // A different epoch breaks the correlation even when the values match.
        let view = probe_view(&state, Some(&cellular_with(Some(numbers.clone()))), None);
        assert!(!view.captured);
        assert_eq!(view.iccid_full, None, "plaintext must not leak on mismatch");
        assert_eq!(
            view.serving_cell_raw, None,
            "raw line must not leak on mismatch"
        );
        assert_eq!(
            view.temperature_raw, None,
            "raw temperature line must not leak on mismatch"
        );
        assert!(view.temperature_sensors.is_empty());
        // A cellular snapshot whose values differ breaks the correlation too.
        let view = probe_view(&state, Some(&cellular_with(None)), Some(DeviceEpoch(3)));
        assert!(!view.captured);
        assert_eq!(view.numbers_status, FeatureStatus::NotProbed);
        // So does a snapshot whose temperature differs from the recorded reading.
        let mut drifted = cellular_with(Some(numbers.clone()));
        drifted.temperature_celsius = Some(59);
        let view = probe_view(&state, Some(&drifted), Some(DeviceEpoch(3)));
        assert!(
            !view.captured,
            "a reading from another cycle must not describe this snapshot"
        );
        // Matching snapshot and epoch expose the correlated secrets.
        let mut cell = cellular_with(Some(numbers));
        cell.sim_identity = Some(SimIdentity {
            iccid_masked: "8986…2345".to_owned(),
            fingerprint: [1; 8],
        });
        let view = probe_view(&state, Some(&cell), Some(DeviceEpoch(3)));
        assert!(view.captured);
        assert_eq!(view.iccid_full.as_deref(), Some("89860123456789012345"));
        assert_eq!(view.serving_cell_raw.as_deref(), Some("+QENG: raw"));
        assert_eq!(view.temperature_raw.as_deref(), Some("+QTEMP: 57,51,51"));
        assert_eq!(
            view.temperature_sensors,
            [SensorTemperature {
                name: None,
                celsius: 57,
            }]
        );
        // Without any cellular evidence there is nothing to annotate.
        let view = probe_view(&state, None, Some(DeviceEpoch(3)));
        assert!(!view.captured);
    }

    #[test]
    fn default_probe_state_is_neutral() {
        let state = FeatureProbeState::default();
        assert_eq!(state.numbers_status, FeatureStatus::NotProbed);
        assert_eq!(state.iccid_status, FeatureStatus::NotProbed);
        assert_eq!(state.serving_cell_status, FeatureStatus::NotProbed);
        assert!(state.iccid_full.is_none());
        let view = probe_view(&state, Some(&cellular_with(None)), Some(DeviceEpoch(1)));
        assert!(!view.captured);
        assert_eq!(view.iccid_full, None);
        assert_eq!(view.serving_cell_raw, None);
    }
}
