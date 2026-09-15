#![forbid(unsafe_code)]

use std::{
    fmt::Write as _,
    io::Write as _,
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Invocation {
    ReadOnly,
    SetEnabled(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CliError {
    UnsupportedDevice,
    InvalidArguments,
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|argument| argument == "--worker") {
        let exit_code = match parse_worker_invocation(&args[1..]) {
            Ok(invocation) => run_worker(invocation),
            Err(CliError::UnsupportedDevice) => {
                print_error("unsupported_device", "pnp:unsupported_device", None)
            }
            Err(CliError::InvalidArguments) => {
                print_error("invalid_arguments", "cli:invalid_arguments", None)
            }
        };
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return;
    }
    let exit_code = match parse_invocation(&args) {
        Ok(invocation) => run(invocation),
        Err(CliError::UnsupportedDevice) => {
            println!(
                "{}",
                error_json("unsupported_device", "pnp:unsupported_device", None)
            );
            3
        }
        Err(CliError::InvalidArguments) => {
            println!(
                "{}",
                error_json("invalid_arguments", "cli:invalid_arguments", None)
            );
            2
        }
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

fn parse_worker_invocation<S: AsRef<str>>(args: &[S]) -> Result<Invocation, CliError> {
    if args.len() != 2 || args[0].as_ref() != "--mode" {
        return Err(CliError::InvalidArguments);
    }
    match args[1].as_ref() {
        "read-only" => Ok(Invocation::ReadOnly),
        "start" => Ok(Invocation::SetEnabled(true)),
        "stop" => Ok(Invocation::SetEnabled(false)),
        _ => Err(CliError::InvalidArguments),
    }
}

fn parse_invocation<S: AsRef<str>>(args: &[S]) -> Result<Invocation, CliError> {
    let mut adapter_pid = None;
    let mut json_requested = false;
    let mut read_only = false;
    let mut state_change = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_ref() {
            "--adapter-pid" => {
                if adapter_pid.is_some() || index + 1 >= args.len() {
                    return Err(CliError::InvalidArguments);
                }
                adapter_pid = Some(args[index + 1].as_ref());
                index += 2;
            }
            "--json" => {
                if json_requested {
                    return Err(CliError::InvalidArguments);
                }
                json_requested = true;
                index += 1;
            }
            "--read-only" => {
                if read_only {
                    return Err(CliError::InvalidArguments);
                }
                read_only = true;
                index += 1;
            }
            "--allow-state-change" => {
                if state_change.is_some() || index + 1 >= args.len() {
                    return Err(CliError::InvalidArguments);
                }
                state_change = Some(match args[index + 1].as_ref() {
                    "start" => true,
                    "stop" => false,
                    _ => return Err(CliError::InvalidArguments),
                });
                index += 2;
            }
            _ => return Err(CliError::InvalidArguments),
        }
    }

    if adapter_pid != Some("4006") {
        return Err(CliError::UnsupportedDevice);
    }
    if !json_requested || (read_only && state_change.is_some()) {
        return Err(CliError::InvalidArguments);
    }
    Ok(match state_change {
        Some(enabled) => Invocation::SetEnabled(enabled),
        None => Invocation::ReadOnly,
    })
}

const READ_ONLY_WORKER_DEADLINE: Duration = Duration::from_secs(20);
const STATE_CHANGE_WORKER_DEADLINE: Duration = Duration::from_secs(35);

fn worker_deadline(invocation: Invocation) -> Duration {
    match invocation {
        Invocation::ReadOnly => READ_ONLY_WORKER_DEADLINE,
        Invocation::SetEnabled(_) => STATE_CHANGE_WORKER_DEADLINE,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerWait {
    Exited(i32),
    DeadlineExceeded,
}

trait WorkerProcess {
    fn try_wait(&mut self) -> std::io::Result<Option<i32>>;
    fn terminate(&mut self) -> std::io::Result<()>;
}

impl WorkerProcess for std::process::Child {
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        self.try_wait()
            .map(|status| status.map_or(Some(1), |value| Some(value.code().unwrap_or(1))))
    }

    fn terminate(&mut self) -> std::io::Result<()> {
        self.kill()
    }
}

fn supervise_worker<W: WorkerProcess>(worker: &mut W, deadline: Instant) -> WorkerWait {
    loop {
        match worker.try_wait() {
            Ok(Some(code)) => return WorkerWait::Exited(code),
            Ok(None) => {}
            Err(_) => {
                let _ = worker.terminate();
                return WorkerWait::Exited(1);
            }
        }
        if Instant::now() >= deadline {
            let _ = worker.terminate();
            return WorkerWait::DeadlineExceeded;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn run(invocation: Invocation) -> i32 {
    stage_marker("supervisor:start");
    let executable = match std::env::current_exe() {
        Ok(value) => value,
        Err(_) => return print_error("worker_error", "hotspot:worker_exe_unavailable", None),
    };
    let mode = match invocation {
        Invocation::ReadOnly => "read-only",
        Invocation::SetEnabled(true) => "start",
        Invocation::SetEnabled(false) => "stop",
    };
    let mut child = match Command::new(executable)
        .arg("--worker")
        .arg("--mode")
        .arg(mode)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(value) => value,
        Err(_) => return print_error("worker_error", "hotspot:worker_spawn_failed", None),
    };
    match supervise_worker(&mut child, Instant::now() + worker_deadline(invocation)) {
        WorkerWait::Exited(code) => code,
        WorkerWait::DeadlineExceeded => {
            stage_marker("supervisor:deadline_exceeded");
            print_error(
                "deadline_exceeded",
                "hotspot:worker_deadline_exceeded",
                None,
            )
        }
    }
}

#[cfg(not(windows))]
fn run(_invocation: Invocation) -> i32 {
    print_error("unsupported_platform", "hotspot:unsupported_platform", None)
}

fn stage_marker(marker: &str) {
    eprintln!("hotspot_spike:{marker}");
    let _ = std::io::stderr().flush();
}

#[cfg(windows)]
fn run_worker(invocation: Invocation) -> i32 {
    use dji4g_domain::DeviceEpoch;
    use dji4g_windows_platform::{
        HotspotPolicy, WindowsAdapterResolver, WindowsDeviceInventory, WindowsHotspotControl,
    };

    stage_marker("worker:start");
    stage_marker("inventory:start");
    let inventory = match WindowsDeviceInventory.scan_now() {
        Ok(value) => value,
        Err(error) => return print_error("inventory_error", error.code, error.os_code),
    };
    stage_marker("inventory:complete");
    let device = match inventory.devices() {
        [] => return print_error("not_detected", "pnp:not_detected", None),
        [device] => device,
        _ => return print_error("ambiguous_device", "pnp:ambiguous_device", None),
    };
    stage_marker("adapter:start");
    let observation = match WindowsAdapterResolver.resolve(device, DeviceEpoch(1)) {
        Ok(value) => value,
        Err(error) => return print_error("adapter_error", error.code, error.os_code),
    };
    stage_marker("adapter:complete");
    let package_control = WindowsHotspotControl::new();
    stage_marker("package_identity:start");
    let package_identity = match package_control.package_identity() {
        Ok(value) => value,
        Err(error) => return print_error("package_identity_error", error.code, error.os_code),
    };
    stage_marker("package_identity:complete");
    let control = match invocation {
        Invocation::ReadOnly => package_control,
        Invocation::SetEnabled(_) => WindowsHotspotControl::with_policy(HotspotPolicy {
            // The flag is an explicit caller authorization. This spike never passes it during
            // read-only HIL; production UI policy must still gate state changes separately.
            allow_unpacked_state_change: true,
            operation_timeout: Duration::from_secs(30),
        }),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return print_error("runtime_error", "hotspot:runtime_unavailable", None),
    };
    let result = runtime.block_on(async {
        stage_marker("capability:start");
        let capability = control.capability(&observation.identity).await;
        stage_marker("capability:complete");
        stage_marker("status:start");
        let status = control.status(&observation.identity).await;
        stage_marker("status:complete");
        let operation = match invocation {
            Invocation::ReadOnly => None,
            Invocation::SetEnabled(enabled) => {
                stage_marker("operation:start");
                Some(control.set_enabled(&observation.identity, enabled).await)
            }
        };
        if operation.is_some() {
            stage_marker("operation:complete");
        }
        (capability, status, operation)
    });

    let (capability, status, operation) = result;
    let capability = match capability {
        Ok(value) => value,
        Err(error) => return print_error("capability_error", error.code, error.os_code),
    };
    let status = match status {
        Ok(value) => value,
        Err(error) => return print_error("status_error", error.code, error.os_code),
    };
    if capability.source_adapter_id != observation.identity.guid_string()
        || status.source_adapter_id != observation.identity.guid_string()
    {
        return print_error("profile_error", "hotspot:source_adapter_mismatch", None);
    }
    let operation = match operation {
        None => None,
        Some(Ok(value)) => Some(value),
        Some(Err(error)) => return print_error("operation_error", error.code, error.os_code),
    };

    println!(
        "{}",
        render_observation(
            invocation,
            observation.identity.epoch().0,
            &observation.identity.guid_string(),
            package_identity,
            &capability,
            &status,
            operation.as_ref(),
        )
    );
    let _ = std::io::stdout().flush();
    stage_marker("worker:complete");
    0
}

#[cfg(not(windows))]
fn run_worker(_invocation: Invocation) -> i32 {
    print_error("unsupported_platform", "hotspot:unsupported_platform", None)
}

#[cfg(windows)]
fn render_observation(
    invocation: Invocation,
    epoch: u64,
    adapter_guid: &str,
    package_identity: dji4g_windows_platform::PackageIdentityState,
    capability: &dji4g_windows_platform::HotspotCapabilityObservation,
    status: &dji4g_windows_platform::HotspotStatusObservation,
    operation: Option<&dji4g_windows_platform::HotspotOperationReceipt>,
) -> String {
    use dji4g_windows_platform::{HotspotCapabilityState, PackageIdentityState};

    let mode = match invocation {
        Invocation::ReadOnly => "read_only",
        Invocation::SetEnabled(_) => "state_change",
    };
    let (capability_state, capability_reason) = match capability.state {
        HotspotCapabilityState::Enabled => ("enabled", None),
        HotspotCapabilityState::Unsupported(reason) => ("unsupported", Some(reason_name(reason))),
        HotspotCapabilityState::Disabled { code } => ("disabled", Some(code)),
    };
    let mut output = format!(
        "{{\"status\":\"observed\",\"mode\":{},\"epoch\":{},\"adapter_guid\":{},\"profile_selection\":\"exact_adapter_guid\",\"package_identity\":{},\"capability\":{{\"state\":{},\"raw\":{},\"code\":{}",
        json(mode),
        epoch,
        json(adapter_guid),
        json(match package_identity {
            PackageIdentityState::Packaged => "packaged",
            PackageIdentityState::Unpackaged => "unpackaged",
        }),
        json(capability_state),
        capability.raw_capability,
        json(render_observation_capability_code(capability)),
    );
    if let Some(reason) = capability_reason {
        let _ = write!(output, ",\"reason\":{}", json(reason));
    }
    let _ = write!(
        output,
        "}},\"hotspot\":{{\"state\":{},\"raw_operational_state\":{},\"stable_code\":{},\"client_count\":{},\"client_count_error\":{}",
        json(status_name(status.status)),
        status.raw_operational_state,
        json(status.stable_code),
        option_u32(status.client_count),
        status
            .client_count_error
            .map_or_else(|| "null".to_owned(), json),
    );
    if let Some(operation) = operation {
        let _ = write!(
            output,
            "}},\"operation\":{{\"requested_enabled\":{},\"operation_status\":{},\"outcome\":{},\"stable_code\":{},\"final_status\":{}",
            operation.requested_enabled,
            operation.operation_status,
            json(operation_outcome_name(operation.outcome)),
            json(operation.stable_code),
            operation
                .final_status
                .map_or_else(|| "null".to_owned(), |value| json(status_name(value))),
        );
    }
    output.push_str("}}");
    output
}

#[cfg(windows)]
fn status_name(status: dji4g_domain::HotspotStatus) -> &'static str {
    use dji4g_domain::HotspotStatus;
    match status {
        HotspotStatus::Unsupported(_) => "unsupported",
        HotspotStatus::Off => "off",
        HotspotStatus::Starting => "starting",
        HotspotStatus::On { .. } => "on",
        HotspotStatus::Stopping => "stopping",
        HotspotStatus::Failed { .. } => "failed",
    }
}

#[cfg(windows)]
fn reason_name(reason: dji4g_domain::HotspotUnsupportedReason) -> &'static str {
    use dji4g_domain::HotspotUnsupportedReason;
    match reason {
        HotspotUnsupportedReason::MissingPackageIdentity => "missing_package_identity",
        HotspotUnsupportedReason::MissingWifiControlCapability => "missing_wifi_control_capability",
        HotspotUnsupportedReason::NoWifiAdapter => "no_wifi_adapter",
        HotspotUnsupportedReason::PolicyDisabled => "policy_disabled",
        HotspotUnsupportedReason::UnsupportedOperatingSystem => "unsupported_operating_system",
        HotspotUnsupportedReason::SourceProfileUnavailable => "source_profile_unavailable",
    }
}

#[cfg(windows)]
fn operation_outcome_name(
    outcome: dji4g_windows_platform::HotspotOperationOutcome,
) -> &'static str {
    use dji4g_windows_platform::HotspotOperationOutcome;
    match outcome {
        HotspotOperationOutcome::Applied => "applied",
        HotspotOperationOutcome::Failed { .. } => "failed",
        HotspotOperationOutcome::OutcomeUnknown { .. } => "outcome_unknown",
    }
}

#[cfg(windows)]
fn render_observation_capability_code(
    capability: &dji4g_windows_platform::HotspotCapabilityObservation,
) -> &'static str {
    match capability.state {
        dji4g_windows_platform::HotspotCapabilityState::Enabled => "hotspot:ok",
        dji4g_windows_platform::HotspotCapabilityState::Unsupported(reason) => match reason {
            dji4g_domain::HotspotUnsupportedReason::PolicyDisabled => "hotspot:policy_disabled",
            dji4g_domain::HotspotUnsupportedReason::NoWifiAdapter => "hotspot:no_wifi_adapter",
            dji4g_domain::HotspotUnsupportedReason::MissingWifiControlCapability => {
                "hotspot:wifi_control_capability_missing"
            }
            _ => "hotspot:unsupported",
        },
        dji4g_windows_platform::HotspotCapabilityState::Disabled { code } => code,
    }
}

#[cfg(windows)]
fn option_u32(value: Option<u32>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn print_error(status: &str, code: &str, os_code: Option<u32>) -> i32 {
    println!("{}", error_json(status, code, os_code));
    let _ = std::io::stdout().flush();
    1
}

fn error_json(status: &str, code: &str, os_code: Option<u32>) -> String {
    format!(
        "{{\"status\":{},\"code\":{},\"os_code\":{}}}",
        json(status),
        json(code),
        option_u32_common(os_code)
    )
}

fn json(value: &str) -> String {
    let mut out = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

fn option_u32_common(value: Option<u32>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_is_the_only_default_mode() {
        assert_eq!(
            parse_invocation(&["--adapter-pid", "4006", "--json"]),
            Ok(Invocation::ReadOnly)
        );
        assert_eq!(
            parse_invocation(&["--adapter-pid", "4006", "--read-only", "--json"]),
            Ok(Invocation::ReadOnly)
        );
    }

    #[test]
    fn explicit_state_change_is_distinct_and_order_independent() {
        assert_eq!(
            parse_invocation(&[
                "--json",
                "--allow-state-change",
                "start",
                "--adapter-pid",
                "4006",
            ]),
            Ok(Invocation::SetEnabled(true))
        );
        assert_eq!(
            parse_invocation(&[
                "--adapter-pid",
                "4006",
                "--allow-state-change",
                "stop",
                "--json",
            ]),
            Ok(Invocation::SetEnabled(false))
        );
    }

    #[test]
    fn invalid_or_ambiguous_state_change_is_rejected() {
        assert_eq!(
            parse_invocation(&[
                "--adapter-pid",
                "4006",
                "--read-only",
                "--allow-state-change",
                "start",
                "--json",
            ]),
            Err(CliError::InvalidArguments)
        );
        assert_eq!(
            parse_invocation(&[
                "--adapter-pid",
                "4006",
                "--allow-state-change",
                "restart",
                "--json",
            ]),
            Err(CliError::InvalidArguments)
        );
        assert_eq!(
            parse_invocation(&["--adapter-pid", "4009", "--json"]),
            Err(CliError::UnsupportedDevice)
        );
        assert_eq!(
            parse_invocation(&["--adapter-pid", "4006"]),
            Err(CliError::InvalidArguments)
        );
    }

    #[test]
    fn errors_are_stable_json_and_never_include_identity_tokens() {
        let rendered = error_json("status_error", "hotspot:access_denied", Some(5));
        assert_eq!(
            rendered,
            r#"{"status":"status_error","code":"hotspot:access_denied","os_code":5}"#
        );
        assert!(!rendered.contains("SSID"));
        assert!(!rendered.contains("password"));
    }

    #[derive(Default)]
    struct BlockingWorker {
        terminated: bool,
    }

    impl WorkerProcess for BlockingWorker {
        fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
            Ok(None)
        }

        fn terminate(&mut self) -> std::io::Result<()> {
            self.terminated = true;
            Ok(())
        }
    }

    #[test]
    fn supervisor_returns_at_deadline_and_terminates_a_blocking_worker() {
        let mut worker = BlockingWorker::default();
        let result = supervise_worker(&mut worker, Instant::now() + Duration::from_millis(1));
        assert_eq!(result, WorkerWait::DeadlineExceeded);
        assert!(worker.terminated);
    }

    #[test]
    fn worker_protocol_rejects_unknown_modes() {
        assert_eq!(
            parse_worker_invocation(&["--mode", "restart"]),
            Err(CliError::InvalidArguments)
        );
        assert_eq!(
            parse_worker_invocation(&["--mode", "read-only"]),
            Ok(Invocation::ReadOnly)
        );
    }
}
