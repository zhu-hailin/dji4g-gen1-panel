# DJI 4G Gen1 Panel Design Specification

- Status: approved for implementation planning
- Date: 2026-09-03
- Target repository: `C:\Users\22050\Desktop\dji4g-panel`
- Product type: unofficial open-source Windows desktop utility
- License target: `MIT OR Apache-2.0`

## 1. Product objective

Build a small, professional, reliable Windows control and diagnostic panel for the first-generation DJI 4G cellular dongle. The application must answer one primary question without ambiguity: can this specific dongle currently act as a usable Internet uplink for Windows?

The application also provides a separate Windows Mobile Hotspot status/control surface, read-only cellular and network diagnostics, and a small set of explicitly modeled repair operations. It must never infer success from device presence, signal strength, a PDP address, or the Windows global network icon alone.

The project is independent and unofficial. It must not imply endorsement by DJI, Baiwang, Quectel, Microsoft, or a mobile carrier.

## 2. Supported scope

### 2.1 Officially supported device

V1 supports exactly one USB device profile:

```text
USB VID: 0x2CA3
USB PID: 0x4006
```

The product is first-generation-only. PID `0x4009`, generic Quectel modules, other DJI generations, and unrelated cellular adapters are not supported. A device with the same vendor ID but another product ID is reported as `UnsupportedDevice`; the program performs no AT writes or repairs against it.

The profile is centralized in the domain layer so identity and safety rules are not duplicated. V1 contains exactly one profile; the abstraction is not a compatibility promise for future generations.

### 2.2 Windows targets

- Architecture: x86-64 only for V1.
- Official target: Windows 11 x64.
- Compatibility target: Windows 10 22H2 x64 where the required APIs and installed hardware drivers work.
- UI languages: Simplified Chinese and English from the first release.

### 2.3 Non-goals

V1 does not provide:

- A raw or arbitrary AT terminal.
- Firmware, QCN, NV, IMEI, band-lock, or carrier-lock operations.
- Automatic SIM PIN or PUK submission.
- Automatic driver download or silent driver installation.
- Bundled DJI, Baiwang, or Quectel drivers, firmware, logos, or manuals without explicit redistribution permission.
- Support for non-Windows operating systems.
- Support for second-generation DJI modules.
- Background telemetry, analytics, cloud accounts, or automatic self-update.
- Decorative dashboards, historical traffic charts, or speed-test claims.

## 3. Approved technology stack

- Language: stable Rust, Rust 2024 Edition.
- GUI: `egui` with `eframe`; no WebView and no JavaScript frontend.
- Async runtime: Tokio.
- Serial I/O: `tokio-serial` behind an application-owned port trait.
- Windows APIs: Microsoft `windows` crate for Win32, COM, and WinRT.
- Serialization/configuration: Serde and TOML.
- Logging: `tracing` with bounded rolling files.
- Domain errors: `thiserror`; application boundaries may use `anyhow` with stable user-facing error codes.
- System tray: `tray-icon` or a thin `Shell_NotifyIconW` adapter selected by the platform spike.
- Project license: dual `MIT OR Apache-2.0`.

No Qt, Tauri, Electron, or browser runtime is used.

## 4. Repository and component architecture

```text
dji4g-panel/
├─ Cargo.toml
├─ Cargo.lock
├─ rust-toolchain.toml
├─ LICENSE-MIT
├─ LICENSE-APACHE
├─ README.md
├─ SECURITY.md
├─ apps/
│  ├─ panel/                 # asInvoker GUI and tray application
│  └─ helper/                # one-shot elevated helper
├─ crates/
│  ├─ domain/                # pure models and availability classifier
│  ├─ application/           # orchestration, reducer, action planning
│  ├─ at-protocol/           # typed AT commands and streaming parser
│  ├─ windows-platform/      # PnP, COM, IP Helper, WinRT hotspot
│  └─ ipc/                   # authenticated bounded helper protocol
├─ tests/
│  ├─ fixtures/at/
│  ├─ state/
│  └─ hardware/
├─ packaging/
└─ docs/
   └─ superpowers/
      ├─ specs/
      └─ plans/
```

Dependency direction is one-way:

