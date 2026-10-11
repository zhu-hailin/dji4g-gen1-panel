//! Stable GitHub release selection, strict integrity boundaries and worker service.
mod service;
mod worker;
use semver::Version;
use serde::Deserialize;
pub use service::{UpdateCheckStatus, UpdateService, UpdateState};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const REPOSITORY: &str = "zhu-hailin/dji4g-gen1-panel";
pub const PORTABLE_ASSET: &str = "dji4g-panel-windows-x64-portable.zip";
pub const MAX_ARCHIVE_SIZE: u64 = 512 * 1024 * 1024;
pub(crate) const CHECKSUM_LIMIT: u64 = 4096;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub version: Version,
    pub tag: String,
    pub download_url: String,
    pub checksum_url: String,
    pub size: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateError {
    Release,
    Assets,
    Source,
    Checksum,
    Network,
    Download,
    Integrity,
    Updater,
    Unsupported,
    Startup,
}
impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "update:{self:?}")
    }
}
impl std::error::Error for UpdateError {}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Accept only the exact versioned portable assets from the fixed upstream repository.
pub fn candidate_from_json(json: &[u8], current: &str) -> Result<Option<Candidate>, UpdateError> {
    let release: Release = serde_json::from_slice(json).map_err(|_| UpdateError::Release)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|_| UpdateError::Release)?;
    let current = Version::parse(current.strip_prefix('v').unwrap_or(current))
        .map_err(|_| UpdateError::Release)?;
    if !version.pre.is_empty() || version.cmp_precedence(&current).is_le() {
        return Ok(None);
    }
    let asset = exact_asset(&release.assets, PORTABLE_ASSET)?;
    let sidecar_name = format!("{PORTABLE_ASSET}.sha256");
    let sidecar = exact_asset(&release.assets, &sidecar_name)?;
    if asset.size == 0
        || asset.size > MAX_ARCHIVE_SIZE
        || sidecar.size == 0
        || sidecar.size > CHECKSUM_LIMIT
    {
        return Err(UpdateError::Assets);
    }
    let base = format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/",
        release.tag_name
    );
    if asset.browser_download_url != format!("{base}{PORTABLE_ASSET}")
        || sidecar.browser_download_url != format!("{base}{sidecar_name}")
    {
        return Err(UpdateError::Source);
    }
    Ok(Some(Candidate {
        version,
        tag: release.tag_name,
        download_url: asset.browser_download_url.clone(),
        checksum_url: sidecar.browser_download_url.clone(),
        size: asset.size,
    }))
}
fn exact_asset<'a>(assets: &'a [Asset], name: &str) -> Result<&'a Asset, UpdateError> {
    let mut matching = assets.iter().filter(|asset| asset.name == name);
    let asset = matching.next().ok_or(UpdateError::Assets)?;
    if matching.next().is_some() {
        return Err(UpdateError::Assets);
    }
    Ok(asset)
}
/// Require the single GNU sha256sum entry naming our exact archive. No hash guessing.
pub fn parse_checksum(bytes: &[u8]) -> Result<String, UpdateError> {
    if bytes.len() as u64 > CHECKSUM_LIMIT {
        return Err(UpdateError::Checksum);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| UpdateError::Checksum)?
        .trim_start_matches('\u{feff}');
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != 2
        || fields[0].len() != 64
        || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
        || fields[1].strip_prefix('*').unwrap_or(fields[1]) != PORTABLE_ASSET
    {
        return Err(UpdateError::Checksum);
    }
    Ok(fields[0].to_ascii_lowercase())
}
/// Stream bounded bytes without ever treating truncation or an I/O error as completion.
pub fn write_download(
    mut reader: impl Read,
    mut writer: impl Write,
    size: u64,
    mut progress: impl FnMut(u64),
) -> Result<(), UpdateError> {
    if size == 0 || size > MAX_ARCHIVE_SIZE {
        return Err(UpdateError::Download);
    }
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| UpdateError::Download)?;
        if count == 0 {
            break;
        }
        downloaded += count as u64;
        if downloaded > size {
            return Err(UpdateError::Download);
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|_| UpdateError::Download)?;
        progress(downloaded);
    }
    if downloaded != size {
        return Err(UpdateError::Download);
    }
    writer.flush().map_err(|_| UpdateError::Download)
}
pub fn validate_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
/// Consume the updater-only option before the existing closed startup parser runs.
pub fn split_startup_marker(
    args: Vec<String>,
) -> Result<(Vec<String>, Option<PathBuf>), UpdateError> {
    let mut remaining = Vec::new();
    let mut marker = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--update-startup-marker" {
            if marker.is_some() {
                return Err(UpdateError::Startup);
            }
            let path = args
                .next()
                .filter(|p| !p.is_empty() && !p.starts_with("--"))
                .ok_or(UpdateError::Startup)?;
            marker = Some(PathBuf::from(path));
        } else {
            remaining.push(arg);
        }
    }
    Ok((remaining, marker))
}
fn startup_session(marker: &Path) -> Result<PathBuf, UpdateError> {
    if marker.file_name().and_then(|p| p.to_str()) != Some("startup-ok") {
        return Err(UpdateError::Startup);
    }
    let session = marker.parent().ok_or(UpdateError::Startup)?;
    let metadata = std::fs::symlink_metadata(session).map_err(|_| UpdateError::Startup)?;
    if !metadata.is_dir() || linked(&metadata) {
        return Err(UpdateError::Startup);
    }
    let session = session.canonicalize().map_err(|_| UpdateError::Startup)?;
    if session.parent()
        != Some(
            std::env::temp_dir()
                .canonicalize()
                .map_err(|_| UpdateError::Startup)?
                .as_path(),
        )
        || !session
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with("dji4g-update-"))
    {
        return Err(UpdateError::Startup);
    }
    let ready =
        std::fs::symlink_metadata(session.join("ready")).map_err(|_| UpdateError::Startup)?;
    if !ready.is_file() || linked(&ready) {
        return Err(UpdateError::Startup);
    }
    Ok(session)
}
pub(crate) fn linked(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}
pub fn write_startup_ack(marker: &Path) -> Result<(), UpdateError> {
    let session = startup_session(marker)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(session.join("startup-ok"))
        .map_err(|_| UpdateError::Startup)?;
    file.write_all(b"ok")
        .and_then(|()| file.sync_all())
        .map_err(|_| UpdateError::Startup)
}
/// A startup command-line path authorizes only acknowledgement. It never authorizes
/// deleting a directory: successful detached sessions are retained for diagnosis.
pub(crate) fn acknowledge_startup(marker: PathBuf) {
    let _ = write_startup_ack(&marker);
}
#[cfg(test)]
pub(crate) enum TestEvent {
    Ready,
    Failed,
}
#[cfg(test)]
pub(crate) use service::test_driver;
#[cfg(test)]
mod startup_safety_tests {
    use super::*;
    #[test]
    fn startup_argument_never_deletes_other_files_in_a_forged_session() {
        let session = tempfile::Builder::new()
            .prefix("dji4g-update-")
            .tempdir()
            .unwrap();
        let note = session.path().join("user-notes.txt");
        std::fs::write(&note, b"preserve").unwrap();
        std::fs::write(session.path().join("ready"), b"ready").unwrap();
        std::fs::write(session.path().join("done"), b"done").unwrap();
        acknowledge_startup(session.path().join("startup-ok"));
        assert_eq!(std::fs::read(note).unwrap(), b"preserve");
    }
}
