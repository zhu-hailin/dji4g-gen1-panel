# Task 3 Report: Typed AT boundary and streaming parser

## Result

Implemented and committed the closed, typed AT protocol boundary for the first-generation DJI 4G panel.

Feature commit: `260f27f` (`feat: add typed AT protocol`)

The raw renderer remains crate-private. External callers can transmit only an opaque `EncodedAtCommand` created by `AtCommand::encode()` and can access its bytes only through `as_bytes()`. No public API accepts an arbitrary AT string or byte buffer.

## Changed files

- `Cargo.lock`
- `crates/at-protocol/Cargo.toml`
- `crates/at-protocol/src/lib.rs`
- `crates/at-protocol/src/command.rs`
- `crates/at-protocol/src/model.rs`
- `crates/at-protocol/src/parser.rs`
- `crates/at-protocol/src/redact.rs`
- `crates/at-protocol/tests/render_whitelist.rs`
- `crates/at-protocol/tests/parser_fixtures.rs`
- `crates/at-protocol/tests/redaction.rs`
- `tests/fixtures/at/binary_wrong_port.txt`
- `tests/fixtures/at/cme_error.txt`
- `tests/fixtures/at/echo_off_identity.txt`
- `tests/fixtures/at/echo_on_csq.txt`
- `tests/fixtures/at/interleaved_cereg_urcs.txt`
- `tests/fixtures/at/nmea_wrong_port.txt`

## Implemented contracts

- Added every `AtCommand` variant named by the Task 3 brief with exact, single-command CR-terminated rendering.
- Added the opaque `EncodedAtCommand`; its fields and constructor are private, while `as_bytes()` supports the future serial actor.
- Kept `AtCommand::wire_bytes()` crate-private. There is no `send_raw_at`, public raw renderer, arbitrary `String`, or arbitrary byte constructor.
- Added validated `Apn` input. It rejects empty input, non-ASCII, quote, comma, semicolon, CR/LF, other controls, and values longer than 100 ASCII bytes.
- Added only `VerifiedUsbNetProfile::DjiNdis` and `VerifiedUsbNetProfile::Ecm`, mapping to raw values 0 and 1. Values 2 and 3 have no typed representation.
- Classified `RestartModule`, `SetApn`, and both USB-network profile writes as writes and assigned `RetryPolicy::Never`. Read commands use `OnceAfterQuietPeriod`.
- Added `StreamingParser::new(DeviceEpoch, AtCommand)`, bounded `push`, and `finish_timeout` using `dji4g-domain`'s `DeviceEpoch` and `ErrorCode`.
- Added echo suppression, CR/LF/CRLF handling, byte-fragmented parsing, multiline responses, typed final codes, known URCs, and command-aware same-prefix response classification.
- Bounded a line at 4096 bytes, a transaction at 64 KiB/128 lines, and sustained unrecognized printable input at 32 lines. NMEA, binary/control bytes, 4097-byte lines, and sustained printable wrong-port data fail closed.
- Added deterministic fragmentation and 256-seed byte fuzz regressions. A parser that fails remains failed and does not later accept a final `OK`.
- Added AT-text redaction for APNs, long numeric identifiers, and PIN/PUK-like values. Debug output for APNs, typed commands, encoded transactions, responses, and URCs is redacted where it can carry sensitive data.
- Backend validation/protocol errors render stable nonlocalized machine codes such as `apn:unsafe_character` and `at_protocol:timeout`; Task 3 adds no user-facing English reason text.

## TDD evidence

All commands ran from `C:\Users\22050\Desktop\dji4g-panel\.worktrees\dji4g-gen1-panel` using the installed Cargo executable at `C:\Users\22050\.cargo\bin\cargo.exe`.

### RED 1: closed renderer API did not exist

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test render_whitelist
```

Result: exit `1`, expected compile failure `E0432`. `Apn`, `AtCommand`, `RetryPolicy`, and `VerifiedUsbNetProfile` were absent.

The first attempted `cargo test` without an absolute executable path was environment-blocked because this PowerShell session did not include Cargo in `PATH`; it was rerun immediately with the absolute installed path before production implementation.

### GREEN 1: exact rendering and write policy

After the minimal command/model implementation, the renderer target passed 3 tests. The temporary dead-code warning for `response_prefix` disappeared when the parser began consuming it.

### RED 2: parser and redaction APIs did not exist

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test parser_fixtures --test redaction
```

