#![forbid(unsafe_code)]

use std::{fmt::Write, time::SystemTime};

use dji4g_domain::DeviceEpoch;
use dji4g_windows_platform::{
    AddressFamily, BoundRouteEvidence, ConnectEvidence, EndpointAttempt, GlobalRouteEvidence,
    HttpEvidence, ProbeStage, TimedStage, TlsEvidence, WindowsAdapterResolver,
    WindowsDeviceInventory, WindowsNetworkProbe,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Invocation {
    Run,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CliError {
    UnsupportedDevice,
    InvalidArguments,
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let exit_code = match parse_invocation(&args) {
        Ok(Invocation::Run) => run_probe(),
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

fn parse_invocation<S: AsRef<str>>(args: &[S]) -> Result<Invocation, CliError> {
    if args.len() != 3 || args[0].as_ref() != "--adapter-pid" || args[2].as_ref() != "--json" {
        return Err(CliError::InvalidArguments);
    }
    if args[1].as_ref() != "4006" {
        return Err(CliError::UnsupportedDevice);
    }
    Ok(Invocation::Run)
}

fn run_probe() -> i32 {
    let inventory = match WindowsDeviceInventory.scan_now() {
        Ok(value) => value,
        Err(error) => return print_error("inventory_error", error.code, error.os_code),
    };
    let device = match inventory.devices() {
        [] => return print_error("not_detected", "pnp:not_detected", None),
        [device] => device,
        _ => return print_error("ambiguous_device", "pnp:ambiguous_device", None),
    };
    let observation = match WindowsAdapterResolver.resolve(device, DeviceEpoch(1)) {
        Ok(value) => value,
        Err(error) => return print_error("adapter_error", error.code, error.os_code),
    };
    let result = match WindowsNetworkProbe.observe_now(&observation.identity, &Default::default()) {
        Ok(value) => value,
        Err(error) => return print_error("probe_error", error.code, error.os_code),
    };
    let mut output = format!(
        "{{\"status\":\"observed\",\"epoch\":{},\"adapter_guid\":{},\"luid\":{},\"ipv4_ifindex\":{},\"ipv6_ifindex\":{},",
        result.epoch.0,
        json(&result.adapter_guid),
        observation.identity.luid(),
        option_u32(observation.identity.ipv4_index()),
        option_u32(observation.identity.ipv6_index()),
    );
    output.push_str("\"target_routes\":[");
    for (index, route) in observation.routes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"family\":{},\"ifindex\":{},\"destination\":{},\"prefix_len\":{},\"next_hop\":{},\"total_metric\":{}}}",
            json(family_name(route.family)),
            route.interface_index,
            json(&route.destination.to_string()),
            route.prefix_len,
            json(&route.next_hop.to_string()),
            route.total_metric,
        );
    }
    output.push_str("],\"families\":[");
    for (index, family) in result.families.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"family\":{},\"ifindex\":{},\"source\":{},\"dns\":{},\"endpoints\":[",
            json(family_name(family.family)),
            family.interface_index,
            family
                .selected_source
                .map_or_else(|| "null".to_owned(), |ip| json(&ip.to_string())),
            dns_stage(&family.dns)
        );
        for (endpoint_index, endpoint) in family.endpoints.iter().enumerate() {
            if endpoint_index > 0 {
                output.push(',');
            }
            output.push_str(&endpoint_attempt(endpoint));
        }
        output.push_str("]}");
    }
    output.push_str("],\"global_routes\":[");
    for (index, route) in result.global_routes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"family\":{},\"target_luid\":{},\"route\":{},\"explanation_only\":true}}",
            json(family_name(route.family)),
            route.target_luid,
            global_stage(&route.route),
        );
    }
    let _ = write!(
        output,
        "],\"started_at_ms\":{},\"finished_at_ms\":{},\"elapsed_ms\":{}}}",
        unix_millis(result.started_at),
        unix_millis(result.finished_at),
        result.elapsed.as_millis(),
    );
    println!("{output}");
    0
}

fn stage_meta<T>(value: &TimedStage<T>) -> String {
    format!(
        "\"started_at_ms\":{},\"finished_at_ms\":{},\"elapsed_ms\":{}",
        unix_millis(value.started_at),
        unix_millis(value.finished_at),
        value.elapsed.as_millis()
    )
}

fn stage_failure<T>(value: &TimedStage<T>) -> Option<String> {
    match &value.outcome {
        ProbeStage::Failed { code, os_code } => Some(format!(
            "{{\"status\":\"failed\",\"code\":{},\"os_code\":{},{}}}",
            json(code),
            option_u32(*os_code),
            stage_meta(value)
        )),
        ProbeStage::Unavailable { code } => Some(format!(
            "{{\"status\":\"unavailable\",\"code\":{},{}}}",
            json(code),
            stage_meta(value)
        )),
        ProbeStage::Unexecuted { code } => Some(format!(
            "{{\"status\":\"unexecuted\",\"code\":{},{}}}",
            json(code),
            stage_meta(value)
        )),
        ProbeStage::Succeeded(_) => None,
    }
}

