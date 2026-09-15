//! Versioned, privacy-neutral panel configuration.
//!
//! The configuration boundary deliberately contains no device or backend values.  A V1 document
//! is decoded into the application's closed enums, written to a sibling temporary file, flushed
//! and synchronised, and only then replaced in one platform atomic operation.

use std::{
    ffi::OsStr,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use dji4g_application::{
    AutostartKnownState, AutostartStatus, LanguageCode, LogLevel, SettingsSnapshot,
};
use serde::{Deserialize, Serialize};

const APPLICATION_DIRECTORY: &str = "Dji4GPanel";
const CONFIG_FILE_NAME: &str = "config.toml";
static NAME_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The only user-controlled configuration values persisted by the panel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigV1 {
    pub language: LanguageCode,
    pub autostart: bool,
    pub start_minimized: bool,
    pub active_probe: bool,
    pub log_level: LogLevel,
}

impl Default for ConfigV1 {
    fn default() -> Self {
        Self {
            language: LanguageCode::ZhCn,
            autostart: false,
            start_minimized: false,
            active_probe: true,
            log_level: LogLevel::Info,
        }
    }
}

impl ConfigV1 {
    /// Derive the document to persist from the settings snapshot the panel currently holds.
    ///
    /// The autostart intent is only taken from states that carry it explicitly: a pending write
    /// (`Saving`) or a confirmed registry observation (`Ready(Enabled/Disabled)`). `Loading`,
    /// `Ready(Drift)`, and `Failed` do not, and guessing a bool there could silently overwrite the
    /// stored intent and hide real registry drift, so `None` means "nothing to write yet".
    #[must_use]
    pub fn from_settings(settings: &SettingsSnapshot) -> Option<Self> {
        let autostart = match &settings.autostart {
            AutostartStatus::Saving {
                desired_enabled, ..
            } => Some(*desired_enabled),
            AutostartStatus::Ready(AutostartKnownState::Enabled) => Some(true),
            AutostartStatus::Ready(AutostartKnownState::Disabled) => Some(false),
            AutostartStatus::Loading
            | AutostartStatus::Ready(AutostartKnownState::Drift)
            | AutostartStatus::Failed { .. } => None,
        }?;
        Some(Self {
            language: settings.language,
            autostart,
            start_minimized: settings.start_minimized,
            active_probe: settings.active_probe,
            log_level: settings.log_level,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigPaths {
    pub config_file: PathBuf,
    pub log_dir: PathBuf,
    pub exports_dir: PathBuf,
}

impl ConfigPaths {
    /// Resolve only the two documented Windows profile variables.
    ///
    /// Falling back to the current directory or a temporary directory would make a successful
    /// save disappear on the next launch, so missing variables are a stable error instead.
    pub fn from_environment() -> Result<Self, ConfigError> {
        Self::from_environment_values(
            std::env::var_os("APPDATA")
                .as_deref()
                .and_then(OsStr::to_str),
            std::env::var_os("LOCALAPPDATA")
                .as_deref()
                .and_then(OsStr::to_str),
        )
    }

    pub fn from_environment_values(
        appdata: Option<&str>,
        local_appdata: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let appdata = appdata
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| ConfigError::new("config:path_unavailable"))?;
        let local_appdata = local_appdata
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| ConfigError::new("config:path_unavailable"))?;
        Ok(Self {
            config_file: appdata.join(APPLICATION_DIRECTORY).join(CONFIG_FILE_NAME),
            log_dir: local_appdata.join(APPLICATION_DIRECTORY).join("logs"),
            exports_dir: local_appdata.join(APPLICATION_DIRECTORY).join("exports"),
        })
    }

    /// A deterministic test seam that still uses the production directory layout.
    #[cfg(test)]
    pub fn under(root: &Path) -> Self {
        Self::under_root(root)
    }

    /// Public equivalent of [`Self::under`] for integration tests.
    pub fn under_root(root: &Path) -> Self {
        Self {
            config_file: root
                .join("roaming")
                .join(APPLICATION_DIRECTORY)
                .join(CONFIG_FILE_NAME),
            log_dir: root.join("local").join(APPLICATION_DIRECTORY).join("logs"),
            exports_dir: root
                .join("local")
                .join(APPLICATION_DIRECTORY)
                .join("exports"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    stable_code: &'static str,
    os_code: Option<u32>,
}

impl ConfigError {
    #[must_use]
    pub const fn new(stable_code: &'static str) -> Self {
        Self {
            stable_code,
            os_code: None,
        }
    }

    #[must_use]
    pub const fn with_os_code(stable_code: &'static str, os_code: u32) -> Self {
        Self {
            stable_code,
            os_code: Some(os_code),
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

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_code)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigLoadOutcome {
    Missing { config: ConfigV1 },
    Loaded { config: ConfigV1 },
    CorruptPreserved { config: ConfigV1, backup: PathBuf },
}

#[derive(Debug)]
pub struct IoFailure {
    pub kind: io::ErrorKind,
    pub os_code: Option<u32>,
}

impl From<io::Error> for IoFailure {
    fn from(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            os_code: error
                .raw_os_error()
                .and_then(|value| u32::try_from(value).ok()),
        }
    }
}

pub trait TempFile {
    fn path(&self) -> &Path;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoFailure>;
    fn flush(&mut self) -> Result<(), IoFailure>;
    fn sync_all(&mut self) -> Result<(), IoFailure>;
}

pub trait FileOps: Clone + Send + Sync + 'static {
    fn read(&self, path: &Path) -> Result<Vec<u8>, IoFailure>;
    fn create_dir_all(&self, path: &Path) -> Result<(), IoFailure>;
    fn create_temp_new(&self, path: &Path) -> Result<Box<dyn TempFile>, IoFailure>;
    fn atomic_replace(&self, temporary: &Path, destination: &Path) -> Result<(), IoFailure>;
    fn preserve_corrupt(&self, source: &Path, backup: &Path) -> Result<(), IoFailure>;
    fn remove_file(&self, path: &Path) -> Result<(), IoFailure>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdFileOps;

struct StdTempFile {
    path: PathBuf,
    file: File,
}

impl TempFile for StdTempFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoFailure> {
        self.file.write_all(bytes).map_err(IoFailure::from)
    }

    fn flush(&mut self) -> Result<(), IoFailure> {
        self.file.flush().map_err(IoFailure::from)
    }

    fn sync_all(&mut self) -> Result<(), IoFailure> {
        self.file.sync_all().map_err(IoFailure::from)
    }
}

impl FileOps for StdFileOps {
    fn read(&self, path: &Path) -> Result<Vec<u8>, IoFailure> {
        fs::read(path).map_err(IoFailure::from)
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), IoFailure> {
        fs::create_dir_all(path).map_err(IoFailure::from)
    }

    fn create_temp_new(&self, path: &Path) -> Result<Box<dyn TempFile>, IoFailure> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(IoFailure::from)?;
        Ok(Box::new(StdTempFile {
            path: path.to_path_buf(),
            file,
        }))
    }

    fn atomic_replace(&self, temporary: &Path, destination: &Path) -> Result<(), IoFailure> {
        dji4g_windows_platform::atomic_replace_file(temporary, destination).map_err(|error| {
            IoFailure {
                kind: io::ErrorKind::Other,
                os_code: error.os_code,
            }
        })
    }

    fn preserve_corrupt(&self, source: &Path, backup: &Path) -> Result<(), IoFailure> {
        fs::rename(source, backup).map_err(IoFailure::from)
    }

    fn remove_file(&self, path: &Path) -> Result<(), IoFailure> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(IoFailure::from(error)),
        }
    }
}

pub trait Clock: Clone + Send + Sync + 'static {
    fn utc_now(&self) -> SystemTime;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn utc_now(&self) -> SystemTime {
        SystemTime::now()
    }
}

pub struct ConfigStore<F = StdFileOps, C = SystemClock> {
    paths: ConfigPaths,
    file_ops: F,
    clock: C,
}

impl ConfigStore<StdFileOps, SystemClock> {
    #[must_use]
    pub fn new(paths: ConfigPaths) -> Self {
        Self {
            paths,
            file_ops: StdFileOps,
            clock: SystemClock,
        }
    }
}

impl<F: FileOps, C: Clock> ConfigStore<F, C> {
    #[must_use]
    pub fn with_ops(paths: ConfigPaths, file_ops: F, clock: C) -> Self {
        Self {
            paths,
            file_ops,
            clock,
        }
    }

    #[must_use]
    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    pub fn load(&self) -> Result<ConfigLoadOutcome, ConfigError> {
        let bytes = match self.file_ops.read(&self.paths.config_file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind == io::ErrorKind::NotFound => {
                return Ok(ConfigLoadOutcome::Missing {
                    config: ConfigV1::default(),
                });
            }
            Err(error) => return Err(io_error("config:read_failed", error)),
        };

        let config = match decode_config(&bytes) {
            Ok(config) => config,
            Err(_reason) => return self.preserve_and_restore(bytes),
        };
        Ok(ConfigLoadOutcome::Loaded { config })
    }

    pub fn save(&self, config: &ConfigV1) -> Result<(), ConfigError> {
        let encoded = encode_config(config)?;
        self.save_bytes(encoded.as_bytes())?;
        let readback = self
            .file_ops
            .read(&self.paths.config_file)
            .map_err(|error| io_error("config:readback_failed", error))?;
        let readback_config =
            decode_config(&readback).map_err(|_| ConfigError::new("config:readback_failed"))?;
        if &readback_config != config {
            return Err(ConfigError::new("config:readback_failed"));
        }
        Ok(())
    }

    fn save_bytes(&self, bytes: &[u8]) -> Result<(), ConfigError> {
        let parent = self
            .paths
            .config_file
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| ConfigError::new("config:path_unavailable"))?;
        self.file_ops
            .create_dir_all(parent)
            .map_err(|error| io_error("config:directory_create_failed", error))?;

        let temporary = self.next_temporary_path();
        let mut temp = self
            .file_ops
            .create_temp_new(&temporary)
            .map_err(|error| io_error("config:temp_create_failed", error))?;
        let temporary_path = temp.path().to_path_buf();
        let write_result = temp
            .write_all(bytes)
            .map_err(|error| io_error("config:write_failed", error))
            .and_then(|()| {
                temp.flush()
                    .map_err(|error| io_error("config:flush_failed", error))
            })
            .and_then(|()| {
                temp.sync_all()
                    .map_err(|error| io_error("config:sync_failed", error))
            });
        // Close the temporary file before asking Windows to replace the destination.  The Rust
        // file handle's default sharing flags do not promise delete/share access, and keeping it
        // open can make ReplaceFileW fail even though all bytes were flushed and synced.
        drop(temp);
        let replace_result = write_result.and_then(|()| {
            self.file_ops
                .atomic_replace(&temporary_path, &self.paths.config_file)
                .map_err(|error| io_error("config:replace_failed", error))
        });
        if let Err(error) = replace_result {
            let _ = self.file_ops.remove_file(&temporary);
            return Err(error);
        }
        Ok(())
    }

    fn preserve_and_restore(&self, bytes: Vec<u8>) -> Result<ConfigLoadOutcome, ConfigError> {
        let backup = self.next_backup_path();
        self.file_ops
            .preserve_corrupt(&self.paths.config_file, &backup)
            .map_err(|error| io_error("config:preserve_failed", error))?;

        let defaults = ConfigV1::default();
        if let Err(error) = self.save(&defaults) {
            // The corrupt bytes are still available at `backup`; report the failed canonical
            // restore rather than claiming a clean save.  The caller can safely use defaults in
            // memory while surfacing this stable code in SettingsPersistenceState.
            let _ = bytes;
            return Err(ConfigError::new(match error.stable_code() {
                "config:directory_create_failed" => "config:default_restore_failed",
                "config:temp_create_failed" => "config:default_restore_failed",
                "config:write_failed" => "config:default_restore_failed",
                "config:flush_failed" => "config:default_restore_failed",
                "config:sync_failed" => "config:default_restore_failed",
                "config:replace_failed" => "config:default_restore_failed",
                _ => "config:default_restore_failed",
            }));
        }
        Ok(ConfigLoadOutcome::CorruptPreserved {
            config: defaults,
            backup,
        })
    }

    fn next_temporary_path(&self) -> PathBuf {
        let parent = self
            .paths
            .config_file
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let counter = NAME_COUNTER.fetch_add(1, Ordering::Relaxed);
        parent.join(format!(
            ".{CONFIG_FILE_NAME}.tmp.{}.{}",
            std::process::id(),
            counter
        ))
    }

    fn next_backup_path(&self) -> PathBuf {
        let parent = self
            .paths
            .config_file
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let seconds = self
            .clock
            .utc_now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let counter = NAME_COUNTER.fetch_add(1, Ordering::Relaxed);
        parent.join(format!(
            "{CONFIG_FILE_NAME}.corrupt-{seconds}-{}-{counter}.bak",
            std::process::id()
        ))
    }

    pub fn encode(config: &ConfigV1) -> Result<String, ConfigError> {
        encode_config(config)
    }

    pub fn decode(bytes: &[u8]) -> Result<ConfigV1, ConfigError> {
        decode_config(bytes).map_err(ConfigError::new)
    }
}

fn io_error(code: &'static str, error: IoFailure) -> ConfigError {
    ConfigError {
        stable_code: code,
        os_code: error.os_code,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigDocument {
    schema_version: u32,
    language: String,
    autostart: bool,
    start_minimized: bool,
    active_probe: bool,
    log_level: String,
}

fn encode_config(config: &ConfigV1) -> Result<String, ConfigError> {
    let document = ConfigDocument {
        schema_version: 1,
        language: match config.language {
            LanguageCode::ZhCn => "zh-CN".to_owned(),
            LanguageCode::EnUs => "en-US".to_owned(),
        },
        autostart: config.autostart,
        start_minimized: config.start_minimized,
        active_probe: config.active_probe,
        log_level: match config.log_level {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
        .to_owned(),
    };
    toml::to_string(&document).map_err(|_| ConfigError::new("config:encode_failed"))
}

fn decode_config(bytes: &[u8]) -> Result<ConfigV1, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "config:parse_failed")?;
    let document: ConfigDocument = toml::from_str(text).map_err(|error| {
        if error.message().contains("schema_version") {
            "config:unsupported_version"
        } else {
            "config:parse_failed"
        }
    })?;
    if document.schema_version != 1 {
        return Err("config:unsupported_version");
    }
    let language = match document.language.as_str() {
        "zh-CN" => LanguageCode::ZhCn,
        "en-US" => LanguageCode::EnUs,
        _ => return Err("config:parse_failed"),
    };
    let log_level = match document.log_level.as_str() {
        "error" => LogLevel::Error,
        "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        "debug" => LogLevel::Debug,
        _ => return Err("config:parse_failed"),
    };
    Ok(ConfigV1 {
        language,
        autostart: document.autostart,
        start_minimized: document.start_minimized,
        active_probe: document.active_probe,
        log_level,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartupOptions {
    pub autostart: bool,
    pub demo: Option<String>,
}

impl StartupOptions {
    pub fn parse<I, S>(args: I) -> Result<Self, StartupParseError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let values = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let mut result = Self::default();
        let mut index = 0;
        while index < values.len() {
            match values[index].as_str() {
                "--autostart" => result.autostart = true,
                "--demo" => {
                    index += 1;
                    let Some(value) = values.get(index) else {
                        return Err(StartupParseError::new("config:startup_missing_value"));
                    };
                    result.demo = Some(value.clone());
                }
                value if value.starts_with('-') => {
                    return Err(StartupParseError::new("config:startup_unknown_argument"));
                }
                _ => return Err(StartupParseError::new("config:startup_unknown_argument")),
            }
            index += 1;
        }
        Ok(result)
    }

    #[must_use]
    pub const fn start_to_tray(&self, configured_start_minimized: bool) -> bool {
        self.autostart || configured_start_minimized
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupParseError {
    stable_code: &'static str,
}

impl StartupParseError {
    #[must_use]
    pub const fn new(stable_code: &'static str) -> Self {
        Self { stable_code }
    }

    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        self.stable_code
    }
}

impl fmt::Display for StartupParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_code)
    }
}

impl std::error::Error for StartupParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_utf8_is_not_a_default_success() {
        assert_eq!(decode_config(&[0xff]), Err("config:parse_failed"));
    }

    #[test]
    fn document_rejects_unknown_fields_and_missing_fields() {
        let unknown = br#"schema_version = 1
language = "zh-CN"
autostart = false
start_minimized = false
active_probe = true
log_level = "info"
extra = true
"#;
        assert!(decode_config(unknown).is_err());
        assert!(decode_config(br#"schema_version = 1"#).is_err());
    }
}
