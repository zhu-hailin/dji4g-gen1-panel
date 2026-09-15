#![cfg_attr(
    all(windows, not(test), not(debug_assertions)),
    windows_subsystem = "windows"
)]
#![forbid(unsafe_code)]

use std::{ffi::OsString, process::ExitCode};

use dji4g_ipc::{OperationNonce, PipeName, ProtocolVersion};

#[derive(Debug)]
struct HelperArgs {
    pipe: PipeName,
    nonce: OperationNonce,
    protocol: ProtocolVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HelperExitCode {
    InvalidArguments,
    ProtocolRejected,
    PeerRejected,
    Timeout,
    OperationCancelled,
    UnsupportedPlatform,
    Internal,
}

impl HelperExitCode {
    const fn as_u8(self) -> u8 {
        match self {
            Self::InvalidArguments => 64,
            Self::ProtocolRejected => 65,
            Self::PeerRejected => 66,
            Self::Timeout => 67,
            Self::OperationCancelled => 68,
            Self::UnsupportedPlatform => 69,
            Self::Internal => 70,
        }
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args_os()) {
        Ok(args) => args,
        Err(error) => return ExitCode::from(error.as_u8()),
    };
    match run_once(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => ExitCode::from(error.as_u8()),
    }
}

fn parse_args<I>(args: I) -> Result<HelperArgs, HelperExitCode>
where
    I: IntoIterator<Item = OsString>,
{
    let mut values = args.into_iter();
    let _program = values.next().ok_or(HelperExitCode::InvalidArguments)?;
    let mut pipe = None;
    let mut nonce = None;
    let mut protocol = None;
    while let Some(flag) = values.next() {
        let flag = flag
            .into_string()
            .map_err(|_| HelperExitCode::InvalidArguments)?;
        let slot = match flag.as_str() {
            "--pipe" => &mut pipe,
            "--nonce" => &mut nonce,
            "--protocol" => &mut protocol,
            _ => return Err(HelperExitCode::InvalidArguments),
        };
        if slot.is_some() {
            return Err(HelperExitCode::InvalidArguments);
        }
        let value = values.next().ok_or(HelperExitCode::InvalidArguments)?;
        let value = value
            .into_string()
            .map_err(|_| HelperExitCode::InvalidArguments)?;
        if value.starts_with('-') || value.is_empty() {
            return Err(HelperExitCode::InvalidArguments);
        }
        *slot = Some(value);
    }

    let pipe = pipe
        .as_deref()
        .and_then(PipeName::parse)
        .ok_or(HelperExitCode::InvalidArguments)?;
    let nonce = nonce
        .as_deref()
        .and_then(OperationNonce::from_hex)
        .ok_or(HelperExitCode::InvalidArguments)?;
    if nonce.is_zero() {
        return Err(HelperExitCode::InvalidArguments);
    }
    let protocol = match protocol.as_deref() {
        Some("1") => ProtocolVersion::V1,
        _ => return Err(HelperExitCode::InvalidArguments),
    };
    Ok(HelperArgs {
        pipe,
        nonce,
        protocol,
    })
}

fn run_once(args: HelperArgs) -> Result<(), HelperExitCode> {
    #[cfg(windows)]
    {
        dji4g_windows_platform::run_helper_once(args.pipe, args.nonce, args.protocol)
            .map_err(map_privilege_error)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err(HelperExitCode::UnsupportedPlatform)
    }
}

#[cfg(windows)]
fn map_privilege_error(error: dji4g_windows_platform::PrivilegeError) -> HelperExitCode {
    use dji4g_windows_platform::PrivilegeError;
    match error {
        PrivilegeError::Protocol(_) => HelperExitCode::ProtocolRejected,
        PrivilegeError::Peer(_) => HelperExitCode::PeerRejected,
        PrivilegeError::Timeout => HelperExitCode::Timeout,
        PrivilegeError::OperationCancelled => HelperExitCode::OperationCancelled,
        PrivilegeError::UnsupportedPlatform => HelperExitCode::UnsupportedPlatform,
        // All trust-boundary failures (path, unsigned, unverifiable) collapse to the generic
        // internal exit code; the helper itself never re-verifies the panel's signature decision.
        PrivilegeError::HelperUntrusted
        | PrivilegeError::HelperUnsigned
        | PrivilegeError::HelperVerificationUnavailable
        | PrivilegeError::LaunchFailed
        | PrivilegeError::Internal => HelperExitCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Vec<OsString> {
        vec![
            "helper.exe".into(),
            "--pipe".into(),
            r"\\.\pipe\dji4g-panel-000102030405060708090a0b0c0d0e0f".into(),
            "--nonce".into(),
            "0202020202020202020202020202020202020202020202020202020202020202".into(),
            "--protocol".into(),
            "1".into(),
        ]
    }

    #[test]
    fn parser_accepts_only_the_three_internal_flags() {
        let parsed = parse_args(valid()).unwrap();
        assert_eq!(parsed.protocol, ProtocolVersion::V1);
        assert_eq!(
            parsed.pipe.as_str(),
            r"\\.\pipe\dji4g-panel-000102030405060708090a0b0c0d0e0f"
        );
    }

    #[test]
    fn parser_rejects_unknown_duplicate_missing_and_raw_action_flags() {
        for args in [
            vec!["helper.exe", "--action", "restart"],
            vec!["helper.exe", "--pipe"],
            vec![
                "helper.exe",
                "--nonce",
                "0202020202020202020202020202020202020202020202020202020202020202",
                "--nonce",
                "0202020202020202020202020202020202020202020202020202020202020202",
            ],
            vec![
                "helper.exe",
                "--pipe",
                "\\\\.\\pipe\\dji4g-panel-000102030405060708090a0b0c0d0e0f",
                "--nonce",
                "0202020202020202020202020202020202020202020202020202020202020202",
                "--protocol",
                "2",
            ],
            vec![
                "helper.exe",
                "--pipe",
                "\\\\.\\pipe\\dji4g-panel-000102030405060708090a0b0c0d0e0f",
                "--nonce",
                "not-hex",
                "--protocol",
                "1",
            ],
        ] {
            assert!(matches!(
                parse_args(args.into_iter().map(OsString::from)),
                Err(HelperExitCode::InvalidArguments)
            ));
        }
    }
}
