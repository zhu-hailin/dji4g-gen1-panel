//! Privacy-first bounded local logging.
//!
//! Callers pass stable event fields to this module.  Values are redacted before they are formatted
//! into a line, and only files with the application's exact prefix are rotated or removed.

use std::{
    cmp::Reverse,
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

use dji4g_application::LogLevel;

const LOG_PREFIX: &str = "dji4g-panel";
const ACTIVE_FILE: &str = "dji4g-panel.log";
const DEFAULT_MAX_FILE_BYTES: u64 = 1024 * 1024;
const DEFAULT_MAX_FILES: usize = 5;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

static ACTIVE_LEVEL: OnceLock<Mutex<Option<Arc<AtomicU8>>>> = OnceLock::new();
static ACTIVE_WRITER: OnceLock<Mutex<Option<std::sync::Weak<Mutex<RollingLog>>>>> = OnceLock::new();

/// Stable, content-free SMS events go to the same bounded writer as diagnostic logging.
pub fn record_sms_state(state: &dji4g_application::SmsSendSnapshot) {
    if let Ok(json) = serde_json::to_string(state) {
        append_event(&format!("sms_state {json}"));
    }
}

pub(crate) fn append_event(event: &str) {
    let writer = ACTIVE_WRITER
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.as_ref().and_then(std::sync::Weak::upgrade));
    if let Some(writer) = writer {
        if let Ok(mut writer) = writer.lock() {
            let _ = writer.append(event);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggingConfig {
    pub directory: PathBuf,
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub level: LogLevel,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("logs"),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_files: DEFAULT_MAX_FILES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            level: LogLevel::Info,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggingError {
    stable_code: &'static str,
    os_code: Option<u32>,
}

impl LoggingError {
    #[must_use]
    pub const fn new(stable_code: &'static str) -> Self {
        Self {
            stable_code,
            os_code: None,
        }
    }

    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        self.stable_code
    }

    #[must_use]
    pub const fn os_code(&self) -> Option<u32> {
        self.os_code
    }
}

impl std::fmt::Display for LoggingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.stable_code)
    }
}

impl std::error::Error for LoggingError {}

fn io_error(code: &'static str, error: io::Error) -> LoggingError {
    LoggingError {
        stable_code: code,
        os_code: error
            .raw_os_error()
            .and_then(|value| u32::try_from(value).ok()),
    }
}

pub struct LoggingGuard {
    writer: Arc<Mutex<RollingLog>>,
    level: Arc<AtomicU8>,
}

impl LoggingGuard {
    pub fn append(&self, line: &str) -> Result<(), LoggingError> {
        self.writer
            .lock()
            .map_err(|_| LoggingError::new("logging:lock_poisoned"))?
            .append(line)
    }

    #[must_use]
    pub fn level(&self) -> LogLevel {
        level_from_byte(self.level.load(Ordering::Relaxed))
    }

    #[must_use]
    pub fn active_path(&self) -> PathBuf {
        self.writer
            .lock()
            .map(|writer| writer.active_path().to_path_buf())
            .unwrap_or_default()
    }
}

pub fn init_logging(config: LoggingConfig) -> Result<LoggingGuard, LoggingError> {
    let level = Arc::new(AtomicU8::new(level_byte(config.level)));
    let writer = Arc::new(Mutex::new(RollingLog::new(config)?));
    if let Ok(mut slot) = ACTIVE_WRITER.get_or_init(|| Mutex::new(None)).lock() {
        *slot = Some(Arc::downgrade(&writer));
    }
    let slot = ACTIVE_LEVEL.get_or_init(|| Mutex::new(None));
    if let Ok(mut current) = slot.lock() {
        *current = Some(Arc::clone(&level));
    }
    // Keep the tracing dependency at the boundary, but only submit a stable event field.  The
    // actual bytes are written through `LoggingGuard::append`, after redaction.
    tracing::debug!(event = "logging_initialized");
    Ok(LoggingGuard { writer, level })
}

pub fn reload_level(level: LogLevel) -> Result<(), LoggingError> {
    let Some(slot) = ACTIVE_LEVEL.get() else {
        return Err(LoggingError::new("logging:not_initialized"));
    };
    let current = slot
        .lock()
        .map_err(|_| LoggingError::new("logging:lock_poisoned"))?
        .clone()
        .ok_or_else(|| LoggingError::new("logging:not_initialized"))?;
    current.store(level_byte(level), Ordering::Relaxed);
    Ok(())
}

