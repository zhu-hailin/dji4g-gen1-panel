//! A deliberately narrow, fail-closed Clash Verge Rev v2.5.5 adapter.
//!
//! The client applies subscription, global extension, script, subscription extension and script
//! in order.  Editing a generated runtime file is not durable; only one unambiguous extension
//! source is eligible.  No JavaScript is executed here.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use dji4g_domain::{ProxyBinding, ProxyClient, sha256};
use yaml_rust2::{
    Yaml, YamlLoader,
    parser::{Event, Parser},
};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const DEFAULT_SCRIPT: &str = "// Define main function (script entry)\n\nfunction main(config, profileName) {\n  return config;\n}\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClashError(pub &'static str);

pub struct ClashInspection {
    pub binding: Option<ProxyBinding>,
    pub candidate: Option<RepairCandidate>,
}

/// Never derive Debug: these fields include complete private client configuration bytes.
#[derive(Clone)]
pub struct RepairCandidate {
    pub path: PathBuf,
    pub root: PathBuf,
    pub alias: String,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub dependencies: Vec<(PathBuf, [u8; 32])>,
    pub executable: Option<PathBuf>,
}

pub struct BackupRecord {
    pub source_path: PathBuf,
    pub backup_path: PathBuf,
    pub original_hash: [u8; 32],
    pub applied_hash: [u8; 32],
}

fn document(source: &str) -> Result<Yaml, ClashError> {
    if source
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
    {
        return Ok(Yaml::Hash(Default::default()));
    }
    let mut parser = Parser::new_from_str(source);
    loop {
        let (event, _) = parser
            .next_token()
            .map_err(|_| ClashError("proxy:yaml_invalid"))?;
        match event {
            Event::Alias(_)
            | Event::Scalar(_, _, 1.., _)
            | Event::SequenceStart(1.., _)
            | Event::MappingStart(1.., _) => return Err(ClashError("proxy:yaml_complex")),
            Event::Scalar(_, _, _, Some(_))
            | Event::SequenceStart(_, Some(_))
            | Event::MappingStart(_, Some(_)) => return Err(ClashError("proxy:yaml_complex")),
            Event::StreamEnd => break,
            _ => {}
        }
    }
    let docs = YamlLoader::load_from_str(source).map_err(|_| ClashError("proxy:yaml_invalid"))?;
    if docs.len() != 1 || !matches!(docs[0], Yaml::Hash(_)) {
        return Err(ClashError("proxy:yaml_complex"));
    }
    Ok(docs.into_iter().next().expect("one document"))
}

fn read_limited(path: &Path) -> Result<(Vec<u8>, Yaml), ClashError> {
    let size = fs::metadata(path)
        .map_err(|_| ClashError("proxy:config_unreadable"))?
        .len();
    if size > MAX_CONFIG_BYTES {
        return Err(ClashError("proxy:config_too_large"));
    }
    let bytes = fs::read(path).map_err(|_| ClashError("proxy:config_unreadable"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ClashError("proxy:config_too_large"));
    }
    let source =
        std::str::from_utf8(&bytes).map_err(|_| ClashError("proxy:encoding_unsupported"))?;
    let parsed = document(source)?;
    Ok((bytes, parsed))
}

fn checked_file(root: &Path, file: &str) -> Result<PathBuf, ClashError> {
    if file.is_empty()
        || file == "."
        || file == ".."
        || file.len() > 255
        || file.contains(['/', '\\', ':'])
    {
        return Err(ClashError("proxy:path_unsafe"));
    }
    let profiles = root.join("profiles");
    reject_reparse(root)?;
    reject_reparse(&profiles)?;
    let path = profiles.join(file);
    reject_reparse(&path)?;
    let canonical_root = profiles
        .canonicalize()
        .map_err(|_| ClashError("proxy:path_unsafe"))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|_| ClashError("proxy:config_unreadable"))?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
        return Err(ClashError("proxy:path_unsafe"));
    }
    Ok(canonical_path)
}

fn reject_reparse(path: &Path) -> Result<(), ClashError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ClashError("proxy:config_unreadable"))?;
    if metadata.file_type().is_symlink() {
        return Err(ClashError("proxy:path_unsafe"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(ClashError("proxy:path_unsafe"));
        }
    }
    Ok(())
}

fn item<'a>(profiles: &'a Yaml, uid: &str) -> Result<&'a Yaml, ClashError> {
    optional_item(profiles, uid)?.ok_or(ClashError("proxy:profile_ambiguous"))
}