```text
panel -> application -> domain
panel -> windows-platform -> domain
panel -> ipc -> domain
helper -> ipc -> domain
helper -> windows-platform privileged adapters
```

The panel cannot link privileged implementation modules. The helper cannot render UI or accept open-ended commands.

## 5. Runtime and data flow

The UI thread owns all eframe/egui state and never blocks on device, serial, DNS, route, or hotspot calls. A named background thread initializes the Windows runtime requirements and runs the Tokio runtime. Blocking Win32 enumeration occurs through bounded background work.

```text
PnP and network notifications + periodic monitor
                    |
                    v
Device / AT / RNDIS / DNS / route / bound reachability observations
                    |
                    v
Pure reducer -> immutable AppSnapshot -> UI and tray
```

User operations flow in the opposite direction:

```text
UI command -> controller -> action plan -> explicit confirmation
-> precondition recheck -> normal or elevated executor
-> post-operation re-enumeration and verification -> audit result
```

The UI consumes immutable snapshots. It does not own device handles or COM ports and does not wait on mutexes held by backend work.

## 6. Core state model

```rust
pub enum Availability {
    Detecting,
    Available,
    Limited(LimitedReason),
    Unavailable(UnavailableReason),
    NotDetected,
    UnsupportedDevice,
}

pub enum HotspotStatus {
    Unsupported(HotspotUnsupportedReason),
    Off,
    Starting,
    On { clients: Option<u32> },
    Stopping,
    Failed { code: ErrorCode },
}

pub struct AppSnapshot {
    pub revision: u64,
    pub observed_at: SystemTime,
    pub freshness: Freshness,
    pub availability: Availability,
    pub hotspot: HotspotStatus,
    pub device: Option<DeviceSnapshot>,
    pub cellular: Option<CellularSnapshot>,
    pub network: Option<NetworkSnapshot>,
    pub active_operation: Option<OperationSnapshot>,
    pub issues: Vec<Issue>,
}
```

`Availability` and `HotspotStatus` are orthogonal. A disabled hotspot does not make the cellular uplink unavailable.

### 6.1 Availability evidence chain

Evidence is collected in layers:

1. USB/PnP identity and problem status.
2. AT control-plane identity, SIM, registration, attach, APN, and PDP status when the AT channel is available.
3. Windows data-plane adapter identity, link state, DHCP, address, gateway, DNS, and routes.
4. Gateway, DNS, and public TCP/HTTP probes explicitly bound to the first-generation adapter.
5. Global route comparison used only to explain VPN/TUN competition, never to prove module connectivity.

The classifier uses evidence from one device epoch only. Replug, removal, re-enumeration, or identity drift invalidates every old positive observation.

### 6.2 Classification rules

- `Available`: the target adapter has a valid non-link-local address and route, and a bound public data probe plus bound DNS evidence succeed.
- `Limited`: public IP/data succeeds but DNS fails; only one required protocol family works; a VPN/TUN owns the system default route; AT information is unavailable while RNDIS data works; or the probe evidence is incomplete but not definitively failed.
- `Unavailable`: SIM/registration is definitively rejected, no usable Windows address or route exists, or independent bound public probes fail across two consecutive cycles.
- `NotDetected`: a successful current enumeration proves that PID `0x4006` is absent.
- `UnsupportedDevice`: a related but out-of-scope device is found.
- `Detecting`: startup, recent insertion, re-enumeration, post-write verification, or expired evidence.

Permission denial is an error, not evidence that the device is absent. A PDP address is not proof that Windows can use the Internet. A global Windows online state is not proof that the module path works.

Positive evidence expires. A stale green state is removed before a refresh is attempted. Physical removal removes green within one second after notification.

## 7. First-generation device discovery

Discovery uses SetupAPI and Configuration Manager APIs:

- Match hardware IDs against `USB\VID_2CA3&PID_4006`.
- Read parent/child topology, ContainerId, devnode status, and problem codes.
- Enumerate COM and NET device interfaces.
- Correlate interfaces through parent ancestry and ContainerId.
- Map the NET devnode to an IP Helper adapter using stable GUID/LUID data, not FriendlyName.
- Rebuild the mapping after every device epoch.

COM numbers, MI values, adapter display names, ifIndex values, and `192.168.225.x` addresses are observations, not persistent identities.

