#![forbid(unsafe_code)]

use std::fmt::Write;

use dji4g_domain::DeviceEpoch;
use dji4g_windows_platform::{
    AtSessionActor, DjiDevice, FunctionRole, InventorySnapshot, WindowsDeviceInventory,
};

fn main() {
    if !std::env::args().any(|argument| argument == "--json") {
        eprintln!("usage: pnp_spike --json");
        std::process::exit(2);
    }

    let inventory = WindowsDeviceInventory;
    match inventory.scan_now() {
        Ok(snapshot) => println!("{}", render_snapshot(snapshot)),
        Err(error) => println!(
            "{{\"status\":\"inventory_error\",\"error_code\":{},\"os_code\":{}}}",
            json_string(error.code),
            optional_u32(error.os_code)
        ),
    }
}

fn render_snapshot(snapshot: InventorySnapshot) -> String {
    let mut output = String::from("{\"status\":");
    let status = match snapshot.devices().len() {
        0 => "not_detected",
        1 => "detected",
        _ => "ambiguous_device",
    };
    let _ = write!(
        output,
        "{},\"device_count\":{},\"devices\":[",
        json_string(status),
        snapshot.devices().len()
    );
    for (index, device) in snapshot.devices().iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        render_device(&mut output, device);
    }
    output.push(']');

    if let [device] = snapshot.devices() {
        match device.select_at_port() {
            Ok(selected) => {
                let _ = write!(
                    output,
                    ",\"selected_port\":{},\"selected_kind\":{}",
                    json_string(selected.port_name()),
                    json_string(match selected.kind() {
                        dji4g_windows_platform::SelectedPortKind::DedicatedAt => "dedicated_at",
                        dji4g_windows_platform::SelectedPortKind::VerifiedModem => {
                            "verified_modem"
                        }
                    })
                );
                match AtSessionActor::open_selected(DeviceEpoch(1), &selected)
                    .and_then(|actor| actor.safe_handshake())
                {
                    Ok(_) => output
                        .push_str(",\"handshake\":\"ok\",\"identity\":\"[REDACTED_AT_IDENTITY]\""),
                    Err(error) => {
                        let _ = write!(
                            output,
                            ",\"handshake\":\"unavailable\",\"error_code\":{}",
                            json_string(error.code())
                        );
                    }
                }
            }
            Err(error) => {
                let _ = write!(
                    output,
                    ",\"selection\":\"unavailable\",\"error_code\":{}",
                    json_string(&error.to_string())
                );
            }
        }
    }
    output.push('}');
    output
}

fn render_device(output: &mut String, device: &DjiDevice) {
    let _ = write!(
        output,
        "{{\"root_instance_id\":{},\"container_id\":{},\"problem_code\":{},\"com\":[",
        json_string(&redact_instance_id(device.root_instance_id())),
        if device.container_id().is_some() {
            json_string("[REDACTED_CONTAINER_ID]")
        } else {
            "null".to_owned()
        },
        optional_u32(device.problem_code())
    );
    for (index, candidate) in device.com_candidates().iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"port_name\":{},\"role\":{},\"instance_id\":{},\"interface_path\":\"[REDACTED_DEVICE_INTERFACE]\",\"problem_code\":{}}}",
            json_string(candidate.port_name()),
            json_string(role_name(candidate.role())),
            json_string(&redact_instance_id(candidate.instance_id())),
            optional_u32(candidate.problem_code())
        );
    }
    output.push_str("],\"net\":[");
    for (index, candidate) in device.net_candidates().iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"instance_id\":{},\"interface_path\":\"[REDACTED_DEVICE_INTERFACE]\",\"netcfg_instance_id\":{},\"problem_code\":{}}}",
            json_string(&redact_instance_id(candidate.instance_id())),
            if candidate.net_cfg_instance_id().is_some() {
                json_string("[REDACTED_ADAPTER_GUID]")
            } else {
                "null".to_owned()
            },
            optional_u32(candidate.problem_code())
        );
    }
    output.push_str("]}");
}

fn role_name(role: FunctionRole) -> &'static str {
    match role {
        FunctionRole::DedicatedAt => "dedicated_at",
        FunctionRole::Modem => "modem",
        FunctionRole::DmDiag => "dm_diag",
        FunctionRole::Nmea => "nmea",
        FunctionRole::Unknown => "unknown",
    }
}

fn redact_instance_id(value: &str) -> String {
    value.rsplit_once('\\').map_or_else(
        || "[REDACTED_DEVICE_INSTANCE]".to_owned(),
        |(prefix, _)| format!("{prefix}\\[REDACTED_INSTANCE_TOKEN]"),
    )
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value.is_control() => {
                let _ = write!(output, "\\u{:04x}", value as u32);
            }
            value => output.push(value),
        }
    }
    output.push('"');
    output
}

fn optional_u32(value: Option<u32>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}
