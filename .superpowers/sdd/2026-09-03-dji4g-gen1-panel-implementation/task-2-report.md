# Task 2 Report: First-generation domain contracts

## Result

Implemented and committed the pure `dji4g-domain` state model and availability classifier for the first-generation DJI 4G dongle. The domain crate has no Windows or other platform dependency. Public snapshot and IPC-facing contracts are cloneable, equatable, and Serde-serializable.

Feature commit: `33b0e2f` (`feat: add first-generation domain state model`)

## Changed files

- `Cargo.lock`
- `crates/domain/Cargo.toml`
- `crates/domain/src/lib.rs`
- `crates/domain/src/device.rs`
- `crates/domain/src/cellular.rs`
- `crates/domain/src/network.rs`
- `crates/domain/src/availability.rs`
- `crates/domain/src/hotspot.rs`
- `crates/domain/src/action.rs`
- `crates/domain/src/snapshot.rs`
- `crates/domain/src/error.rs`
- `crates/domain/tests/availability_matrix.rs`
- `crates/domain/tests/action_safety.rs`

## Implemented contracts

- Centralized the only supported profile as `DeviceProfile::DJI_GEN1` / `DJI_GEN1`, exactly VID `0x2CA3`, PID `0x4006`.
- Added `DeviceEpoch`, stable target identity, typed timestamped `Evidence<T>`, evidence sources, and freshness/epoch validation.
- Added cellular, Windows network, bound public/DNS probe, protocol-family, global route, and global connectivity observations.
- Implemented `classify(&ClassificationInput, SystemTime) -> AvailabilityDecision` with the spec status names: `Detecting`, `Available`, `Limited`, `Unavailable`, `NotDetected`, and `UnsupportedDevice`.
- Kept global connectivity explanatory only; it cannot satisfy missing adapter-bound reachability.
- Made current successful absence/unsupported enumeration authoritative even if old-epoch positive network evidence remains in the input.
- Added independent `HotspotStatus` and immutable `AppSnapshot` contracts; hotspot is not an input to the availability classifier.
- Added the closed nine-operation `ActionKind`, action plan metadata, PID/epoch creation gates, and execution-time revision/epoch/identity/before-hash/expiry revalidation.
- Added stable domain errors and the closed `Applied` / `Failed` / `OutcomeUnknown` operation outcome.

## TDD evidence

### RED 1: identity and classifier

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --test availability_matrix
```

Result: exit `1`, expected compile failure `E0432`; the domain imports and `classify` did not exist.

### RED 2: action safety

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --test action_safety
```

Result: exit `1`, expected compile failure `E0432`; action-plan contracts did not exist.

### RED 3: removal priority self-review regression

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --test availability_matrix current_absence_overrides_leftover_positive_evidence_from_the_previous_epoch -- --exact
```

Result: exit `1`; actual `Detecting`, expected `NotDetected`. This exposed that stale old-epoch positive evidence was checked before a fresh current absence result.

### RED 4: incomplete AT evidence

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --test availability_matrix classification_table_covers_the_spec_rules -- --exact
```

Result: exit `1`; missing AT evidence incorrectly classified `Available` instead of `Limited(IncompleteEvidence)`.

### GREEN

After the minimal implementations and classification-order fixes, both focused regressions passed. Final package verification is recorded below.

## Final verification

All commands ran from `C:\Users\22050\Desktop\dji4g-panel\.worktrees\dji4g-gen1-panel` after the final edits.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' fmt --all -- --check
```

Result: exit `0`, no output.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --locked
```

