#![forbid(unsafe_code)]

//! Typed AT protocol support for the first-generation DJI 4G module.

mod command;
mod model;
mod parser;
mod pdp;
mod redact;
mod sms_pdu;
mod tool_command;
mod tool_parser;

pub use command::{AtCommand, Effect, EncodedAtCommand, RetryPolicy, Sensitivity};
pub use model::{
    Apn, ApnError, AtEvent, AtFinalCode, AtResponse, AtUrc, PdpContext, PdpContextId,
    PdpContextIdError, PdpContextState, PdpType, ProtocolError, ProtocolErrorKind,
    SensorTemperature, VerifiedUsbNetProfile,
};
pub use parser::{
    CnumParseError, StreamingParser, at_csv, parse_cmti_line, parse_cnum_lines, parse_iccid_line,
    parse_qtemp_lines, parse_serving_cell_line,
};
pub use pdp::{PdpParseError, parse_pdp_contexts, parse_pdp_contexts_with_activity};
pub use redact::{redact_at_text, redact_at_transaction_line};
pub use sms_pdu::{
    DecodedSms, EncodedSubmit, SmsPduError, build_ucs2_submit, decode_deliver_pdu,
    validate_sms_recipient,
};
pub use tool_command::{
    MAX_TOOL_LINE_BYTES, ToolInputError, ToolReadId, ToolWriteId, ValidatedToolLine,
    classify_known_write, classify_read, typed_read,
};
pub use tool_parser::{ToolParseError, ToolResponse, ToolResponseParser, ToolWireRequest};