## 8. AT protocol and serial ownership

Exactly one `AtSessionActor` owns the serial handle. Requests are typed and pass through a bounded queue. Repeated read refreshes may be coalesced; operation results and audit events cannot be dropped.

### 8.1 AT port selection

Candidate ports must belong to the target device. Known DM/DIAG and NMEA ports are excluded. An explicitly named AT port is preferred; a modem port may be used only as a verified fallback. An unknown port is not probed blindly.

Safe handshake:

1. Open exclusively at the validated port configuration.
2. Send `AT` and require a final `OK` within the handshake timeout.
3. Send `ATI` and require a printable identity plus `OK`.
4. If multiple candidates remain equally valid, enter `AmbiguousPort` and disable writes.

The binding stores device epoch, interface path, ContainerId, and returned identity. It never stores COM5 as a rule.

### 8.2 Read-only command whitelist

V1 may model:

- `AT`, `ATI`, `AT+CGMI`, `AT+CGMM`, `AT+CGMR`
- `AT+GSN` or `AT+CGSN` with masked display
- `AT+CPIN?`
- `AT+CSQ`
- `AT+COPS?`
- `AT+CEREG?`, with `AT+CGREG?` and `AT+CREG?` fallbacks
- `AT+CGATT?`
- `AT+CGDCONT?`
- `AT+CGACT?`
- `AT+CGPADDR` or a validated CID query form
- `AT+CGCONTRDP` when confirmed supported by the installed firmware
- `AT+QNWINFO` when confirmed supported
- `AT+QCFG="usbnet"` as an exact query
- `AT+CEER` after an attach or PDP failure

Optional commands returning a normal unsupported error degrade one detail field; they do not make the device unavailable.

The parser supports fragmented input, echo on/off, multiline responses, known URCs interleaved with command responses, final result codes, and removal before a final result. Binary or sustained unrecognized data causes the port to be closed as a probable wrong-port selection.

Idempotent reads may retry once after a short quiet period. Writes never retry automatically.

## 9. UI design

The default logical window is approximately 420 by 380 pixels, DPI-aware, non-maximizable, and scrollable when accessibility scaling requires more space.

The top area permanently displays:

- Colored availability state.
- One-sentence reason.
- Last observation freshness.

The compact overview shows device identity, carrier/RAT, signal, Windows address, DNS status, and hotspot state. Primary actions are `Refresh`, `Diagnostics`, and `Repair`. No chart or animation is required.

The diagnostics view follows the evidence chain in section 6.1 and shows passed, failed, unavailable, and unexecuted checks separately.

Closing the window hides it to the tray. Tray commands are Open, Refresh Now, Hotspot Status, and Exit. If tray creation fails, the main window is shown. A second instance activates the first instance rather than starting another monitor.

## 10. Startup behavior

Autostart is optional and disabled by default. When enabled, login starts the application with `--autostart`, which opens directly in the tray without flashing a window.

- Packaged build: use a package StartupTask.
- Traditional installed or development build: use only the application's named HKCU Run value.
- A moved executable or externally deleted registration is reported as configuration drift.
- Disabling autostart removes only the application's own registration.

## 11. Windows Mobile Hotspot

Hotspot control uses `NetworkOperatorTetheringManager`. Connection profiles are enumerated and matched to the PID `0x4006` RNDIS adapter; the global Internet connection profile is not trusted because it may select Meta/TUN or another uplink.

The package manifest declares `wiFiControl`. Runtime capability is checked every time before an operation because hardware, policy, and connection state may change.

The hotspot UI reports Off, Starting, On, Stopping, Unsupported, or Failed with a stable reason. Failure to control the hotspot never changes cellular `Availability` to unavailable.

M0 must prove the required package identity and manifest behavior on the target machines before hotspot support is considered implemented.

## 12. Repair operations

### 12.1 Low-risk Windows operations

- Refresh and re-detect.
- Renew DHCP for the identified RNDIS adapter.
- Apply a modeled DNS profile and restore the exact previous mode.
- Start or stop Mobile Hotspot after capability and source-profile checks.

### 12.2 Disruptive operations