fn dns_stage(value: &TimedStage<Vec<std::net::IpAddr>>) -> String {
    if let Some(failure) = stage_failure(value) {
        return failure;
    }
    let ProbeStage::Succeeded(addresses) = &value.outcome else {
        unreachable!()
    };
    format!(
        "{{\"status\":\"succeeded\",\"answers\":[{}],{}}}",
        addresses
            .iter()
            .map(|ip| json(&ip.to_string()))
            .collect::<Vec<_>>()
            .join(","),
        stage_meta(value)
    )
}

fn endpoint_attempt(value: &EndpointAttempt) -> String {
    format!(
        "{{\"endpoint_id\":{},\"destination\":{},\"route\":{},\"connect\":{},\"tls\":{},\"http\":{},\"started_at_ms\":{},\"finished_at_ms\":{},\"elapsed_ms\":{}}}",
        json(value.endpoint_id),
        json(&value.destination.to_string()),
        route_stage(&value.route),
        connect_stage(&value.connect),
        tls_stage(&value.tls),
        http_stage(&value.http),
        unix_millis(value.started_at),
        unix_millis(value.finished_at),
        value.elapsed.as_millis(),
    )
}

fn route_stage(value: &TimedStage<BoundRouteEvidence>) -> String {
    if let Some(failure) = stage_failure(value) {
        return failure;
    }
    let ProbeStage::Succeeded(route) = &value.outcome else {
        unreachable!()
    };
    format!(
        "{{\"status\":\"succeeded\",\"luid\":{},\"source\":{},\"ifindex\":{},\"next_hop\":{},{}}}",
        route.luid,
        json(&route.source.to_string()),
        route.route.interface_index,
        json(&route.route.next_hop.to_string()),
        stage_meta(value)
    )
}

fn connect_stage(value: &TimedStage<ConnectEvidence>) -> String {
    if let Some(failure) = stage_failure(value) {
        return failure;
    }
    let ProbeStage::Succeeded(connect) = &value.outcome else {
        unreachable!()
    };
    format!(
        "{{\"status\":\"succeeded\",\"actual_source\":{},{}}}",
        json(&connect.actual_source.to_string()),
        stage_meta(value)
    )
}

fn tls_stage(value: &TimedStage<TlsEvidence>) -> String {
    if let Some(failure) = stage_failure(value) {
        return failure;
    }
    let ProbeStage::Succeeded(tls) = &value.outcome else {
        unreachable!()
    };
    format!(
        "{{\"status\":\"succeeded\",\"validated\":{},{}}}",
        tls.validated,
        stage_meta(value)
    )
}

fn http_stage(value: &TimedStage<HttpEvidence>) -> String {
    if let Some(failure) = stage_failure(value) {
        return failure;
    }
    let ProbeStage::Succeeded(http) = &value.outcome else {
        unreachable!()
    };
    format!(
        "{{\"status\":\"succeeded\",\"http_status\":{},\"response_bytes\":{},{}}}",
        http.status,
        http.response_bytes,
        stage_meta(value)
    )
}

fn global_stage(value: &TimedStage<GlobalRouteEvidence>) -> String {
    if let Some(failure) = stage_failure(value) {
        return failure;
    }
    let ProbeStage::Succeeded(route) = &value.outcome else {
        unreachable!()
    };
    format!(
        "{{\"status\":\"succeeded\",\"global_luid\":{},\"ifindex\":{},\"owner\":{},{}}}",
        route
            .global_luid
            .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        option_u32(route.interface_index),
        json(&format!("{:?}", route.owner)),
        stage_meta(value)
    )
}

fn family_name(value: AddressFamily) -> &'static str {
    match value {
        AddressFamily::Ipv4 => "ipv4",
        AddressFamily::Ipv6 => "ipv6",
    }
}

fn unix_millis(value: SystemTime) -> u128 {
    value
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn option_u32(value: Option<u32>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn print_error(status: &str, code: &str, os_code: Option<u32>) -> i32 {
    println!("{}", error_json(status, code, os_code));
    1
}

fn error_json(status: &str, code: &str, os_code: Option<u32>) -> String {
    format!(
        "{{\"status\":{},\"code\":{},\"os_code\":{}}}",
        json(status),
        json(code),
        option_u32(os_code)
    )
}

fn json(value: &str) -> String {
    let mut out = String::from("\"");
    for character in value.chars() {
        match character {
            '\"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            character if character.is_control() => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('\"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_first_generation_invocation_is_accepted() {
        assert_eq!(
            parse_invocation(&["--adapter-pid", "4006", "--json"]),
            Ok(Invocation::Run)
        );
    }

    #[test]
    fn other_pid_is_a_stable_unsupported_device_error() {
        assert_eq!(
            parse_invocation(&["--adapter-pid", "4009", "--json"]),
            Err(CliError::UnsupportedDevice)
        );
        assert!(
            error_json("unsupported_device", "pnp:unsupported_device", None)
                .contains("\"status\":\"unsupported_device\"")
        );
    }

    #[test]
    fn malformed_invocation_is_a_stable_usage_error() {
        assert_eq!(
            parse_invocation(&["--json"]),
            Err(CliError::InvalidArguments)
        );
    }
}