Result: exit `1`, expected compile failures `E0432` and `E0425`. `StreamingParser`, `AtEvent`, `AtResponse`, `AtFinalCode`, `ProtocolErrorKind`, and `redact_at_text` were absent.

The first parser GREEN attempt produced 6 passing tests and one failing echo assertion because the test fixture incorrectly contained `AT+CSQ?`; the approved command is `AT+CSQ`. The fixture was corrected, not the renderer, and the target then passed 7/7.

### RED 3: sustained printable wrong-port input was not yet rejected

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test render_whitelist --test parser_fixtures
```

Result: exit `1`; 10 parser tests passed and `rejects_sustained_unrecognized_printable_wrong_port_data` failed because 33 unrecognized printable lines were still accepted as an incomplete response. The parser then gained the bounded 32-line unrecognized-data threshold and fail-closed behavior.

### RED 4: backend errors still exposed English prose

Commands:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test render_whitelist apn_validation_errors_are_stable_nonlocalized_codes -- --exact
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test parser_fixtures timeout_discards_an_incomplete_final_line_and_uses_domain_error_code -- --exact
```

Result: both exited `1` as expected. Actual strings were `APN must not be empty` and `AT protocol failed: Timeout`; expected strings were the stable codes `apn:empty` and `at_protocol:timeout`. The implementation was then changed to machine-code-only display.

### GREEN

The final package run passed 19 integration tests: 12 parser/fragmentation/fuzz tests, 2 redaction tests, and 5 renderer/safety tests. Unit and doc-test targets also passed.

## Final verification

All commands below ran after the final source edits.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' fmt --all -- --check
```

Result: exit `0`, no formatting differences.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --locked
```

Result: exit `0`; 19 integration tests passed, 0 failed; unit and doc-test targets also passed.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' clippy -p dji4g-at-protocol --all-targets --locked -- -D warnings
```

Result: exit `0`; strict Clippy completed with warnings denied.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test --workspace --locked
```

Result: exit `0`; all workspace targets passed, including the 19 AT protocol integration tests and the existing 19 domain integration tests.

```powershell
git diff --check
git diff --cached --check
```

Result before the feature commit: exit `0`; no whitespace errors.

## Self-review

- Inspected the complete staged 16-file feature diff and confirmed it contained only Task 3 crate, fixture, test, manifest, and lockfile changes.
- Re-read Task 3 brief and design section 8 against the final public API.
- Confirmed the raw renderer is `pub(crate)`, `EncodedAtCommand` fields are private, and no raw input constructor or raw-send function exists.
- Confirmed exact bytes for every modeled enum variant, one CR terminator, no LF, no `ATE0`, no semicolon concatenation, and only USB-network raw values 0/1.
- Confirmed all four modeled write instances (including both USB profiles) are writes with `RetryPolicy::Never`.
- Confirmed APN injection boundaries cover quote, comma, semicolon, CR/LF, controls, non-ASCII, empty, 100-byte accepted, and 101-byte rejected cases.
- Confirmed same-prefix `+CEREG:` data is retained in the active query response while unrelated known URCs are emitted separately.
- Confirmed NMEA, binary controls, sustained printable noise, line overflow, transaction bounds, incomplete timeout, and post-failure reuse all fail closed without retaining raw partial content in the returned error.
- Confirmed debug formatting cannot reveal a typed APN or encoded command bytes, and explicit AT redaction removes APNs, long identifiers, and PIN-like values.
- Mutation check: changing any exact command, profile mapping, write retry policy, APN delimiter rule, final-code branch, URC/response precedence, 4096-byte boundary, timeout code, or redaction marker is covered by a failing test.

## Concerns and deferred boundaries

- This task verifies the pure typed protocol boundary with fixtures and deterministic fuzzing only. It does not open a COM port, choose a Windows device interface, or perform hardware-in-the-loop AT acceptance; those remain Task 4 and later HIL work.
- Coverage-guided `cargo-fuzz` infrastructure was not added to the repository because Task 3 requested fixture tests and did not specify a persistent fuzz workspace. The deterministic byte and fragmentation sweeps run in normal CI and cover fail-closed/no-panic regressions reproducibly.
- Actual firmware variants may expose additional single-line or multiline URCs. Only explicitly recognized, currently modeled URCs are separated; unknown sustained data is bounded and rejected rather than treated as trusted indefinitely.