#[derive(Debug)]
pub struct RollingLog {
    config: LoggingConfig,
    active_path: PathBuf,
    file: Option<File>,
    size: u64,
}

impl RollingLog {
    pub fn new(mut config: LoggingConfig) -> Result<Self, LoggingError> {
        config.max_file_bytes = config.max_file_bytes.max(1);
        config.max_files = config.max_files.max(1);
        config.max_total_bytes = config.max_total_bytes.max(1);
        fs::create_dir_all(&config.directory)
            .map_err(|error| io_error("logging:init_failed", error))?;
        let active_path = config.directory.join(ACTIVE_FILE);
        let mut writer = Self {
            config,
            active_path,
            file: None,
            size: 0,
        };
        writer.rotate_startup()?;
        writer.open_active()?;
        writer.enforce_total()?;
        Ok(writer)
    }

    #[must_use]
    pub fn active_path(&self) -> &Path {
        &self.active_path
    }

    #[must_use]
    pub fn config(&self) -> &LoggingConfig {
        &self.config
    }

    pub fn append(&mut self, line: &str) -> Result<(), LoggingError> {
        let redacted = redact_sensitive(line);
        let mut bytes = redacted.into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let max = self.config.max_file_bytes.min(self.config.max_total_bytes) as usize;
        if bytes.len() > max {
            bytes.truncate(max);
            if let Some(last) = bytes.last_mut() {
                *last = b'\n';
            }
        }
        if self.size.saturating_add(bytes.len() as u64) > self.config.max_file_bytes {
            self.rotate()?;
        }
        if self.size.saturating_add(bytes.len() as u64) > self.config.max_total_bytes {
            self.truncate_active(self.config.max_total_bytes.saturating_sub(self.size))?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| LoggingError::new("logging:writer_unavailable"))?;
        file.write_all(&bytes)
            .and_then(|()| file.flush())
            .map_err(|error| io_error("logging:write_failed", error))?;
        self.size = self.size.saturating_add(bytes.len() as u64);
        self.enforce_total()
    }

    fn rotate_startup(&mut self) -> Result<(), LoggingError> {
        if !self.active_path.exists() {
            return Ok(());
        }
        self.rotate_owned_files()?;
        Ok(())
    }

    fn rotate(&mut self) -> Result<(), LoggingError> {
        self.file.take();
        self.rotate_owned_files()?;
        self.open_active()
    }

    fn rotate_owned_files(&self) -> Result<(), LoggingError> {
        // Oldest-to-newest names are shifted in descending order.  Only exact generated names
        // are touched; e.g. `dji4g-panel-user-export.json` is left alone.
        if self.config.max_files > 1 {
            for index in (1..self.config.max_files).rev() {
                let source = self.rotated_path(index - 1);
                let destination = self.rotated_path(index);
                if source.exists() {
                    if destination.exists() {
                        fs::remove_file(&destination)
                            .map_err(|error| io_error("logging:rotation_failed", error))?;
                    }
                    fs::rename(&source, &destination)
                        .map_err(|error| io_error("logging:rotation_failed", error))?;
                }
            }
        }
        if self.active_path.exists() {
            let destination = self.rotated_path(1);
            if self.config.max_files == 1 {
                fs::remove_file(&self.active_path)
                    .map_err(|error| io_error("logging:rotation_failed", error))?;
            } else {
                if destination.exists() {
                    fs::remove_file(&destination)
                        .map_err(|error| io_error("logging:rotation_failed", error))?;
                }
                fs::rename(&self.active_path, destination)
                    .map_err(|error| io_error("logging:rotation_failed", error))?;
            }
        }
        Ok(())
    }