- Restart the identified RNDIS adapter as one transaction with best-effort re-enable.
- Re-enumerate the target physical device.
- Restart the cellular module with the validated typed command.

### 12.3 Persistent module operations

- Modify an existing APN context only after complete parse, pre-read, inactive-context validation, confirmation, write, and readback.
- Switch only between versioned first-generation USB-network profiles proven on hardware. Raw numeric values are never accepted from the UI.

The known project profile may map the validated first-generation NDIS behavior to raw `usbnet=0` and the observed ECM behavior to `usbnet=1`. Values 2 and 3 are not writable in V1. UI labels describe project-verified profiles, not a universal Quectel interpretation.

### 12.4 Explicitly forbidden operations

- Raw AT input.
- Factory reset, `AT&W`, arbitrary `QCFG`, USB identity edits.
- Firmware, QCN, NV, IMEI, band, or carrier-lock modification.
- SIM PIN/PUK submission.
- Automated attach/PDP toggling.
- Automatic write retries.
- Driver installation.

## 13. Confirmation, execution, and audit

Every side effect follows a two-phase protocol:

1. Create an `ActionPlan` from a specific snapshot revision.
2. Display target, expected effects, interruption, risks, and elevation requirement.
3. Receive explicit user confirmation.
4. Re-read identity and preconditions; reject stale plans.
5. Execute once.
6. Re-enumerate and read back the state.
7. Record `Applied`, `Failed`, or `OutcomeUnknown`.

A write timeout is `OutcomeUnknown`, not an automatic failure or success. The program must never resend a timed-out write without a new user-approved plan.

Audit logs contain operation type, stable target identity, before/after hashes, timestamps, result code, and rollback result. Sensitive values are redacted.

## 14. Privilege separation and IPC

`dji4g-panel.exe` is `asInvoker`. `dji4g-helper.exe` is launched with `runas` only after confirmation and executes one privileged action before exiting.

IPC uses a per-operation named pipe with:

- A random name and bounded lifetime.
- Local-client-only mode.
- An explicit DACL for the current user, Administrators, and SYSTEM.
- Server/client PID verification.
- Protocol version, request ID, issue/expiry times, size limit, and unknown-field rejection.
- Strongly typed operation enums.

The helper re-enumerates the target and proves PID `0x4006` plus expected identity before acting. It never trusts a display name supplied by the UI.

The helper accepts no executable command, script, arbitrary registry path, arbitrary file path, arbitrary device, or raw AT command.

An unsigned helper in a user-writable portable directory is never elevated. Full repair features require a signed installation placing executables in an administrator-protected location.

## 15. Configuration, logs, and privacy

