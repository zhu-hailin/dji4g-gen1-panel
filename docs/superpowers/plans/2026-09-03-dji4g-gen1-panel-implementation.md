# DJI 4G Gen1 Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build, package, and verify a small Windows desktop application that detects only the first-generation DJI 4G module, reports whether its own data path is usable, exposes separate Windows Mobile Hotspot state/control, supports optional autostart, and offers only modeled safe repair actions.

**Architecture:** A non-elevated `eframe` panel consumes immutable snapshots produced by an application controller. Pure domain and AT crates are separated from Windows PnP/network/WinRT adapters; a one-shot elevated helper accepts only versioned typed IPC operations. Hardware evidence is tied to a device epoch so unplug/replug and Meta/TUN route competition cannot create a false green state.

**Tech Stack:** Rust 2024, Cargo workspace, egui/eframe, Tokio, windows-rs, tokio-serial, Serde/TOML, tracing, thiserror, proptest, cargo-deny, PowerShell packaging scripts, Windows x64/MSVC.

**Spec:** `docs/superpowers/specs/2026-09-03-dji4g-gen1-panel-design.md`

## Global Constraints

- Officially supported device is exactly `USB\VID_2CA3&PID_4006`; related devices are read-only `UnsupportedDevice` and never mutation targets.
- Official OS is Windows 11 x64; Windows 10 22H2 x64 is a compatibility target.
- The main process is always `asInvoker`; elevation is one-shot and operation-scoped.
- No raw AT terminal, raw shell command, arbitrary registry/file/device target, driver installer, firmware/QCN/NV/IMEI/band operation, SIM secret submission, or automatic AT write retry.
- Availability and Mobile Hotspot state are independent.
- A positive connectivity result must be bound to the first-generation adapter and must not fall back through phone tethering, Ethernet, Wi-Fi, VPN, or Meta/TUN.
- Positive evidence from an earlier device epoch is invalid immediately after removal or re-enumeration.
- Autostart defaults off and starts to the tray without flashing a window.
- Simplified Chinese and English are required for user-visible strings.
- Development/unsigned artifacts must be labeled accurately; stable distribution requires trusted Authenticode signing.
- Hardware-dependent acceptance is reported separately from automated test and compile results.

## Planned File Map

```text
Cargo.toml                              workspace members and shared dependency versions
rust-toolchain.toml                     stable MSVC toolchain, rustfmt and clippy
.cargo/config.toml                      Windows x64 target defaults
deny.toml                               dependency/source/license policy
LICENSE-MIT / LICENSE-APACHE            dual project license
README.md / SECURITY.md                 usage, limits, privacy, vulnerability reporting

crates/domain/src/
  lib.rs                                public domain exports
  device.rs                             first-generation identity and device epochs
  cellular.rs                           SIM/registration/signal/PDP observations
  network.rs                            adapter, route, DNS and bound-probe evidence
  availability.rs                       pure classifier and reasons
  hotspot.rs                            hotspot states and failure reasons
  action.rs                             typed action plans and outcomes
  snapshot.rs                           immutable application snapshot
  error.rs                              stable machine-readable error codes

crates/at-protocol/src/
  lib.rs                                public AT API
  command.rs                            closed command enum and renderer
  parser.rs                             streaming line/URC/final-code parser
  model.rs                              typed parsed responses
  redact.rs                             sensitive-field redaction

crates/application/src/
  lib.rs                                public application API
  ports.rs                              platform traits
  reducer.rs                            events to immutable snapshots
  monitor.rs                            refresh/device-epoch orchestration
  controller.rs                         UI command and action-plan workflows
  confirmation.rs                       expiring confirmation tokens

crates/windows-platform/src/
  lib.rs                                Windows adapter composition
  pnp.rs                                SetupAPI/Configuration Manager inventory
  adapter.rs                            PnP NET devnode to IP Helper mapping
  probe.rs                              adapter-bound DNS/TCP/HTTP evidence
  serial.rs                             candidate selection and serial actor
  hotspot.rs                            WinRT tethering adapter
  autostart.rs                          StartupTask/HKCU Run backend
  single_instance.rs                    mutex and activation pipe
  privilege.rs                          ShellExecuteEx one-shot helper launcher

crates/ipc/src/
  lib.rs                                public bounded IPC API
  protocol.rs                           versioned request/response enums
  framing.rs                            length-delimited Serde framing
  security.rs                           nonce, expiry, size and peer validation
  client.rs / server.rs                 single-use named-pipe endpoints

apps/panel/src/
  main.rs                               bootstrap, single instance, runtime, eframe
  app.rs                                snapshot-driven eframe App
  ui/overview.rs                        permanent availability and overview
  ui/diagnostics.rs                     layered evidence page
  ui/repairs.rs                         confirmation and operation outcomes
  ui/settings.rs                        autostart, language and privacy settings
  tray.rs                               tray lifecycle and commands
  localization.rs                       zh-CN and en-US catalogs
  config.rs                             versioned atomic TOML settings
  logging.rs                            bounded redacted tracing setup

apps/helper/src/main.rs                 one typed privileged operation then exit
packaging/msix/Package.appxmanifest     package identity, wiFiControl, startup task
packaging/scripts/build-msix.ps1        reproducible development MSIX build
packaging/scripts/verify-release.ps1    hashes, signatures, required-file checks
.github/workflows/ci.yml                Windows fmt/clippy/test/build/security checks
tests/hardware/README.md                explicit HIL procedure and evidence format
tests/hardware/report-template.md       passed/failed/unexecuted acceptance record
```

