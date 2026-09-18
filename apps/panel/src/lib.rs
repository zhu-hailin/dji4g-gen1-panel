#![forbid(unsafe_code)]

//! Snapshot-driven user interface for the DJI first-generation 4G panel.
//!
//! The library boundary deliberately contains only immutable application snapshots and
//! closed [`dji4g_application::UiCommand`] values. Platform handles and privileged work stay
//! in the application/platform crates.

pub mod app;
pub mod config;
pub mod diagnostics_export;
pub mod feature_probe;
pub mod font;
pub mod localization;
pub mod logging;
pub mod native_dialog;
pub mod runtime;
pub mod support_report;
pub mod tray;
pub mod ui;

#[cfg(debug_assertions)]
pub mod demo;