## Fix round 1: CID, response-schema, removal, and redaction hardening

Fix commit: `9931f58` (`fix: harden typed AT response validation`)

### Findings addressed

1. Replaced the public raw `u8` APN context field with opaque `PdpContextId`. Construction accepts only the conservative V1 range `1..=16`; 0, 17, and 255 fail. Later application logic must still prove the selected context exists and is inactive.
2. Replaced prefix-only `+CEREG:` classification with shape-aware parsing. A query response requires mode and status fields, while a status-only registration line is emitted as a URC even when interleaved with the active `AT+CEREG?` transaction.
3. Added per-command response schemas and successful-response line-count checks. `Attention` and writes accept zero response lines; structured queries require their command-specific prefix and field shape; only identity/manufacturer/model/revision accept bounded printable free-form lines.
4. Removed `+CMT:` and `+CDS:` from recognized single-line URCs. Their unmodeled multipart form now fails closed before a payload can be attached to the active response. Required modeled single-line and registration URCs remain covered.
5. Added `StreamingParser::finish_removed()`. It returns domain `ErrorCode::DeviceRemoved` plus stable protocol kind/code, clears partial line and transaction data, and leaves the parser in a sticky failed state.
6. Fixed PIN/PUK redaction to skip optional spaces/tabs around the separator before replacing a value. `PIN: 1234`, `PUK= 87654321`, and the prior unspaced form are covered.

### Changed files

- `crates/at-protocol/src/command.rs`
- `crates/at-protocol/src/lib.rs`
- `crates/at-protocol/src/model.rs`
- `crates/at-protocol/src/parser.rs`
- `crates/at-protocol/src/redact.rs`
- `crates/at-protocol/tests/render_whitelist.rs`
- `crates/at-protocol/tests/parser_fixtures.rs`
- `crates/at-protocol/tests/redaction.rs`
- `tests/fixtures/at/interleaved_cereg_same_prefix.txt`

### Focused RED evidence

CID boundary command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test render_whitelist pdp_context_id_accepts_only_the_v1_range -- --exact
```

Result: exit `1`, expected `E0432`; `PdpContextId` did not exist.

Parser schema and same-prefix command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test parser_fixtures --test redaction
```

Result: exit `1`; 14 parser tests passed and 5 focused tests failed. The failures proved status-only `+CEREG:` was retained as a response, `Attention` accepted arbitrary printable garbage, malformed structured responses were accepted, an empty successful query response was accepted, and multipart SMS URCs corrupted a free-form transaction.

Spaced secret redaction command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test redaction redacts_apns_identifiers_and_pin_like_values -- --exact
```

Result: exit `1`; `PIN: 5678` remained visible before the whitespace fix.

Device-removal command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test parser_fixtures device_removal -- --nocapture
```

Result: exit `1`, expected `E0599`; `finish_removed` and `ProtocolErrorKind::DeviceRemoved` did not exist.

### Focused tests

- Added 1 renderer boundary test covering CID 0, 1, 16, 17, and 255.
- Added 9 parser tests covering same-prefix CEREG interleaving, arbitrary fixed-command text, malformed and valid command schemas, missing successful response data, bounded free-form eligibility, multipart URC rejection, partial-line removal, and partial-transaction removal.
- Expanded the existing redaction test with spaced PIN and PUK values.
- Final Task 3 package total: 29 tests (21 parser, 2 redaction, 6 renderer).

### GREEN and final verification

All commands ran after the final fix from `C:\Users\22050\Desktop\dji4g-panel\.worktrees\dji4g-gen1-panel`.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' fmt --all -- --check
```

Result: exit `0`; the entire workspace had no formatting differences.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --locked
```