Result: exit `0`; 12 integration tests passed (4 action safety, 8 availability), 0 failed; unit/doc targets also passed.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' clippy -p dji4g-domain --all-targets --locked -- -D warnings
```

Result: exit `0`; Clippy completed with warnings denied.

```powershell
git diff --cached --check
git diff --check
```

Result before commit: exit `0`; no whitespace errors.

## Self-review

- Inspected the full staged Task 2 diff and confirmed only the domain crate, lockfile, and the two specified test files were included.
- Rechecked every brief requirement against the public API and tests.
- Confirmed PID `0x4009` cannot match the supported profile or create an action plan.
- Confirmed epoch mismatch discards prior positive evidence and prevents plan creation/execution.
- Confirmed bound success with unavailable AT control is `Limited`, not `Unavailable`.
- Confirmed global phone connectivity cannot create a green result.
- Confirmed hotspot state is structurally separate from cellular availability.
- Mutation check: changing the supported PID, freshness/epoch check, classification branches, two-cycle failure threshold, or any action-plan precondition is covered by a failing test.

## Concerns and deferred boundaries

- No platform or hardware behavior was tested in this pure-domain task; Windows integration and HIL acceptance remain later plan tasks.
- `ActionKind::EditApn` carries the requested modeled value, but wire-level APN syntax validation and typed AT rendering intentionally belong to Task 3. Executors must consume the validated Task 3 type and must never render this string as raw AT input.

## Review fix round 1

Fix commit: `c3627c1` (`fix: bind domain evidence to verified device identity`)

### Findings addressed

1. `StableDeviceIdentity::is_supported()` now parses a canonical three-part USB PnP instance ID, extracts exactly one four-digit VID and PID, and requires those values to equal both the `2CA3/4006` profile and the separate scalar fields. PID 4009 hidden behind 4006 scalar fields and arbitrary instance strings fail closed.
2. Added `AdapterBinding` and generic `BoundEvidence<T>`. `ClassificationInput` now carries the current PnP target identity and Windows adapter binding; public, DNS, and protocol-family evidence must be fresh for the current epoch, carry the exact expected source, and match the current DJI adapter binding.
3. Fresh authoritative `NotDetected` and `UnsupportedDevice` enumeration is evaluated before transition phases, so those states win during insertion and re-enumeration.
4. `Evidence::is_fresh_for()` now uses `now.duration_since(observed_at)` and rejects future-dated observations.
5. Global connectivity remains explanatory: stale global evidence is ignored rather than gating fresh bound evidence, and only fresh evidence with `EvidenceSource::GlobalConnectivity` participates in the no-bound-reachability explanation.

### Focused RED evidence

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --test action_safety
```

Before the identity fix: exit `1`; 4 passed and 2 failed. Both `scalar_pid_cannot_hide_an_unsupported_instance_identity` and `arbitrary_instance_identity_cannot_create_an_action_plan` returned `Ok(ActionPlan)` instead of `Err(UnsupportedDevice)`.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --test availability_matrix
```

Before the binding API existed: exit `1` with `E0432`, `E0560`, and `E0609` because `AdapterBinding`, `BoundEvidence`, the current target/binding fields, and binding-bearing probe values did not exist. The new tests exercise the requested unavailable behavior through the public classifier rather than source inspection.

### Covering regression tests

- `scalar_pid_cannot_hide_an_unsupported_instance_identity`
- `arbitrary_instance_identity_cannot_create_an_action_plan`
- `phone_wifi_or_tun_binding_cannot_supply_dji_bound_success`
- `global_sources_cannot_masquerade_as_bound_probe_evidence`
- `current_absence_and_unsupported_device_win_during_transitional_phases`
- `future_dated_evidence_is_not_fresh`
- `stale_global_connectivity_does_not_demote_fresh_bound_success`

### GREEN and final verification

All commands ran after the final fix from `C:\Users\22050\Desktop\dji4g-panel\.worktrees\dji4g-gen1-panel`.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' fmt --all -- --check
```

Result: exit `0`, no output.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-domain --locked
```

Result: exit `0`; 19 integration tests passed (6 action safety, 13 availability), 0 failed; unit and doc-test targets also passed.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' clippy -p dji4g-domain --all-targets --locked -- -D warnings
```

Result: exit `0`; Clippy completed with warnings denied.

### Fix-round self-review

- Re-read the five findings against the final control flow and inspected the complete five-file fix diff.
- Confirmed an action plan cannot be prepared or executed when scalar VID/PID disagrees with the parsed PnP identity.
- Confirmed public success from phone, Wi-Fi, Meta/TUN, or a global evidence source cannot satisfy the DJI binding.
- Confirmed stale and future observations cannot revive a green result, while stale optional global status does not demote fresh bound evidence.
- Confirmed the crate remains pure Rust plus Serde/thiserror and adds no Windows dependency.
