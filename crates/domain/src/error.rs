use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum ErrorCode {
    PermissionDenied,
    DeviceRemoved,
    DeviceIdentityChanged,
    EvidenceExpired,
    ProbeFailed,
    DnsFailed,
    Timeout,
    Unsupported,
    CapabilityUnavailable,
    OperationCancelled,
    VerificationFailed,
    RollbackFailed,
    Internal,
}

#[derive(Clone, Debug, Eq, Error, PartialEq, Serialize, Deserialize)]
#[error("domain operation failed with {code:?}")]
pub struct DomainError {
    pub code: ErrorCode,
}