Result: exit `0`; 29 tests passed, 0 failed; unit and doc-test targets also passed.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' clippy -p dji4g-at-protocol --all-targets --locked -- -D warnings
```

Result: exit `0`; strict package Clippy completed with warnings denied.

```powershell
git diff --check
git diff --cached --check
```

Result before commit: exit `0`; no whitespace errors.

### Fix-round self-review

- Inspected the full nine-file fix diff and verified no application, Windows, IPC, UI, or raw serial API was added.
- Confirmed `SetApn` can no longer contain a CID outside 1 through 16 and the renderer still has no raw constructor.
- Confirmed same-prefix classification checks the active command and field shape before general URC classification.
- Confirmed every fixed command rejects arbitrary printable lines and every successful structured query requires at least one valid response line; normal error finals remain valid without response data.
- Confirmed identity/version free-form data remains bounded by the existing line, response-byte, response-line, and unrecognized-line caps.
- Confirmed unsupported multipart SMS URCs fail even under a free-form identity command rather than emitting a header and misattaching the payload.
- Confirmed both timeout and removal clear buffered data and make parser failure sticky; removal uses the shared domain `DeviceRemoved` identity.
- Confirmed spaced secret redaction preserves formatting whitespace while replacing only the secret value.

### Remaining concerns

- Per-command schemas are intentionally conservative. A firmware response shape outside the modeled V1 forms will fail closed and must be added with an evidence-backed fixture rather than weakened to arbitrary text.
- Serial actor integration and physical removal timing remain Task 4/HIL concerns; this round verifies only the pure parser termination contract.

## Fix round 2: Preserve terminal protocol causes

Fix commit: `1e4b3e8` (`fix: preserve AT parser terminal causes`)

### Finding addressed

`StreamingParser` previously stored only `failed: bool`. `finish_removed()` initially returned domain `DeviceRemoved`, but every later `push()` replaced that terminal cause with `VerificationFailed` / `UnexpectedData`. The parser now stores the first terminal `ProtocolError`. First terminal cause wins for removal, timeout, and ordinary protocol failures; all subsequent `push`, `finish_timeout`, and `finish_removed` calls replay that same code and kind. Buffer clearing still occurs when the first terminal cause is recorded.

### Changed files

- `crates/at-protocol/src/parser.rs`
- `crates/at-protocol/tests/parser_fixtures.rs`

### Focused RED evidence

Command:

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test parser_fixtures device_removal_discards_a_partial_line_and_makes_failure_sticky -- --exact
```

Result: exit `1`. After `finish_removed`, the next `push` returned `ProtocolError { code: VerificationFailed, kind: UnexpectedData }` instead of the expected stored `ProtocolError { code: DeviceRemoved, kind: DeviceRemoved }`.

### Focused regressions

- Strengthened both existing removal tests to compare the complete replayed error rather than merely asserting another failure.
- The partial-line removal test now verifies later `push`, `finish_timeout`, and `finish_removed` all retain `DeviceRemoved`.
- Added `timeout_remains_the_terminal_cause_across_later_calls`.
- Added `ordinary_protocol_failure_remains_fail_closed_with_its_original_cause`.
- Updated the UTF-8 conversion failure path to enter the same stored terminal-error path instead of returning an unstored one-off error.

### GREEN and final verification

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --test parser_fixtures
```

Result: exit `0`; 23 parser tests passed, 0 failed.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' test -p dji4g-at-protocol --locked
```

Result: exit `0`; all 31 package tests passed (23 parser, 2 redaction, 6 renderer), 0 failed; unit and doc-test targets also passed.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' fmt --all -- --check
```

Result: exit `0`; the workspace had no formatting differences.

```powershell
& 'C:\Users\22050\.cargo\bin\cargo.exe' clippy -p dji4g-at-protocol --all-targets --locked -- -D warnings
```

Result: exit `0`; strict package Clippy completed with warnings denied.

```powershell
git diff --check
git diff --cached --check
```

Result before commit: exit `0`; no whitespace errors.

### Fix-round self-review

- Confirmed the boolean terminal marker is gone; the parser retains `Option<ProtocolError>` instead.
- Confirmed first terminal cause wins and cannot be overwritten by a later timeout, removal signal, protocol data, or final code.
- Confirmed every terminal path clears partial line, response lines, byte counts, and unrecognized-line counts exactly once.
- Confirmed ordinary wrong-port and malformed-response failures remain fail-closed while now preserving their original cause.
- Confirmed no public API, command whitelist, response schema, retry policy, redaction boundary, or localization behavior changed in this round.

### Remaining concerns

- This pure parser contract cannot prove when the future serial actor detects physical removal; Task 4 must call `finish_removed()` when its epoch/handle is invalidated.