fn optional_item<'a>(profiles: &'a Yaml, uid: &str) -> Result<Option<&'a Yaml>, ClashError> {
    let Yaml::Array(items) = &profiles["items"] else {
        return Err(ClashError("proxy:profile_ambiguous"));
    };
    let matches = items
        .iter()
        .filter(|item| item["uid"].as_str() == Some(uid))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [item] => Ok(Some(item)),
        _ => Err(ClashError("proxy:profile_ambiguous")),
    }
}

fn referenced_file(root: &Path, item: &Yaml) -> Result<PathBuf, ClashError> {
    let file = item["file"]
        .as_str()
        .ok_or(ClashError("proxy:profile_ambiguous"))?;
    checked_file(root, file)
}

fn binding_alias(doc: &Yaml) -> Result<Option<String>, ClashError> {
    match &doc["interface-name"] {
        Yaml::BadValue => Ok(None),
        Yaml::String(value) if !value.trim().is_empty() && !value.contains(['\r', '\n']) => {
            Ok(Some(value.clone()))
        }
        _ => Err(ClashError("proxy:binding_ambiguous")),
    }
}

/// Return an edited copy with the exact source line commented out. All unrelated bytes,
/// comments and line endings remain untouched; semantic YAML comparison verifies the change.
pub fn remove_simple_binding(source: &[u8]) -> Result<(String, Vec<u8>), ClashError> {
    let text = std::str::from_utf8(source).map_err(|_| ClashError("proxy:encoding_unsupported"))?;
    let before = document(text)?;
    let alias = binding_alias(&before)?.ok_or(ClashError("proxy:binding_missing"))?;
    let mut offsets = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if let Some(value) = content.strip_prefix("interface-name:") {
            offsets.push(start);
            if value.trim().starts_with(['|', '>']) {
                return Err(ClashError("proxy:yaml_complex"));
            }
        }
        start += line.len();
    }
    if offsets.len() != 1 {
        return Err(ClashError("proxy:binding_ambiguous"));
    }
    let mut after = source.to_vec();
    after.splice(offsets[0]..offsets[0], b"# ".iter().copied());
    let parsed = document(
        std::str::from_utf8(&after).map_err(|_| ClashError("proxy:encoding_unsupported"))?,
    )?;
    let mut expected = before;
    if let Yaml::Hash(map) = &mut expected {
        map.remove(&Yaml::String("interface-name".into()));
    }
    if parsed != expected {
        return Err(ClashError("proxy:yaml_change_unverified"));
    }
    Ok((alias, after))
}

pub fn inspect_directory(
    root: &Path,
    version: Option<&str>,
) -> Result<ClashInspection, ClashError> {
    reject_reparse(root)?;
    let profiles_path = root.join("profiles.yaml");
    reject_reparse(&profiles_path)?;
    let (profiles_bytes, profiles) = read_limited(&profiles_path)?;
    let current_uid = profiles["current"]
        .as_str()
        .ok_or(ClashError("proxy:profile_ambiguous"))?;
    let current = item(&profiles, current_uid)?;
    let base_path = referenced_file(root, current)?;
    let (base_bytes, base) = read_limited(&base_path)?;

    let option_uid = |key: &str, default: &'static str| match &current["option"][key] {
        Yaml::BadValue => Ok(default),
        Yaml::String(value) if !value.is_empty() => Ok(value.as_str()),
        _ => Err(ClashError("proxy:profile_ambiguous")),
    };
    let merge_uid = option_uid("merge", "Merge")?;
    let script_uid = option_uid("script", "Script")?;
    let mut sources = vec![(base_path, base_bytes, base, false)];
    let global_merge = item(&profiles, "Merge")?;
    if global_merge["type"].as_str() != Some("merge") {
        return Err(ClashError("proxy:profile_ambiguous"));
    }
    let global_path = referenced_file(root, global_merge)?;
    let (bytes, doc) = read_limited(&global_path)?;
    sources.push((global_path, bytes, doc, true));
    if merge_uid != "Merge" {
        let merge = item(&profiles, merge_uid)?;
        if merge["type"].as_str() != Some("merge") {
            return Err(ClashError("proxy:profile_ambiguous"));
        }
        let path = referenced_file(root, merge)?;
        let (bytes, doc) = read_limited(&path)?;
        sources.push((path, bytes, doc, true));
    }
    let mut dependencies = vec![(profiles_path, sha256(&profiles_bytes))];
    for (path, bytes, _, _) in &sources {
        dependencies.push((path.clone(), sha256(bytes)));
    }
    let script_safe = script_uid == "Script"
        && match optional_item(&profiles, "Script")? {
            Some(script) if script["type"].as_str() == Some("script") => {
                let path = referenced_file(root, script)?;
                let bytes = fs::read(&path).map_err(|_| ClashError("proxy:config_unreadable"))?;
                if bytes.len() as u64 > MAX_CONFIG_BYTES {
                    return Err(ClashError("proxy:config_too_large"));
                }
                dependencies.push((path, sha256(&bytes)));
                normalize_newlines(&bytes) == DEFAULT_SCRIPT.as_bytes()
            }
            None => true, // the client's built-in default is an identity script
            _ => false,
        };
    let mut found = Vec::new();
    for (index, (_, _, doc, _)) in sources.iter().enumerate() {
        if let Some(alias) = binding_alias(doc)? {
            found.push((index, alias));
        }
    }
    let Some((index, alias)) = found.last() else {
        return Ok(ClashInspection {
            binding: None,
            candidate: None,
        });
    };
    let supported =
        version == Some("2.5.5") && script_safe && found.len() == 1 && sources[*index].3;
    let candidate = if supported {
        let (path, before, _, _) = &sources[*index];
        let (parsed_alias, after) = remove_simple_binding(before)?;
        if &parsed_alias != alias {
            return Err(ClashError("proxy:binding_ambiguous"));
        }
        Some(RepairCandidate {
            path: path.clone(),
            root: root.to_path_buf(),
            alias: alias.clone(),
            before: before.clone(),
            after,
            dependencies,
            executable: None,
        })
    } else {
        None
    };
    Ok(ClashInspection {
        binding: Some(ProxyBinding {
            client: ProxyClient::ClashVergeRev,
            version: version.map(str::to_owned),
            interface_alias: alias.clone(),
            repairable: candidate.is_some(),
        }),
        candidate,
    })
}