    fn open_active(&mut self) -> Result<(), LoggingError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&self.active_path)
            .map_err(|error| io_error("logging:init_failed", error))?;
        let size = file
            .seek(SeekFrom::End(0))
            .map_err(|error| io_error("logging:init_failed", error))?;
        if size > self.config.max_file_bytes {
            drop(file);
            self.file = None;
            self.size = size;
            self.rotate()?;
            return Ok(());
        }
        self.size = size;
        self.file = Some(file);
        Ok(())
    }

    fn truncate_active(&mut self, to_size: u64) -> Result<(), LoggingError> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| LoggingError::new("logging:writer_unavailable"))?;
        file.set_len(to_size)
            .and_then(|()| file.seek(SeekFrom::End(0)).map(|_| ()))
            .map_err(|error| io_error("logging:write_failed", error))?;
        self.size = to_size;
        Ok(())
    }

    fn enforce_total(&mut self) -> Result<(), LoggingError> {
        let mut files = owned_files(&self.config.directory)?;
        // Suffix 1 is the newest rotated file; evict the largest suffix first so retaining the
        // bounded total keeps the most recent diagnostic context.
        files.sort_by_key(|(index, _path, size)| (Reverse(*index), Reverse(*size)));
        let mut total = files.iter().map(|(_, _, size)| *size).sum::<u64>();
        for (index, path, size) in files {
            if total <= self.config.max_total_bytes {
                break;
            }
            if path == self.active_path {
                continue;
            }
            fs::remove_file(path).map_err(|error| io_error("logging:rotation_failed", error))?;
            total = total.saturating_sub(size);
            let _ = index;
        }
        if total > self.config.max_total_bytes && self.size > self.config.max_total_bytes {
            self.truncate_active(self.config.max_total_bytes)?;
        }
        Ok(())
    }

    fn rotated_path(&self, index: usize) -> PathBuf {
        self.config
            .directory
            .join(format!("{LOG_PREFIX}.log.{index}"))
    }
}

fn owned_files(directory: &Path) -> Result<Vec<(usize, PathBuf, u64)>, LoggingError> {
    let mut result = Vec::new();
    for entry in
        fs::read_dir(directory).map_err(|error| io_error("logging:rotation_failed", error))?
    {
        let entry = entry.map_err(|error| io_error("logging:rotation_failed", error))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let index = if name == ACTIVE_FILE {
            Some(0)
        } else if let Some(raw) = name.strip_prefix("dji4g-panel.log.") {
            raw.parse::<usize>().ok().filter(|index| *index > 0)
        } else {
            None
        };
        if let Some(index) = index {
            let metadata = entry
                .metadata()
                .map_err(|error| io_error("logging:rotation_failed", error))?;
            if metadata.is_file() {
                result.push((index, path, metadata.len()));
            }
        }
    }
    Ok(result)
}

/// Replace sensitive fields before any log formatting.  Stable field names remain visible so a
/// diagnosis can still explain what was intentionally omitted.
#[must_use]
pub fn redact_sensitive(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for (token_index, token) in input.split_inclusive(char::is_whitespace).enumerate() {
        let trailing = token
            .chars()
            .last()
            .filter(|character| character.is_whitespace())
            .map_or("", |_| " ");
        let body = token.trim_end_matches(char::is_whitespace);
        let replacement = redact_token(body);
        output.push_str(&replacement);
        if !trailing.is_empty() {
            // Preserve exact whitespace except for a possible line break; this does not expose
            // any sensitive value and keeps multi-line diagnostics readable.
            output.push_str(&token[body.len()..]);
        }
        if token_index == usize::MAX {
            break;
        }
    }
    if output.is_empty() && !input.is_empty() {
        "<redacted>".to_owned()
    } else {
        output
    }
}

fn redact_token(token: &str) -> String {
    let Some((key, value)) = token.split_once('=') else {
        return if token.contains("AT+") || token.contains("+CG") || token.contains("+QCFG") {
            "<redacted>".to_owned()
        } else {
            token.to_owned()
        };
    };
    let sensitive = matches!(
        key.to_ascii_lowercase().as_str(),
        "imei"
            | "imsi"
            | "iccid"
            | "msisdn"
            | "phone"
            | "telephone"
            | "apn"
            | "ssid"
            | "containerid"
            | "container_id"
            | "instanceid"
            | "instance_id"
            | "device_instance_id"
            | "raw"
            | "raw_response"
            | "http_body"
            | "body"
            | "config"
            | "path"
    );
    if sensitive || value.contains("AT+") || value.contains("+CG") || value.contains("+QCFG") {
        format!("{key}=<redacted>")
    } else {
        token.to_owned()
    }
}

const fn level_byte(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

const fn level_from_byte(value: u8) -> LogLevel {
    match value {
        0 => LogLevel::Error,
        1 => LogLevel::Warn,
        3 => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}