- Configuration: `%APPDATA%\Dji4GPanel\config.toml`.
- Logs: `%LOCALAPPDATA%\Dji4GPanel\logs\`.
- Config writes are atomic and versioned.
- Corrupt configuration is preserved with a timestamped name and replaced with safe defaults.
- Logs rotate with bounded retention and total size.
- Full IMEI, IMSI, ICCID, phone numbers, PIN/PUK, raw serial buffers, and full config files are not logged.
- Diagnostic export is explicit, redacted by default, and includes a human-readable report plus structured JSON.
- No diagnostic data is uploaded automatically.

## 16. Packaging and release

The full feature target is a signed packaged Windows release with `wiFiControl`. M0 validates MSIX package identity, hotspot control, and the elevation model. If MSIX cannot safely coexist with the one-shot helper in GitHub sideload distribution, the predetermined fallback is a signed traditional installer that places binaries under Program Files and registers only the minimal package identity required for hotspot capability.

V1 does not promise a full-feature portable build. Development binaries may provide read-only diagnostics, but they must label unavailable packaged/elevated features accurately.

A public stable release requires trusted Authenticode signing. Without a trusted certificate, builds are labeled development/unsigned and are not represented as a stable installable release.

GitHub Actions on `windows-latest` performs formatting, Clippy with warnings denied, unit/integration tests, locked release build, dependency/license checks, SBOM generation, hashes, and artifact provenance. Hardware-in-the-loop tests run separately on the project machine and produce a recorded acceptance report.

V1 does not self-update. It may open the GitHub Releases page on explicit user action.

## 17. Testing strategy

### 17.1 Pure and simulated tests

- Table-driven availability classification, freshness, and device-epoch invalidation.
- AT parser fixtures for echo, line endings, fragmentation, multiline responses, URCs, result errors, timeout, and removal.
- Properties proving only modeled commands can be rendered.
- Injection rejection for APN and IPC data.
- Writes never automatically retry.
- Controller scenarios for insert, remove, COM renumbering, TUN route competition, DNS-only failure, stale confirmation, and cancellation.
- IPC rejection for unknown fields, oversized messages, expired requests, wrong PID, wrong device, and second clients.

### 17.2 Windows integration spikes

M0 includes executable proofs for:

1. PnP root/children, COM, NET, ContainerId, and Code 28 enumeration.
2. Stable mapping from PnP NET devnode to IP Helper adapter and WinRT NetworkAdapterId.
3. Bound DNS and public TCP/HTTP probes that cannot fall back to Meta/TUN.
4. Packaged hotspot capability and start/stop behavior.
5. One-shot UAC helper and named-pipe identity checks.
6. Tray recovery, single instance, and autostart with spaces in the install path.

### 17.3 Hardware-in-the-loop acceptance

- Cold insert, hot insert, removal, direct USB, dock, port change, and COM renumbering.
- Thirty insert/remove cycles with zero writes to DM/NMEA/wrong ports.
- Normal SIM, no SIM, locked test SIM, searching, registered, roaming, and rejected states where test hardware permits.
- Normal DHCP, APIPA, no gateway, IPv4-only, IPv6-only, and DNS failure.
- Meta/TUN enabled with a lower-metric default route while all module probes remain bound.
- Public IP succeeds while DNS fails, producing `Limited` rather than green or fully unavailable.
- Hotspot supported, policy-disabled, no Wi-Fi adapter, start failure, stop failure, and client connection.
- APN round-trip on a test SIM.
- Validated `usbnet` profile switch on a dedicated test device, including re-enumeration and readback.
- Module restart, UAC cancellation, device removal after confirmation, and best-effort rollback.
- Sleep/resume, Explorer restart, logon autostart, duplicate launch, and serial port contention.

### 17.4 Acceptance thresholds

- PnP-stable port rebinding p95 <= 10 seconds.
- Stable network availability result <= 15 seconds.
- Physical removal clears green <= 1 second after notification.
- No write command can originate from an unmodeled string.
- Every side-effect result is `Applied`, `Failed`, or `OutcomeUnknown` with evidence.
- CI success is never used as a substitute for hardware acceptance.

## 18. Delivery milestones

1. M0 platform feasibility: PnP mapping, bound probes, package hotspot, elevation, tray/autostart.
2. M1 domain core and simulators: state classifier, typed AT protocol, fake platform ports.
3. M2 read-only hardware diagnostics: first-generation discovery, serial actor, RNDIS/network evidence.
4. M3 desktop shell: small window, tray, single instance, autostart, localization, privacy-safe logs.
5. M4 hotspot: capability detection, selected source profile, start/stop, separate status.
6. M5 controlled repairs: action plans, confirmations, helper, IPC, rollback, audit.
7. M6 release: installer, signing path, CI, SBOM, licenses, documentation, acceptance report.

M0 is a stop gate. If package identity, hotspot, or secure elevation cannot be proven, the release architecture is revised before product UI implementation proceeds.

## 19. Definition of done

The objective is complete only when:

- The desktop repository contains the documented Rust workspace and builds from a clean checkout.
- All automated tests pass on Windows x64.
- The application identifies only PID `0x4006` as supported.
- The small window and tray reliably show fresh availability and hotspot states.
- Bound probes distinguish the module from phone, Ethernet, VPN, and Meta/TUN paths.
- AT reads and all repair operations obey the approved whitelist and confirmation rules.
- Autostart is optional, defaults off, and starts silently to the tray.
- The required hardware-in-the-loop matrix has an evidence-backed acceptance report with passed, failed, and unexecuted items separated.
- Release artifacts include licenses, hashes, SBOM, and accurate signed/unsigned labeling.
- README and SECURITY documentation state first-generation-only, unofficial status, privacy behavior, recovery steps, and known limitations.
- No required item is represented as complete solely because compilation or mocks passed.