fn normalize_newlines(value: &[u8]) -> Vec<u8> {
    value
        .iter()
        .copied()
        .filter(|byte| *byte != b'\r')
        .collect()
}

pub fn verify_candidate(candidate: &RepairCandidate) -> Result<(), ClashError> {
    reject_reparse(&candidate.root)?;
    reject_reparse(&candidate.root.join("profiles"))?;
    let canonical_root = candidate
        .root
        .canonicalize()
        .map_err(|_| ClashError("proxy:path_unsafe"))?;
    for (path, digest) in &candidate.dependencies {
        reject_reparse(path)?;
        let canonical = path
            .canonicalize()
            .map_err(|_| ClashError("proxy:config_changed"))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(ClashError("proxy:path_unsafe"));
        }
        let bytes = fs::read(path).map_err(|_| ClashError("proxy:config_changed"))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES || sha256(&bytes) != *digest {
            return Err(ClashError("proxy:config_changed"));
        }
    }
    if fs::read(&candidate.path).map_err(|_| ClashError("proxy:config_changed"))?
        != candidate.before
    {
        return Err(ClashError("proxy:config_changed"));
    }
    Ok(())
}

fn write_new(path: &Path, content: &[u8]) -> Result<(), ClashError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| ClashError("proxy:write_failed"))?;
    file.write_all(content)
        .map_err(|_| ClashError("proxy:write_failed"))?;
    file.sync_all()
        .map_err(|_| ClashError("proxy:write_failed"))?;
    Ok(())
}

fn nonce() -> Result<u128, ClashError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|age| age.as_nanos())
        .map_err(|_| ClashError("proxy:clock_invalid"))
}

pub fn apply_candidate(candidate: &RepairCandidate) -> Result<BackupRecord, ClashError> {
    verify_candidate(candidate)?;
    let backup_dir = candidate.root.join(".dji4g-repair-backups");
    fs::create_dir_all(&backup_dir).map_err(|_| ClashError("proxy:backup_failed"))?;
    reject_reparse(&backup_dir)?;
    let serial = nonce()?;
    let backup_path = backup_dir.join(format!("repair-{}-{serial}.yaml.bak", std::process::id()));
    write_new(&backup_path, &candidate.before).map_err(|_| ClashError("proxy:backup_failed"))?;
    if sha256(&fs::read(&backup_path).map_err(|_| ClashError("proxy:backup_failed"))?)
        != sha256(&candidate.before)
    {
        return Err(ClashError("proxy:backup_failed"));
    }
    // Recheck after the backup, immediately before replacing the original.
    verify_candidate(candidate)?;
    let temporary = candidate.path.with_extension(format!("dji4g-{serial}.tmp"));
    write_new(&temporary, &candidate.after)?;
    let replaced = crate::atomic_replace_file(&temporary, &candidate.path);
    if replaced.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    let readback = fs::read(&candidate.path).map_err(|_| ClashError("proxy:outcome_unknown"))?;
    if readback != candidate.after {
        return Err(if readback == candidate.before {
            ClashError("proxy:replace_failed")
        } else {
            ClashError("proxy:outcome_unknown")
        });
    }
    Ok(BackupRecord {
        source_path: candidate.path.clone(),
        backup_path,
        original_hash: sha256(&candidate.before),
        applied_hash: sha256(&candidate.after),
    })
}

