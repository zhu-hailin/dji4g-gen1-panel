//! Classification tests for the platform tool exchange.
//!
//! These run without a module: they pin how an actor-level result becomes "the module answered",
//! "the answer made no sense" or "nothing usable came back, and here is whether anything was
//! written".

use std::io;

use super::{ToolExchangeOutcome, UnansweredReason, classify_exchange};
use crate::ActorError;
use dji4g_at_protocol::{AtFinalCode, ToolParseError, ToolResponse};

fn response(final_code: AtFinalCode) -> ToolResponse {
    ToolResponse {
        lines: Vec::new(),
        urc_lines: Vec::new(),
        unclassified_lines: 0,
        final_code,
    }
}

#[test]
fn a_final_code_is_an_answer_even_when_it_is_an_error() {
    let exchange = classify_exchange(Ok(response(AtFinalCode::Error)), true);
    match exchange.outcome {
        ToolExchangeOutcome::Answered(response) => {
            assert_eq!(response.final_code, AtFinalCode::Error);
            assert_eq!(response.final_code_tag(), "error");
        }
        other => panic!("expected an answer, got {other:?}"),
    }
    // A module refusal is never reported as a written-but-unanswered transaction.
    let exchange = classify_exchange(
        Ok(response(AtFinalCode::CmeError("SENTINEL".to_owned()))),
        true,
    );
    assert!(matches!(exchange.outcome, ToolExchangeOutcome::Answered(_)));
    assert!(!format!("{:?}", exchange.outcome).contains("SENTINEL"));
}

#[test]
fn a_parser_refusal_is_malformed_and_never_a_success() {
    for error in [
        ToolParseError::UnsupportedInteraction,
        ToolParseError::LineTooLong,
        ToolParseError::ResponseTooLarge,
        ToolParseError::TooManyLines,
        ToolParseError::UnexpectedData,
    ] {
        let exchange = classify_exchange(Err(ActorError::Tool(error)), true);
        match exchange.outcome {
            ToolExchangeOutcome::Malformed(found) => assert_eq!(found, error),
            other => panic!("expected malformed, got {other:?}"),
        }
    }
}

#[test]
fn cancellation_and_deadlines_keep_the_write_attempt() {
    let cancelled = classify_exchange(Err(ActorError::Io(io::ErrorKind::Interrupted)), false);
    assert_eq!(
        cancelled.outcome,
        ToolExchangeOutcome::Unanswered {
            wrote: false,
            reason: UnansweredReason::Cancelled
        }
    );
    let cancelled_after_write =
        classify_exchange(Err(ActorError::Io(io::ErrorKind::Interrupted)), true);
    assert_eq!(
        cancelled_after_write.outcome,
        ToolExchangeOutcome::Unanswered {
            wrote: true,
            reason: UnansweredReason::Cancelled
        }
    );
    let expired = classify_exchange(Err(ActorError::Io(io::ErrorKind::TimedOut)), false);
    assert!(matches!(
        expired.outcome,
        ToolExchangeOutcome::Unanswered {
            wrote: false,
            reason: UnansweredReason::Deadline
        }
    ));
}

#[test]
fn transport_and_session_failures_are_told_apart() {
    let transport = classify_exchange(Err(ActorError::Io(io::ErrorKind::NotConnected)), true);
    assert!(matches!(
        transport.outcome,
        ToolExchangeOutcome::Unanswered {
            wrote: true,
            reason: UnansweredReason::Transport
        }
    ));
    for error in [
        ActorError::CloseTimeout,
        ActorError::Closed,
        ActorError::LeaseBusy,
    ] {
        let exchange = classify_exchange(Err(error), true);
        assert!(matches!(
            exchange.outcome,
            ToolExchangeOutcome::Unanswered {
                wrote: true,
                reason: UnansweredReason::SessionUnavailable
            }
        ));
    }
}