---

### Task 1: Install the official Windows Rust toolchain and bootstrap the workspace

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.cargo/config.toml`
- Create: `.gitignore`
- Create: `LICENSE-MIT`
- Create: `LICENSE-APACHE`
- Create: each crate/app `Cargo.toml` and minimal `src/lib.rs` or `src/main.rs`

**Interfaces:**
- Produces workspace packages `dji4g-domain`, `dji4g-at-protocol`, `dji4g-application`, `dji4g-windows-platform`, `dji4g-ipc`, `dji4g-panel`, and `dji4g-helper`.
- Pins one Cargo lockfile and the `x86_64-pc-windows-msvc` target.

- [ ] **Step 1: Record the current prerequisite failure**

Run `rustc --version`, `cargo --version`, and `where.exe cl.exe`.

Expected before installation on this machine: commands are missing.

- [ ] **Step 2: Install only official toolchains**

Run:

```powershell
winget install --id Rustlang.Rustup --exact --source winget --accept-package-agreements --accept-source-agreements
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget --accept-package-agreements --accept-source-agreements --override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable-x86_64-pc-windows-msvc
rustup component add rustfmt clippy
```

Start a fresh PowerShell process and expect `rustc --version --verbose` and `cargo --version` to succeed.

- [ ] **Step 3: Create the workspace manifests and minimal compile targets**

Root manifest must use resolver 3 and centralized metadata:

```toml
[workspace]
resolver = "3"
members = [
  "apps/panel", "apps/helper",
  "crates/domain", "crates/application", "crates/at-protocol",
  "crates/windows-platform", "crates/ipc",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
rust-version = "1.85"
```

Every package inherits workspace version, edition, and license. `panel` and `helper` use `#![windows_subsystem = "windows"]` only in non-test release builds so test failures remain visible.

- [ ] **Step 4: Verify bootstrap**

Run `cargo metadata --no-deps --format-version 1`, `cargo fmt --all -- --check`, and `cargo check --workspace --all-targets`.

Expected: all commands succeed and `Cargo.lock` is created.

- [ ] **Step 5: Commit**

Stage only the bootstrap files and commit `build: bootstrap Rust workspace`.

### Task 2: Implement the pure first-generation domain and availability classifier

**Files:**
- Create: `crates/domain/src/{lib,device,cellular,network,availability,hotspot,action,snapshot,error}.rs`
- Create: `crates/domain/tests/availability_matrix.rs`
- Create: `crates/domain/tests/action_safety.rs`

**Interfaces:**
- Produces `DeviceProfile::DJI_GEN1`, `DeviceEpoch(u64)`, `Evidence<T>`, `Availability`, `HotspotStatus`, `AppSnapshot`, `ActionPlan`, and `OperationOutcome`.
- Produces `fn classify(input: &ClassificationInput, now: SystemTime) -> AvailabilityDecision`.

- [ ] **Step 1: Write failing identity and classifier tests**

Tests must assert:

```rust
assert!(DeviceProfile::DJI_GEN1.matches(0x2CA3, 0x4006));
assert!(!DeviceProfile::DJI_GEN1.matches(0x2CA3, 0x4009));
assert_eq!(classify(&bound_public_and_dns_ok(), now).status, Availability::Available);
assert!(matches!(classify(&public_ok_dns_failed(), now).status, Availability::Limited(LimitedReason::DnsFailure)));
assert_eq!(classify(&old_epoch_success_after_replug(), now).status, Availability::Detecting);
assert_eq!(classify(&global_phone_only(), now).status, Availability::Unavailable(UnavailableReason::NoBoundReachability));
```

Run `cargo test -p dji4g-domain --test availability_matrix`; expect compile failure because the types do not exist.

- [ ] **Step 2: Implement closed domain types**

Use these signatures:

```rust
pub const DJI_GEN1: DeviceProfile = DeviceProfile { vid: 0x2CA3, pid: 0x4006 };

pub struct Evidence<T> {
    pub epoch: DeviceEpoch,
    pub observed_at: SystemTime,
    pub ttl: Duration,
    pub source: EvidenceSource,
    pub value: T,
}

pub fn classify(input: &ClassificationInput, now: SystemTime) -> AvailabilityDecision;
```

The classifier rejects expired evidence and evidence whose epoch differs from `input.current_epoch`. Bound success is stronger than unavailable AT control; global connectivity is explanatory only.

- [ ] **Step 3: Add action safety tests and implementation**

`ActionKind` is a closed enum containing refresh, DHCP renew, modeled DNS profile, adapter restart, device re-enumeration, module restart, APN edit, verified USB-network profile, and hotspot toggle. `ActionPlan` includes snapshot revision, epoch, stable target identity, before-state hash, expiry, disruption/risk level, and elevation requirement.

Tests prove unsupported PID and stale epoch cannot create an executable plan.

- [ ] **Step 4: Verify and commit**

Run domain tests and Clippy with warnings denied; commit `feat: add first-generation domain state model`.

### Task 3: Implement the typed AT command renderer and streaming parser

**Files:**
- Create: `crates/at-protocol/src/{lib,command,parser,model,redact}.rs`
- Create: `crates/at-protocol/tests/{render_whitelist,parser_fixtures,redaction}.rs`
- Create: `tests/fixtures/at/*.txt`

**Interfaces:**
- Consumes `DeviceEpoch` and stable error codes from `dji4g-domain`.
- Produces `AtCommand`, `AtResponse`, `AtEvent`, `StreamingParser`, `Apn`, and `VerifiedUsbNetProfile`.
- Does not expose `send_raw_at(String)` or a public raw renderer.

- [ ] **Step 1: Write failing renderer safety tests**

The command enum contains the exact read queries from the spec plus three modeled writes:

```rust
pub enum AtCommand {
    Attention, Identity, Manufacturer, Model, Revision,
    SimState, SignalQuality, Operator, EpsRegistration, PacketAttach,
    PdpContexts, PdpActivation, PdpAddresses, UsbNetQuery, ExtendedError,
    RestartModule,
    SetApn { cid: u8, apn: Apn },
    SetUsbNetProfile(VerifiedUsbNetProfile),
}
```

Tests enumerate every variant, compare exact bytes, and reject APNs containing quote, comma, CR/LF, control characters, empty text, or more than 100 ASCII bytes.

- [ ] **Step 2: Implement the minimal closed renderer**

`AtCommand::wire_bytes()` is crate-private. `VerifiedUsbNetProfile` maps `DjiNdis` to raw value 0 and `Ecm` to raw value 1; no representation exists for 2 or 3. `AtCommand::is_write()` and `retry_policy()` return `Never` for all writes.

- [ ] **Step 3: Write parser fixture tests**

Cover echo on/off, CR/LF variations, byte-by-byte fragmentation, multiline responses, interleaved known URCs, `+CME ERROR`, timeout/incomplete final line, NMEA text, binary input, and 4097-byte lines. Tests must show a query response sharing the `+CEREG:` prefix is not misclassified as an unrelated URC.

- [ ] **Step 4: Implement and fuzz the streaming parser**

```rust
pub struct StreamingParser { /* bounded buffer and transaction schema */ }
pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<AtEvent>, ProtocolError>;
pub fn finish_timeout(&mut self) -> ProtocolError;
```

No input path allocates based on an untrusted advertised length; lines over 4096 bytes and binary control data fail closed.

- [ ] **Step 5: Verify and commit**

Run AT protocol tests and Clippy with warnings denied; commit `feat: add typed AT protocol`.

### Task 4: Implement Windows PnP inventory and safe AT-port selection

**Files:**
- Create: `crates/windows-platform/src/{lib,pnp,serial}.rs`
- Create: `crates/windows-platform/tests/pnp_fixture.rs`
- Create: `apps/panel/src/bin/pnp_spike.rs`

**Interfaces:**
- Produces `trait DeviceInventory { async fn scan(&self) -> Result<InventorySnapshot>; }`.
- Produces `DjiDevice`, `ComCandidate`, `NetCandidate`, and `select_at_candidate(&[ComCandidate]) -> Result<SelectedPort, PortSelectionError>`.
- A selected port is valid only when its ancestry resolves to PID `0x4006` and its function is dedicated AT or verified modem fallback.

- [ ] **Step 1: Write failing topology and selection tests**

Fixtures include direct USB and dock paths, COM renumbering, Code 28, unsupported PID 4009, dedicated AT + DM + NMEA, modem-only fallback, and two ambiguous AT candidates. Assert DM/NMEA are never returned.

- [ ] **Step 2: Implement SetupAPI/Configuration Manager enumeration**

Use `SetupDiGetClassDevsW`, `SetupDiEnumDeviceInfo`, `SetupDiGetDevicePropertyW`, `CM_Get_Parent`, and device-interface enumeration. Read hardware IDs, instance ID, parent, ContainerId, class, problem code, interface path, and serial port name. Centralize UTF-16 and variable-buffer helpers and bound all retry loops.

- [ ] **Step 3: Implement the serial actor**

Exactly one actor owns the handle. It performs `AT` then `ATI` handshake only on a selected candidate, has a bounded queue of 64, coalesces duplicate reads, allows one retry only for idempotent reads, never retries writes, and closes immediately when its epoch is invalidated.

- [ ] **Step 4: Run the read-only spike on current hardware**

Run `cargo run -p dji4g-panel --bin pnp_spike -- --json`. Save a redacted sample under `tests/hardware/local-pnp-sample.json` only if it contains no full serial/IMEI. Expected: exactly one root PID 4006, correlated COM/NET functions, explicit problem codes, and no bytes sent to DM/NMEA.

- [ ] **Step 5: Verify and commit**

Run Windows platform tests and Clippy; commit `feat: discover DJI Gen1 Windows interfaces`.

### Task 5: Implement stable adapter mapping and interface-bound reachability

**Files:**
- Create: `crates/windows-platform/src/{adapter,probe}.rs`
- Modify: `crates/windows-platform/src/lib.rs`
- Create: `crates/windows-platform/tests/{adapter_mapping,probe_policy}.rs`
- Create: `apps/panel/src/bin/bound_probe_spike.rs`

**Interfaces:**
- Produces `AdapterIdentity { guid, luid, if_index, epoch, unicast_addresses }`.
- Produces `trait NetworkProbe { async fn observe(&self, target: &AdapterIdentity, policy: &ProbePolicy) -> Result<NetworkEvidence>; }`.
- `BoundProbeResult` includes chosen family, source address, interface index, route, DNS result, endpoint result, and timestamps.

- [ ] **Step 1: Write failing mapping/policy tests**

Fixtures prove friendly-name changes and ifIndex changes do not break re-discovery, while a mismatched PnP ancestry is rejected. A fake phone adapter and Meta/TUN default route must not satisfy DJI bound-probe success.

- [ ] **Step 2: Implement adapter observation**

Use `GetAdaptersAddresses` and `GetIpForwardTable2`; map PnP NET identity to adapter GUID/LUID, not FriendlyName. Reject APIPA-only IPv4 as a usable uplink. Record DHCP, gateways, DNS servers, metrics, and OperStatus.

- [ ] **Step 3: Implement bound probes**

Use `GetBestRoute2` constrained to the adapter. Bind IPv4 sockets with `IP_UNICAST_IF` and IPv6 with `IPV6_UNICAST_IF`; verify `getsockname` belongs to the target adapter. Use `DnsQueryEx` with `InterfaceIndex`. Probe two small HTTPS endpoints with strict connect/read caps; no unbound fallback is permitted.

- [ ] **Step 4: Run the Meta/TUN acceptance spike**

With Meta/TUN enabled and phone hotspot available, execute `bound_probe_spike --adapter-pid 4006 --json`. Then remove or disable only the DJI adapter and rerun. First run must report the actual DJI source address; second must fail rather than use phone/TUN.

- [ ] **Step 5: Verify and commit**

Run Windows platform tests and Clippy; commit `feat: add adapter-bound connectivity probes`.

### Task 6: Implement the application reducer, monitor, and two-phase controller

**Files:**
- Create: `crates/application/src/{lib,ports,reducer,monitor,controller,confirmation}.rs`
- Create: `crates/application/tests/{monitor_scenarios,confirmation_scenarios}.rs`

**Interfaces:**
- Consumes domain models, `DeviceInventory`, `NetworkProbe`, `AtPort`, `HotspotControl`, `AutostartControl`, and `PrivilegedExecutor` traits.
- Produces `UiCommand`, `BackendEvent`, `ControllerHandle`, and a `watch::Receiver<Arc<AppSnapshot>>`.

- [ ] **Step 1: Write failing scenario tests with fake ports and fake clock**

Cover startup, insert, removal, COM renumber, stale results arriving after removal, DNS-only failure, TUN route competition, AT busy while bound data succeeds, duplicate refresh coalescing, confirmation expiry, UAC cancellation, and removal after confirmation.

- [ ] **Step 2: Implement the pure reducer**

```rust
pub fn reduce(previous: &AppSnapshot, event: BackendEvent, now: SystemTime) -> AppSnapshot;
```

Every physical removal increments the epoch and clears all prior positive evidence before any refresh. Revision increases monotonically. Backend errors map to stable error codes and never panic the UI.

- [ ] **Step 3: Implement monitor and controller**

The controller receives a bounded `mpsc::Receiver<UiCommand>`, schedules periodic and notification-driven scans, and publishes immutable `Arc<AppSnapshot>` values. `PrepareAction` creates a plan; `ConfirmAction` rechecks snapshot revision, epoch, target and before hash before one execution.

- [ ] **Step 4: Verify and commit**

Run application tests and Clippy; commit `feat: orchestrate monitoring and safe actions`.

### Task 7: Build the compact bilingual snapshot-driven panel

**Files:**
- Modify: `apps/panel/src/main.rs`
- Create: `apps/panel/src/{app,localization}.rs`
- Create: `apps/panel/src/ui/{mod,overview,diagnostics,repairs,settings}.rs`
- Create: `apps/panel/tests/ui_state.rs`

**Interfaces:**
- Consumes snapshots and sends only `UiCommand` values.
- Produces a 420x380 DPI-aware non-maximizable window with permanent availability, reason and freshness display.

- [ ] **Step 1: Write failing UI-state tests**

Test the view model without opening a native window: every `Availability` has a color, Chinese and English title, one-sentence reason, and stale/detecting presentation. Test hotspot state does not change availability color.

- [ ] **Step 2: Implement localization as compile-time catalogs**

Define a closed `TextKey` enum and exhaustive `zh_cn(TextKey)` and `en_us(TextKey)` functions. Stable error codes are mapped in the UI; backend crates contain no localized prose.

- [ ] **Step 3: Implement the window**

Top area is always visible. Overview shows identity, carrier/RAT, signal, adapter address, DNS and hotspot. Diagnostics shows USB, AT, Windows data, route, bound public, and bound DNS rows with passed/failed/unavailable/unexecuted distinctions. Repairs are disabled when there is no valid action plan.

- [ ] **Step 4: Add a deterministic demo backend**

`--demo available|limited|unavailable|absent` feeds snapshots for UI review without hardware. It is compiled in debug builds and rejected in release builds.

- [ ] **Step 5: Verify and commit**

Run tests, Clippy and `cargo run -p dji4g-panel -- --demo limited`; capture a review screenshot under `docs/screenshots/dev-limited.png`; commit `feat: add compact bilingual desktop panel`.

### Task 8: Add tray lifecycle, single instance, settings, logs, and optional autostart

**Files:**
- Create: `apps/panel/src/{tray,config,logging}.rs`
- Create: `crates/windows-platform/src/{autostart,single_instance}.rs`
- Modify: `crates/windows-platform/src/lib.rs`
- Modify: `apps/panel/src/main.rs`
- Create: `apps/panel/tests/{config_roundtrip,localization_complete}.rs`

**Interfaces:**
- Produces `AutostartControl::{status,set_enabled}` and `SingleInstance::{acquire,activate_existing}`.
- Config schema is `ConfigV1 { language, autostart, start_minimized, active_probe, log_level }`; defaults have autostart false and start_minimized false.

- [ ] **Step 1: Write failing config and command-line tests**

Test defaults, atomic round-trip, corrupt-file preservation, `--autostart` forcing start-to-tray, quoted executable paths, and disabling only the program's own HKCU Run value.

- [ ] **Step 2: Implement configuration and privacy-safe logs**

Write TOML to a sibling temporary file, flush, and atomically replace. Preserve corrupt config with a timestamp. Configure bounded rolling logs under `%LOCALAPPDATA%\Dji4GPanel\logs`; redaction occurs before formatting.

- [ ] **Step 3: Implement tray and single instance**

Tray commands are Open, Refresh Now, Hotspot Status, Exit. Closing hides; explicit Exit terminates. If tray initialization fails the window stays visible. Handle `TaskbarCreated` or the selected tray crate's documented recreation event. A second instance sends one activation message and exits.

- [ ] **Step 4: Implement autostart**

Use package StartupTask when identity is present, otherwise the named HKCU Run value with a quoted path and `--autostart`. Default remains disabled and no administrator privilege is requested.

- [ ] **Step 5: Verify and commit**

Run automated tests, then manual duplicate-launch, Explorer restart, path-with-spaces, enable/reboot/disable checks. Record manual results in `tests/hardware/report-template.md`; commit `feat: add tray and optional autostart`.

### Task 9: Implement package-aware Windows Mobile Hotspot status and control

**Files:**
- Create: `crates/windows-platform/src/hotspot.rs`
- Modify: `crates/windows-platform/src/lib.rs`
- Create: `crates/windows-platform/tests/hotspot_mapping.rs`
- Create: `apps/panel/src/bin/hotspot_spike.rs`
- Create: `packaging/msix/Package.appxmanifest`

**Interfaces:**
- Produces `HotspotControl::capability`, `status`, and `set_enabled`.
- Source `ConnectionProfile` must map to the target `AdapterIdentity`; `GetInternetConnectionProfile()` is not accepted as source proof.

- [ ] **Step 1: Write failing profile-selection tests**

Fixtures include DJI + Meta/TUN + phone Wi-Fi profiles, no Wi-Fi radio, policy disabled, missing package identity, and unsupported OS. The selector returns only the profile whose `NetworkAdapterId` matches the DJI adapter GUID.

- [ ] **Step 2: Implement WinRT capability and control**

Use `NetworkInformation::GetConnectionProfiles`, `GetTetheringCapabilityFromConnectionProfile`, `CreateFromConnectionProfile`, `StartTetheringAsync`, and `StopTetheringAsync`. Map all known operation statuses to stable errors. UI operations transition through Starting/Stopping and requery final state.

- [ ] **Step 3: Add the package manifest**

Declare desktop full-trust entry, x64 architecture, `runFullTrust`, `wiFiControl`, protocol identity, and StartupTask. The app remains asInvoker; restricted elevation capability is not added until the helper packaging spike proves the exact distribution model.

- [ ] **Step 4: Execute the package matrix**

Run the same `hotspot_spike` unpackaged and from a development-signed MSIX. Unpackaged failure must be a clear `Unsupported(MissingPackageIdentity)`; packaged run must report capability and test start/stop only after an explicit `--allow-state-change` flag.

- [ ] **Step 5: Verify and commit**

Run tests and record Windows build, Wi-Fi hardware, package identity, source profile and operation status in the HIL report. Commit `feat: add package-aware hotspot control`.

### Task 10: Implement bounded one-shot IPC and the elevated helper

**Files:**
- Create: `crates/ipc/src/{lib,protocol,framing,security,client,server}.rs`
- Create: `crates/ipc/tests/{roundtrip,rejection}.rs`
- Modify: `apps/helper/src/main.rs`
- Create: `crates/windows-platform/src/privilege.rs`
- Modify: `crates/windows-platform/src/lib.rs`

**Interfaces:**
- Produces `HelperRequestV1`, `HelperResponseV1`, `OperationNonce`, `PipeServer`, `PipeClient`, and `launch_elevated_helper`.
- Helper request body is a closed enum; maximum frame is 64 KiB; request lifetime is at most 60 seconds; exactly one client and one operation are allowed.

- [ ] **Step 1: Write failing protocol rejection tests**

Reject unknown version, unknown enum, oversized frame, expired request, nonce mismatch, second client, remote client, wrong peer PID, unsupported device, and target identity drift. Serialization must reject unknown fields.

- [ ] **Step 2: Implement length-delimited framing and validation**

Frame is `u32` little-endian length followed by Serde payload, limited before allocation. Named pipe uses `PIPE_REJECT_REMOTE_CLIENTS` and an explicit DACL for current user, Administrators, and SYSTEM. Verify process IDs at both ends.

- [ ] **Step 3: Implement helper launch and one-shot execution shell**

Use `ShellExecuteExW` with verb `runas`; pass only pipe name, nonce and protocol version. The helper connects, validates, re-enumerates PID 4006, executes exactly one typed operation, reports a typed outcome, zeroizes nonce material, and exits.

- [ ] **Step 4: Verify and commit**

Run IPC tests and a harmless elevated `InspectTarget` operation, including UAC cancellation. Commit `feat: add secure one-shot helper IPC`.

### Task 11: Add controlled repair workflows with readback and audit outcomes

**Files:**
- Create: `crates/windows-platform/src/repair.rs`
- Modify: `crates/windows-platform/src/lib.rs`
- Modify: `crates/application/src/controller.rs`
- Modify: `apps/panel/src/ui/repairs.rs`
- Create: `crates/application/tests/repair_workflows.rs`

**Interfaces:**
- Consumes `ActionPlan` and helper/AT traits.
- Produces only `Applied`, `Failed { code }`, or `OutcomeUnknown { code }` with before/after evidence hashes.

- [ ] **Step 1: Write failing workflow tests**

Cover DHCP renew, DNS set/restore, hotspot toggle, adapter restart, device re-enumerate, module restart, APN round-trip, USB profile 0/1 switch, stale confirmation, removal during action, write timeout, and rollback failure. Assert an AT write count of exactly one even after timeout.

- [ ] **Step 2: Implement Windows repairs**

DHCP targets only the mapped adapter. DNS captures exact prior automatic/static state and restores it. Adapter restart and re-enumeration revalidate root ancestry before acting and perform best-effort re-enable on partial failure.

- [ ] **Step 3: Implement AT repairs**

APN edit requires complete parsing of an existing inactive context and exact readback. Module restart expects temporary removal. USB-network selection allows only verified profiles 0/1, expects re-enumeration, re-discovers a fresh port, and reads the setting back. A timeout is `OutcomeUnknown` and is never resent.

- [ ] **Step 4: Implement confirmation UI and redacted audit log**

The dialog shows target, old/new values, expected interruption, risk and elevation. Execution rechecks every token field. Audit records type, stable target, hashes, time, outcome and rollback; APN and identifiers are redacted.

- [ ] **Step 5: Verify and commit**

Run automated tests. Execute state-changing HIL only on the approved module/test SIM and capture before/readback/after evidence. Commit `feat: add controlled repair workflows`.

### Task 12: Complete release engineering, documentation, and acceptance evidence

**Files:**
- Create: `deny.toml`
- Create: `.github/workflows/ci.yml`
- Create: `packaging/scripts/{build-msix,verify-release}.ps1`
- Create: `README.md`
- Create: `SECURITY.md`
- Create: `tests/hardware/{README,report-template}.md`
- Create: `docs/architecture.md`

**Interfaces:**
- Produces reproducible unsigned-development artifacts, SHA-256 hashes, dependency/license reports, and an explicit HIL report.
- Does not label an unsigned build stable or imply DJI endorsement.

- [ ] **Step 1: Write the release verifier before packaging**

`verify-release.ps1` fails if expected EXEs, manifest, licenses, README, SECURITY, hashes, SBOM, or signature-status label are missing. When a trusted certificate is not configured, artifact names include `unsigned-development-only`.

- [ ] **Step 2: Add Windows CI**

CI runs formatting, Clippy with warnings denied, all locked tests, locked Windows x64 release build, and `cargo deny check`. It generates SBOM, dependency/license output and hashes. Hardware checks are not reported as CI passes.

- [ ] **Step 3: Write user and security documentation**

README contains first-generation identity, unofficial status, install/run/uninstall, small-window/tray/autostart behavior, availability meaning, hotspot package requirement, repair risks, privacy, troubleshooting and current limitations. SECURITY defines supported versions, private reporting path, sensitive diagnostics handling, and the no-raw-command boundary.

- [ ] **Step 4: Execute the full automated acceptance suite**

Run:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked --release --target x86_64-pc-windows-msvc
cargo deny check
```

Run `verify-release.ps1` against the produced development MSIX/artifact directory. Save exact command output in `tests/hardware/automated-acceptance-2026-09-03.md`.

- [ ] **Step 5: Execute and classify hardware acceptance**

Follow the design matrix for insert/remove, dock/port change, COM renumbering, normal/no/locked SIM as available, registration states, DHCP/APIPA/no-gateway, DNS failure, Meta/TUN competition, hotspot capability/start/stop, UAC cancellation, sleep/resume, Explorer restart and duplicate launch. Every row is marked Passed, Failed, Environment-blocked, or Unexecuted with evidence; unavailable test hardware is never silently counted as passed.

- [ ] **Step 6: Final review and commit**

Compare every design Definition of Done item against current files, command output, package contents and HIL evidence. Commit `release: prepare DJI 4G Gen1 panel 0.1.0` only after automated checks pass; keep failed/environment-blocked/manual items explicit in the acceptance report.