pub fn restore_candidate(record: &BackupRecord) -> Result<(), ClashError> {
    reject_reparse(&record.source_path)?;
    reject_reparse(&record.backup_path)?;
    let current =
        fs::read(&record.source_path).map_err(|_| ClashError("proxy:restore_conflict"))?;
    if sha256(&current) != record.applied_hash {
        return Err(ClashError("proxy:restore_conflict"));
    }
    let original =
        fs::read(&record.backup_path).map_err(|_| ClashError("proxy:backup_unavailable"))?;
    if sha256(&original) != record.original_hash {
        return Err(ClashError("proxy:backup_changed"));
    }
    let temporary = record
        .source_path
        .with_extension(format!("dji4g-restore-{}.tmp", nonce()?));
    write_new(&temporary, &original)?;
    let replaced = crate::atomic_replace_file(&temporary, &record.source_path);
    if replaced.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    let readback =
        fs::read(&record.source_path).map_err(|_| ClashError("proxy:outcome_unknown"))?;
    if readback != original {
        return Err(if readback == current {
            ClashError("proxy:restore_failed")
        } else {
            ClashError("proxy:outcome_unknown")
        });
    }
    Ok(())
}

/// Observe known client/core process names without executing or stopping any process.
#[cfg(windows)]
pub fn running_processes() -> Result<Vec<(String, Option<PathBuf>)>, ClashError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(ClashError("proxy:process_inventory_failed"));
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    let mut out = Vec::new();
    let mut has = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has {
        let end = entry
            .szExeFile
            .iter()
            .position(|part| *part == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "clash-verge.exe" | "verge-mihomo.exe" | "clash-verge-service.exe"
        ) {
            let process =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
            let path = if process.is_null() {
                None
            } else {
                let mut buffer = vec![0_u16; 32768];
                let mut size = buffer.len() as u32;
                let result = unsafe {
                    QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size)
                };
                unsafe { CloseHandle(process) };
                if result == 0 {
                    None
                } else {
                    use std::os::windows::ffi::OsStringExt;
                    Some(PathBuf::from(std::ffi::OsString::from_wide(
                        &buffer[..size as usize],
                    )))
                }
            };
            out.push((name, path));
        }
        has = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    Ok(out)
}

#[cfg(not(windows))]
pub fn running_processes() -> Result<Vec<(String, Option<PathBuf>)>, ClashError> {
    Ok(Vec::new())
}

#[cfg(windows)]
pub fn executable_version(path: &Path) -> Result<String, ClashError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut()) };
    if size < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32 || size > 1024 * 1024 {
        return Err(ClashError("proxy:client_version_unknown"));
    }
    let mut bytes = vec![0_u8; size as usize];
    if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, bytes.as_mut_ptr().cast()) } == 0 {
        return Err(ClashError("proxy:client_version_unknown"));
    }
    let mut pointer = std::ptr::null_mut();
    let mut length = 0_u32;
    if unsafe {
        VerQueryValueW(
            bytes.as_ptr().cast(),
            [b'\\' as u16, 0].as_ptr(),
            &mut pointer,
            &mut length,
        )
    } == 0
        || pointer.is_null()
        || length < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
    {
        return Err(ClashError("proxy:client_version_unknown"));
    }
    let info = unsafe { &*pointer.cast::<VS_FIXEDFILEINFO>() };
    if info.dwSignature != 0xFEEF04BD {
        return Err(ClashError("proxy:client_version_unknown"));
    }
    Ok(format!(
        "{}.{}.{}",
        info.dwFileVersionMS >> 16,
        info.dwFileVersionMS & 0xffff,
        info.dwFileVersionLS >> 16
    ))
}

#[cfg(not(windows))]
pub fn executable_version(_path: &Path) -> Result<String, ClashError> {
    Err(ClashError("proxy:client_version_unknown"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dji4g-proxy-parser-{}-{}",
            std::process::id(),
            nonce().unwrap()
        ));
        let profiles_dir = root.join("profiles");
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(root.join("profiles.yaml"), "current: active\nitems:\n  - uid: active\n    type: remote\n    file: active.yaml\n  - uid: Merge\n    type: merge\n    file: global.yaml\n").unwrap();
        fs::write(profiles_dir.join("active.yaml"), "profile: test\n").unwrap();
        fs::write(
            profiles_dir.join("global.yaml"),
            "interface-name: missing-port\n",
        )
        .unwrap();
        root
    }

    fn remove_fixture(root: &Path) {
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        let actual = root.canonicalize().unwrap();
        assert!(actual.starts_with(&temporary));
        assert!(
            actual
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dji4g-proxy-parser-")
        );
        fs::remove_dir_all(actual).unwrap();
    }

    #[test]
    fn edits_only_the_unique_top_level_key_and_preserves_other_bytes() {
        let source = b"# user's comment\r\ninterface-name: \"Ethernet 5\" # chosen outlet\r\nproxies:\r\n  - name: interface-name\r\n";
        let (alias, after) = remove_simple_binding(source).unwrap();
        assert_eq!(alias, "Ethernet 5");
        assert_eq!(&after[..], b"# user's comment\r\n# interface-name: \"Ethernet 5\" # chosen outlet\r\nproxies:\r\n  - name: interface-name\r\n");
    }

    #[test]
    fn rejects_duplicate_key_anchor_and_multidocument() {
        assert!(remove_simple_binding(b"interface-name: x\ninterface-name: y\n").is_err());
        assert!(remove_simple_binding(b"interface-name: &out x\n").is_err());
        assert!(remove_simple_binding(b"interface-name: x\n---\nfoo: bar\n").is_err());
    }

    #[test]
    fn extension_is_repairable_only_when_it_is_the_sole_binding() {
        let root = fixture_root();
        let profiles_dir = root.join("profiles");
        let inspection = inspect_directory(&root, Some("2.5.5")).unwrap();
        assert!(inspection.binding.unwrap().repairable);
        assert!(inspection.candidate.is_some());
        fs::write(
            profiles_dir.join("active.yaml"),
            "interface-name: old-port\n",
        )
        .unwrap();
        let inspection = inspect_directory(&root, Some("2.5.5")).unwrap();
        assert!(!inspection.binding.unwrap().repairable);
        assert!(inspection.candidate.is_none());
        remove_fixture(&root);
    }

    #[test]
    fn ambiguous_default_script_disables_automatic_editing() {
        let root = fixture_root();
        let profiles = root.join("profiles.yaml");
        let mut content = fs::read_to_string(&profiles).unwrap();
        content.push_str("  - uid: Script\n    type: script\n    file: a.js\n  - uid: Script\n    type: script\n    file: b.js\n");
        fs::write(profiles, content).unwrap();
        assert_eq!(
            inspect_directory(&root, Some("2.5.5")).err(),
            Some(ClashError("proxy:profile_ambiguous"))
        );
        remove_fixture(&root);
    }

    #[test]
    fn backup_apply_and_restore_verify_file_identity() {
        let root = fixture_root();
        let candidate = inspect_directory(&root, Some("2.5.5"))
            .unwrap()
            .candidate
            .unwrap();
        let record = apply_candidate(&candidate).unwrap();
        assert_eq!(fs::read(&record.source_path).unwrap(), candidate.after);
        assert_eq!(fs::read(&record.backup_path).unwrap(), candidate.before);
        assert_eq!(
            verify_candidate(&candidate).err(),
            Some(ClashError("proxy:config_changed"))
        );
        restore_candidate(&record).unwrap();
        assert_eq!(fs::read(&record.source_path).unwrap(), candidate.before);
        remove_fixture(&root);
    }

    #[test]
    fn external_change_blocks_restore_without_overwriting_it() {
        let root = fixture_root();
        let candidate = inspect_directory(&root, Some("2.5.5"))
            .unwrap()
            .candidate
            .unwrap();
        let record = apply_candidate(&candidate).unwrap();
        fs::write(&record.source_path, b"custom: later edit\n").unwrap();
        assert_eq!(
            restore_candidate(&record).err(),
            Some(ClashError("proxy:restore_conflict"))
        );
        assert_eq!(
            fs::read(&record.source_path).unwrap(),
            b"custom: later edit\n"
        );
        remove_fixture(&root);
    }
}
