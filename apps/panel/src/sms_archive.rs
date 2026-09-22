//! Opt-in, current-user encrypted history. Only explicitly observed incoming display records
//! are captured. Retention is 90 days from capture and at most 5,000 rows, oldest removed first.
//! Archived rows deliberately cannot be converted to a live message or deletion key.

use dji4g_application::ControllerSnapshot;
use dji4g_domain::{SmsDirection, SmsDisplayMessage, SmsStatus, sha256};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_ARCHIVED_MESSAGES: usize = 5_000;
pub const RETENTION_DAYS: u64 = 90;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAGIC: &[u8] = b"DJI4G-SMS-DPAPI-1\0";

#[derive(Clone)]
pub struct ArchivedSms {
    id: [u8; 32],
    context: [u8; 32],
    sender: String,
    body: String,
    reported_timestamp: Option<String>,
    captured_unix_secs: u64,
    incomplete: bool,
}

impl std::fmt::Debug for ArchivedSms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchivedSms")
            .field("content", &"[REDACTED]")
            .field("captured_unix_secs", &self.captured_unix_secs)
            .finish()
    }
}

impl ArchivedSms {
    pub fn sender(&self) -> &str {
        &self.sender
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn reported_timestamp(&self) -> Option<&str> {
        self.reported_timestamp.as_deref()
    }
    pub fn captured_unix_secs(&self) -> u64 {
        self.captured_unix_secs
    }
    pub fn incomplete(&self) -> bool {
        self.incomplete
    }
    /// A non-sensitive label for grouping independent device/SIM contexts.
    pub fn context_label(&self) -> String {
        self.context[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

// Serialization is intentionally private. Neither the public row nor the service implements
// Serialize, so generic diagnostics cannot accidentally export sender/body plaintext.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRow {
    id: [u8; 32],
    context: [u8; 32],
    sender: String,
    body: String,
    reported_timestamp: Option<String>,
    captured_unix_secs: u64,
    incomplete: bool,
}

impl From<&ArchivedSms> for FileRow {
    fn from(row: &ArchivedSms) -> Self {
        Self {
            id: row.id,
            context: row.context,
            sender: row.sender.clone(),
            body: row.body.clone(),
            reported_timestamp: row.reported_timestamp.clone(),
            captured_unix_secs: row.captured_unix_secs,
            incomplete: row.incomplete,
        }
    }
}
impl From<FileRow> for ArchivedSms {
    fn from(row: FileRow) -> Self {
        Self {
            id: row.id,
            context: row.context,
            sender: row.sender,
            body: row.body,
            reported_timestamp: row.reported_timestamp,
            captured_unix_secs: row.captured_unix_secs,
            incomplete: row.incomplete,
        }
    }
}

trait Cipher: Send + Sync {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str>;
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str>;
}
struct UserCipher;
impl Cipher for UserCipher {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
        dji4g_windows_platform::sms_archive_crypto::protect(data)
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
        dji4g_windows_platform::sms_archive_crypto::unprotect(data)
    }
}

enum Command {
    Capture(Vec<ArchivedSms>),
    Clear,
    Export(PathBuf),
    Maintain,
}
struct Update {
    rows: Option<Vec<ArchivedSms>>,
    status: &'static str,
    failed: bool,
    completes_request: bool,
}

/// All filesystem access and DPAPI calls take place on one background thread. UI calls use
/// bounded, nonblocking queues; full queues retry the same snapshot on a subsequent frame.
pub struct ArchiveService {
    #[cfg(debug_assertions)]
    fixture: bool,
    tx: SyncSender<Command>,
    rx: Receiver<Update>,
    rows: Vec<ArchivedSms>,
    enabled: bool,
    pending: usize,
    ready: bool,
    failed: bool,
    status: &'static str,
    last_revision: Option<u64>,
}

impl ArchiveService {
    pub fn new(path: PathBuf) -> Self {
        Self::with_cipher(path, Arc::new(UserCipher))
    }
    fn with_cipher(path: PathBuf, cipher: Arc<dyn Cipher>) -> Self {
        Self::with_worker_options(
            path,
            cipher,
            std::time::Duration::from_secs(60),
            Arc::new(now_secs),
        )
    }
    fn with_worker_options(
        path: PathBuf,
        cipher: Arc<dyn Cipher>,
        maintenance_interval: std::time::Duration,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        let (tx, commands) = mpsc::sync_channel(4);
        let (updates, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("sms-archive".into())
            .spawn(move || {
                let mut store = ArchiveStore {
                    path,
                    cipher,
                    rows: Vec::new(),
                };
                let loaded = store.load(clock());
                let mut failed = loaded.is_err();
                // Losing the UI receiver must not abandon already authorized queued work,
                // especially Clear behind an in-flight Capture. Drain until senders are gone.
                let _ = updates.send(Update {
                    rows: (!failed).then(|| store.rows.clone()),
                    status: loaded
                        .err()
                        .unwrap_or("本地历史已就绪；仅保留最近 90 天、最多 5000 条"),
                    failed,
                    completes_request: true,
                });
                loop {
                    let command = match commands.recv_timeout(maintenance_interval) {
                        Ok(command) => command,
                        Err(mpsc::RecvTimeoutError::Timeout) if !failed => Command::Maintain,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let completes_request = !matches!(command, Command::Maintain);
                    let result = match command {
                        Command::Capture(rows) if !failed => store.capture(rows, clock()),
                        Command::Capture(_) => {
                            Err("档案读取或保存失败，已暂停采集；请检查或清空本地历史")
                        }
                        Command::Clear => store.clear(),
                        Command::Maintain => match store.maintain(clock()) {
                            Ok(false) => continue,
                            Ok(true) => Ok(()),
                            Err(error) => Err(error),
                        },
                        Command::Export(path) => {
                            let result = if failed {
                                Err("档案读取或保存失败，已暂停导出；请先处理档案错误")
                            } else {
                                store.export_at(&path, clock())
                            };
                            let _ = updates.send(Update {
                                rows: Some(store.rows.clone()),
                                status: result.err().unwrap_or("已导出明文 TXT，请妥善保管该文件"),
                                failed,
                                completes_request: true,
                            });
                            continue;
                        }
                    };
                    failed = result.is_err();
                    let _ = updates.send(Update {
                        rows: Some(store.rows.clone()),
                        failed,
                        completes_request,
                        status: result
                            .err()
                            .unwrap_or("本地历史已更新；仅保留最近 90 天、最多 5000 条"),
                    });
                }
            });
        let failed = spawned.is_err();
        Self {
            #[cfg(debug_assertions)]
            fixture: false,
            tx,
            rx,
            rows: Vec::new(),
            enabled: false,
            pending: usize::from(!failed),
            ready: failed,
            failed,
            status: if failed {
                "无法启动短信档案后台任务"
            } else {
                "正在读取本地历史…"
            },
            last_revision: None,
        }
    }
    pub fn poll(&mut self) {
        #[cfg(debug_assertions)]
        if self.fixture {
            return;
        }
        loop {
            match self.rx.try_recv() {
                Ok(update) => {
                    if update.completes_request {
                        self.pending = self.pending.saturating_sub(1);
                    }
                    self.ready = true;
                    self.failed = update.failed;
                    self.status = update.status;
                    if let Some(rows) = update.rows {
                        self.rows = rows;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = 0;
                    self.ready = true;
                    self.failed = true;
                    self.status = "短信档案后台任务已停止";
                    return;
                }
            }
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.last_revision = None;
        }
        self.enabled = enabled;
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn rows(&self) -> &[ArchivedSms] {
        &self.rows
    }
    pub fn status(&self) -> &str {
        self.status
    }
    pub fn busy(&self) -> bool {
        self.pending != 0
    }

    /// In-memory, invented records for the Noop capture harness; no file or DPAPI access.
    #[cfg(debug_assertions)]
    pub fn review_fixture(loaded: bool) -> Self {
        let (tx, _) = mpsc::sync_channel(1);
        let (_, rx) = mpsc::channel();
        Self {
            fixture: true,
            tx,
            rx,
            enabled: false,
            pending: 0,
            ready: true,
            failed: false,
            status: "模拟历史，仅用于界面验收，没有读取或保存真实短信",
            last_revision: None,
            rows: if loaded {
                (1..=3)
                    .map(|n| ArchivedSms {
                        id: [n; 32],
                        context: [n; 32],
                        sender: format!("+861380000000{n}"),
                        body: "【模拟短信】这是一条本地历史示例，仅用于检查阅读、筛选和导出提示。"
                            .into(),
                        reported_timestamp: Some("2026-09-22 12:00:00".into()),
                        captured_unix_secs: now_secs(),
                        incomplete: false,
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }
    }
    pub fn observe(&mut self, snapshot: &ControllerSnapshot) {
        if !self.enabled
            || !self.ready
            || self.failed
            || self.busy()
            || self.last_revision == Some(snapshot.publication_revision)
        {
            return;
        }
        let Some(context) = context_hash(snapshot) else {
            self.status = "未取得稳定设备和 SIM 身份，本次不写入本地历史";
            return;
        };
        let device_epoch = snapshot.app.device.as_ref().map(|device| device.epoch.0);
        let now = now_secs();
        let rows = snapshot
            .sms_messages
            .iter()
            .filter(|row| {
                row.message.direction == SmsDirection::Incoming
                    && Some(row.message.device_epoch) == device_epoch
                    && row.message.sim_epoch == snapshot.sim_epoch
            })
            .take(MAX_ARCHIVED_MESSAGES)
            .map(|row| archived_row(context, row, now))
            .collect();
        if self.queue(Command::Capture(rows)) {
            self.last_revision = Some(snapshot.publication_revision);
        }
    }
    /// Called only after the UI's explicit clear confirmation. Capture is disabled as part of
    /// clearing so the still-visible inbox cannot immediately re-populate the archive.
    pub fn clear(&mut self) {
        self.enabled = false;
        self.last_revision = None;
        self.queue(Command::Clear);
    }
    /// Explicit user-selected export only; output is unencrypted UTF-8 TXT. Existing paths
    /// are never overwritten, including the encrypted archive itself.
    pub fn export_text(&mut self, path: PathBuf) {
        self.queue(Command::Export(path));
    }
    fn queue(&mut self, command: Command) -> bool {
        match self.tx.try_send(command) {
            Ok(()) => {
                self.pending += 1;
                true
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.status = "本地历史正在处理，请稍后重试";
                false
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.failed = true;
                self.status = "短信档案后台任务已停止";
                false
            }
        }
    }
}

fn context_hash(snapshot: &ControllerSnapshot) -> Option<[u8; 32]> {
    let identity = &snapshot.app.device.as_ref()?.identity;
    let sim = &snapshot
        .app
        .cellular
        .as_ref()?
        .sim_identity
        .as_ref()?
        .fingerprint;
    if identity.container_id.trim().is_empty() || identity.device_instance_id.trim().is_empty() {
        return None;
    }
    let mut material = b"dji4g-archive-context-v1".to_vec();
    field(
        &mut material,
        identity.container_id.to_ascii_lowercase().as_bytes(),
    );
    field(
        &mut material,
        identity.device_instance_id.to_ascii_lowercase().as_bytes(),
    );
    field(&mut material, &identity.vid.to_le_bytes());
    field(&mut material, &identity.pid.to_le_bytes());
    field(&mut material, sim);
    Some(sha256(&material))
}

fn archived_row(context: [u8; 32], row: &SmsDisplayMessage, now: u64) -> ArchivedSms {
    let message = &row.message;
    let mut material = b"dji4g-archive-message-v1".to_vec();
    field(&mut material, &context);
    field(&mut material, message.sender().as_bytes());
    field(&mut material, message.body().as_bytes());
    field(
        &mut material,
        message
            .service_centre_timestamp
            .as_deref()
            .unwrap_or("")
            .as_bytes(),
    );
    field(&mut material, &message.payload_fingerprint());
    field(&mut material, message.storage.0.as_bytes());
    field(&mut material, &message.index.to_le_bytes());
    // Epoch counters only identify the running session, and must not affect restart dedup.
    // Physical index/storage plus payload preserve distinct identical messages in two slots.
    let mut fragments = row
        .fragments
        .iter()
        .map(|f| (&f.storage.0, f.index, f.payload_fingerprint))
        .collect::<Vec<_>>();
    fragments.sort();
    fragments.dedup();
    for (storage, index, payload) in fragments {
        field(&mut material, storage.as_bytes());
        field(&mut material, &index.to_le_bytes());
        field(&mut material, &payload);
    }
    ArchivedSms {
        id: sha256(&material),
        context,
        sender: message.sender().into(),
        body: message.body().into(),
        reported_timestamp: message.service_centre_timestamp.clone(),
        captured_unix_secs: now,
        incomplete: message.status == SmsStatus::Incomplete,
    }
}
fn field(material: &mut Vec<u8>, bytes: &[u8]) {
    material.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    material.extend_from_slice(bytes);
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct ArchiveStore {
    path: PathBuf,
    cipher: Arc<dyn Cipher>,
    rows: Vec<ArchivedSms>,
}
impl ArchiveStore {
    fn load(&mut self, now: u64) -> Result<(), &'static str> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("无法打开本地短信档案；原文件已保留"),
        };
        if file.metadata().map_err(|_| "无法检查档案大小")?.len() > MAX_FILE_BYTES {
            return Err("本地短信档案超过 32 MiB，未加载或覆盖");
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取短信档案失败")?;
        if bytes.len() as u64 > MAX_FILE_BYTES || !bytes.starts_with(MAGIC) {
            return Err("短信档案格式无效；原文件已保留");
        }
        let plain = self.cipher.unprotect(&bytes[MAGIC.len()..])?;
        if plain.len() as u64 > MAX_FILE_BYTES {
            return Err("解密后的短信档案超过大小限制");
        }
        let rows: Vec<FileRow> =
            serde_json::from_slice(&plain).map_err(|_| "短信档案内容损坏；原文件已保留")?;
        if rows.len() > MAX_ARCHIVED_MESSAGES {
            return Err("短信档案记录数超过上限；原文件已保留");
        }
        self.rows = rows.into_iter().map(Into::into).collect();
        if self.purge(now) {
            self.save()?;
        }
        Ok(())
    }
    fn capture(&mut self, rows: Vec<ArchivedSms>, now: u64) -> Result<(), &'static str> {
        let previous = self.rows.clone();
        let mut changed = self.purge(now);
        let mut known: HashSet<_> = self.rows.iter().map(|row| row.id).collect();
        for row in rows {
            if known.insert(row.id) {
                self.rows.push(row);
                changed = true;
            }
        }
        changed |= self.purge(now);
        if changed {
            if let Err(error) = self.save() {
                self.rows = previous;
                return Err(error);
            }
        }
        Ok(())
    }
    fn purge(&mut self, now: u64) -> bool {
        let length = self.rows.len();
        let mut known = HashSet::new();
        self.rows.retain(|row| {
            now.saturating_sub(row.captured_unix_secs) < RETENTION_DAYS * 86400
                && row.captured_unix_secs <= now.saturating_add(86400)
                && known.insert(row.id)
        });
        self.rows.sort_by_key(|row| row.captured_unix_secs);
        if self.rows.len() > MAX_ARCHIVED_MESSAGES {
            self.rows.drain(..self.rows.len() - MAX_ARCHIVED_MESSAGES);
        }
        length != self.rows.len()
    }
    fn save(&self) -> Result<(), &'static str> {
        let plain = serde_json::to_vec(&self.rows.iter().map(FileRow::from).collect::<Vec<_>>())
            .map_err(|_| "无法编码短信档案")?;
        if plain.len() as u64 > MAX_FILE_BYTES {
            return Err("短信档案已达大小上限，本次未保存");
        }
        let encrypted = self.cipher.protect(&plain)?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&encrypted);
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("加密档案已达大小上限，本次未保存");
        }
        atomic_write(&self.path, &bytes)
    }
    fn clear(&mut self) -> Result<(), &'static str> {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法清空本地短信档案；未删除现有记录"),
        }
        self.rows.clear();
        Ok(())
    }
    fn maintain(&mut self, now: u64) -> Result<bool, &'static str> {
        let previous = self.rows.clone();
        let changed = self.purge(now);
        if changed {
            if let Err(error) = self.save() {
                self.rows = previous;
                return Err(error);
            }
        }
        Ok(changed)
    }
    #[cfg(test)]
    fn export(&mut self, path: &Path) -> Result<(), &'static str> {
        self.export_at(path, now_secs())
    }
    fn export_at(&mut self, path: &Path, now: u64) -> Result<(), &'static str> {
        // Retention applies even with capture disabled and before plaintext leaves the archive.
        self.maintain(now)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|_| "无法创建导出目录，请选择可写位置")?;
        }
        // create_new also protects the canonical archive and aliases to any existing file.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "无法导出：请选择可写位置和一个不存在的新文件名")?;
        let result = (|| -> std::io::Result<()> {
            writeln!(
                file,
                "DJI 4G 本地短信历史（明文导出）\n采集时间为 Unix UTC 秒；原报时间保留模块原文。\n"
            )?;
            for row in &self.rows {
                writeln!(
                    file,
                    "设备/SIM 分区：{}\n发件人：{}\n原报时间：{}\n采集时间：{}\n内容{}：\n{}\n--------",
                    row.context_label(),
                    row.sender(),
                    row.reported_timestamp().unwrap_or("未知"),
                    row.captured_unix_secs,
                    if row.incomplete {
                        "（分片不完整）"
                    } else {
                        ""
                    },
                    row.body()
                )?;
            }
            file.sync_all()
        })();
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(path);
            return Err("导出写入失败，未保留不完整文件");
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("短信档案路径无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建短信档案目录")?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .ok_or("短信档案路径无效")?
        .to_string_lossy();
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "无法创建加密档案临时文件")?;
    let write = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = write.map_err(|_| "写入加密档案失败").and_then(|()| {
        dji4g_windows_platform::atomic_replace_file(&temp, path)
            .map_err(|_| "替换加密档案失败；原文件已保留")
    });
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_domain::{
        AttachState, CellularSnapshot, DeviceEpoch, DeviceSnapshot, FeatureStatus,
        RegistrationState, SimIdentity, SimState, SmsEncoding, SmsMessage, SmsStorageId,
        StableDeviceIdentity,
    };

    struct TestCipher;
    impl Cipher for TestCipher {
        fn protect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
            Ok(b"test-only"
                .iter()
                .copied()
                .chain(data.iter().map(|v| v ^ 0xa5))
                .collect())
        }
        fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
            data.strip_prefix(b"test-only")
                .map(|data| data.iter().map(|v| v ^ 0xa5).collect())
                .ok_or("test decrypt failed")
        }
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "dji4g-sms-archive-test-{}-{}-{}",
                std::process::id(),
                now_secs(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> PathBuf {
            self.0.join("history.dpapi")
        }
        fn store(&self) -> ArchiveStore {
            ArchiveStore {
                path: self.path(),
                cipher: Arc::new(TestCipher),
                rows: Vec::new(),
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn message(index: u32, epoch: u64) -> SmsDisplayMessage {
        let mut message = SmsMessage::new(
            index,
            SmsStorageId("SM".into()),
            epoch,
            epoch,
            "+100000-test",
            "synthetic SECRET body",
            SmsEncoding::Gsm7,
            SmsStatus::Received,
        );
        message.service_centre_timestamp = Some("26/09/22,20:00:00+32".into());
        let fragments = vec![message.fragment_key()];
        SmsDisplayMessage {
            message,
            fragments,
            delete_allowed: true,
        }
    }
    fn snapshot() -> ControllerSnapshot {
        let mut snapshot = dji4g_application::ReducerState::new(SystemTime::now()).snapshot();
        let app = Arc::make_mut(&mut snapshot.app);
        app.device = Some(DeviceSnapshot {
            epoch: DeviceEpoch(1),
            identity: StableDeviceIdentity {
                container_id: "test-container".into(),
                device_instance_id: "USB\\VID_2CA3&PID_4006\\SYNTHETIC".into(),
                vid: 0x2ca3,
                pid: 0x4006,
            },
            problem_code: None,
            at_port: None,
            adapter_id: None,
        });
        app.cellular = Some(CellularSnapshot {
            sim: SimState::Ready,
            registration: RegistrationState::RegisteredHome,
            attached: AttachState::Attached,
            carrier: None,
            radio_access_technology: None,
            signal_rssi_dbm: None,
            apn: None,
            pdp_address: None,
            pdp_state: None,
            firmware: None,
            serving_cell: None,
            sim_identity: Some(SimIdentity {
                iccid_masked: "test".into(),
                fingerprint: [1; 8],
            }),
            numbers: None,
            temperature_celsius: None,
            temperature_status: FeatureStatus::NotProbed,
        });
        snapshot.sim_epoch = 1;
        snapshot.sms_messages = vec![message(1, 1)];
        snapshot
    }
    fn settle(service: &mut ArchiveService) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while service.busy() {
            service.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn encrypted_roundtrip_dedup_across_restart_and_distinct_contexts() {
        let temp = Temp::new();
        let mut store = temp.store();
        let now = now_secs();
        store
            .capture(vec![archived_row([1; 32], &message(1, 1), now)], now)
            .unwrap();
        let bytes = fs::read(temp.path()).unwrap();
        assert!(!bytes.windows(b"SECRET".len()).any(|v| v == b"SECRET"));
        let mut restarted = temp.store();
        restarted.load(now).unwrap();
        restarted
            .capture(
                vec![archived_row([1; 32], &message(1, 8), now + 1)],
                now + 1,
            )
            .unwrap();
        assert_eq!(restarted.rows.len(), 1);
        assert_eq!(restarted.rows[0].captured_unix_secs(), now);
        assert_eq!(
            restarted.rows[0].reported_timestamp(),
            Some("26/09/22,20:00:00+32")
        );
        restarted
            .capture(
                vec![
                    archived_row([2; 32], &message(1, 8), now + 1),
                    archived_row([1; 32], &message(2, 8), now + 1),
                ],
                now + 1,
            )
            .unwrap();
        assert_eq!(restarted.rows.len(), 3);
        assert!(!format!("{:?}", restarted.rows).contains("SECRET"));
        assert!(!format!("{:?}", restarted.rows).contains("+100000"));
    }
    #[test]
    fn retention_purges_at_load_and_capture_and_caps_rows() {
        let temp = Temp::new();
        let mut store = temp.store();
        let now = now_secs();
        store.rows = (0..5002)
            .map(|i| archived_row([1; 32], &message(i, 1), now - u64::from(5002 - i)))
            .collect();
        assert!(store.purge(now));
        assert_eq!(store.rows.len(), 5000);
        store.rows[0].captured_unix_secs = now - RETENTION_DAYS * 86400;
        store.save().unwrap();
        let mut loaded = temp.store();
        loaded.load(now).unwrap();
        assert_eq!(loaded.rows.len(), 4999);
        loaded
            .capture(Vec::new(), now + RETENTION_DAYS * 86400)
            .unwrap();
        assert!(loaded.rows.is_empty());
    }
    #[test]
    fn corruption_and_oversize_do_not_overwrite_original() {
        let temp = Temp::new();
        let mut store = temp.store();
        fs::write(temp.path(), b"invalid").unwrap();
        assert!(store.load(now_secs()).is_err());
        assert_eq!(fs::read(temp.path()).unwrap(), b"invalid");
        let file = File::create(temp.path()).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        drop(file);
        assert!(store.load(now_secs()).is_err());
        assert_eq!(fs::metadata(temp.path()).unwrap().len(), MAX_FILE_BYTES + 1);
    }
    #[test]
    fn save_failure_keeps_committed_rows_and_never_writes_plaintext() {
        struct FailingCipher;
        impl Cipher for FailingCipher {
            fn protect(&self, _: &[u8]) -> Result<Vec<u8>, &'static str> {
                Err("synthetic cipher failure")
            }
            fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, &'static str> {
                Err("synthetic cipher failure")
            }
        }
        let temp = Temp::new();
        let mut store = temp.store();
        let now = now_secs();
        store
            .capture(vec![archived_row([1; 32], &message(1, 1), now)], now)
            .unwrap();
        let before = fs::read(temp.path()).unwrap();
        store.cipher = Arc::new(FailingCipher);
        assert!(
            store
                .capture(vec![archived_row([1; 32], &message(2, 1), now)], now)
                .is_err()
        );
        assert_eq!(store.rows.len(), 1);
        assert_eq!(fs::read(temp.path()).unwrap(), before);
        store.cipher = Arc::new(TestCipher);
        store.path = temp.path().join("bad-parent.dpapi");
        assert!(
            store
                .capture(vec![archived_row([1; 32], &message(2, 1), now)], now)
                .is_err()
        );
        assert_eq!(store.rows.len(), 1);
    }
    #[test]
    fn default_disabled_context_gaps_epochs_and_clear_are_safe() {
        let temp = Temp::new();
        let mut service = ArchiveService::with_cipher(temp.path(), Arc::new(TestCipher));
        settle(&mut service);
        let mut snapshot = snapshot();
        service.observe(&snapshot);
        settle(&mut service);
        assert!(!temp.path().exists());
        service.set_enabled(true);
        Arc::make_mut(&mut snapshot.app)
            .cellular
            .as_mut()
            .unwrap()
            .sim_identity = None;
        service.observe(&snapshot);
        assert!(service.status().contains("身份"));
        assert!(!temp.path().exists());
        snapshot = self::snapshot();
        snapshot.sms_messages[0].message.sim_epoch = 0;
        service.observe(&snapshot);
        settle(&mut service);
        assert!(service.rows().is_empty());
        snapshot = self::snapshot();
        snapshot.publication_revision += 1;
        service.observe(&snapshot);
        settle(&mut service);
        assert_eq!(service.rows().len(), 1);
        service.set_enabled(false);
        snapshot.publication_revision += 1;
        snapshot.sms_messages.push(message(2, 1));
        service.observe(&snapshot);
        settle(&mut service);
        assert_eq!(service.rows().len(), 1);
        service.clear();
        settle(&mut service);
        assert!(!service.enabled());
        assert!(service.rows().is_empty());
        assert!(!temp.path().exists());
    }
    #[test]
    fn device_and_sim_identity_never_merge() {
        let mut snapshot = snapshot();
        let original = context_hash(&snapshot).unwrap();
        Arc::make_mut(&mut snapshot.app)
            .cellular
            .as_mut()
            .unwrap()
            .sim_identity
            .as_mut()
            .unwrap()
            .fingerprint = [2; 8];
        assert_ne!(context_hash(&snapshot).unwrap(), original);
        snapshot = self::snapshot();
        Arc::make_mut(&mut snapshot.app)
            .device
            .as_mut()
            .unwrap()
            .identity
            .container_id = "other".into();
        assert_ne!(context_hash(&snapshot).unwrap(), original);
        Arc::make_mut(&mut snapshot.app)
            .device
            .as_mut()
            .unwrap()
            .identity
            .container_id
            .clear();
        assert!(context_hash(&snapshot).is_none());
    }
    #[test]
    fn explicit_txt_export_cannot_overwrite_archive_or_existing_file() {
        let temp = Temp::new();
        let mut store = temp.store();
        let now = now_secs();
        store
            .capture(vec![archived_row([1; 32], &message(1, 1), now)], now)
            .unwrap();
        let before = fs::read(temp.path()).unwrap();
        assert!(store.export(&temp.path()).is_err());
        assert_eq!(fs::read(temp.path()).unwrap(), before);
        let export = temp.0.join("chosen.txt");
        store.export(&export).unwrap();
        assert!(
            fs::read_to_string(&export)
                .unwrap()
                .contains("synthetic SECRET body")
        );
        assert!(store.export(&export).is_err());
    }

    #[test]
    fn disabled_startup_loads_history_and_clear_wins_over_pending_capture() {
        let temp = Temp::new();
        let now = now_secs();
        temp.store()
            .capture(vec![archived_row([1; 32], &message(1, 1), now)], now)
            .unwrap();
        let mut service = ArchiveService::with_cipher(temp.path(), Arc::new(TestCipher));
        settle(&mut service);
        assert!(!service.enabled());
        assert_eq!(service.rows().len(), 1);
        service.set_enabled(true);
        service.observe(&snapshot());
        // Do not poll before clear: capture is queued/in flight. FIFO must ensure it cannot
        // finish *after* clearing and resurrect records on disk.
        service.clear();
        settle(&mut service);
        assert!(service.rows().is_empty());
        assert!(!temp.path().exists());
        service.observe(&snapshot());
        assert!(!service.busy());
    }

    #[test]
    fn corrupt_startup_blocks_capture_until_explicit_clear() {
        let temp = Temp::new();
        fs::write(temp.path(), b"corrupt original").unwrap();
        let mut service = ArchiveService::with_cipher(temp.path(), Arc::new(TestCipher));
        settle(&mut service);
        service.set_enabled(true);
        service.observe(&snapshot());
        assert!(!service.busy());
        assert_eq!(fs::read(temp.path()).unwrap(), b"corrupt original");
        service.clear();
        settle(&mut service);
        service.set_enabled(true);
        service.observe(&snapshot());
        settle(&mut service);
        assert_eq!(service.rows().len(), 1);
    }

    #[test]
    fn dropping_receiver_still_drains_authorized_clear_after_capture() {
        struct BlockingCipher {
            entered: mpsc::Sender<()>,
            release: std::sync::Mutex<mpsc::Receiver<()>>,
            stopped: mpsc::Sender<()>,
        }
        impl Cipher for BlockingCipher {
            fn protect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                TestCipher.protect(data)
            }
            fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
                TestCipher.unprotect(data)
            }
        }
        impl Drop for BlockingCipher {
            fn drop(&mut self) {
                let _ = self.stopped.send(());
            }
        }
        let temp = Temp::new();
        let (entered, started) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (stopped, finished) = mpsc::channel();
        let cipher = Arc::new(BlockingCipher {
            entered,
            release: std::sync::Mutex::new(released),
            stopped,
        });
        let mut service = ArchiveService::with_cipher(temp.path(), cipher);
        settle(&mut service);
        service.set_enabled(true);
        service.observe(&snapshot());
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        service.clear();
        drop(service);
        release.send(()).unwrap();
        finished
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(
            !temp.path().exists(),
            "queued clear must survive dropping the update receiver"
        );
    }

    #[test]
    fn export_purges_expired_rows_before_creating_plaintext() {
        let temp = Temp::new();
        let mut store = temp.store();
        let now = now_secs();
        store.rows = vec![archived_row(
            [1; 32],
            &message(1, 1),
            now - RETENTION_DAYS * 86400,
        )];
        store.save().unwrap();
        let export = temp.0.join("expired.txt");
        store.export(&export).unwrap();
        assert!(!fs::read_to_string(export).unwrap().contains("SECRET"));
        assert!(store.rows.is_empty());
    }

    #[test]
    fn export_creates_missing_parent_but_never_overwrites_existing_output() {
        let temp = Temp::new();
        let mut store = temp.store();
        let now = now_secs();
        store
            .capture(vec![archived_row([1; 32], &message(1, 1), now)], now)
            .unwrap();
        let output = temp.0.join("new-exports").join("chosen.txt");
        assert!(!output.parent().unwrap().exists());
        store.export(&output).unwrap();
        let original = fs::read(&output).unwrap();
        assert!(String::from_utf8_lossy(&original).contains("SECRET"));
        assert!(store.export(&output).is_err());
        assert_eq!(fs::read(&output).unwrap(), original);
    }

    #[test]
    fn disabled_idle_worker_purges_by_clock_without_rewriting_unchanged_data() {
        struct CountingCipher(Arc<AtomicU64>);
        impl Cipher for CountingCipher {
            fn protect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
                self.0.fetch_add(1, Ordering::SeqCst);
                TestCipher.protect(data)
            }
            fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
                TestCipher.unprotect(data)
            }
        }
        let temp = Temp::new();
        let now = now_secs();
        temp.store()
            .capture(vec![archived_row([1; 32], &message(1, 1), now)], now)
            .unwrap();
        let clock = Arc::new(AtomicU64::new(now));
        let reads = Arc::new(AtomicU64::new(0));
        let writes = Arc::new(AtomicU64::new(0));
        let worker_clock = Arc::clone(&clock);
        let worker_reads = Arc::clone(&reads);
        let mut service = ArchiveService::with_worker_options(
            temp.path(),
            Arc::new(CountingCipher(Arc::clone(&writes))),
            std::time::Duration::from_millis(5),
            Arc::new(move || {
                worker_reads.fetch_add(1, Ordering::SeqCst);
                worker_clock.load(Ordering::SeqCst)
            }),
        );
        settle(&mut service);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while reads.load(Ordering::SeqCst) < 4 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!service.enabled());
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        clock.store(now + RETENTION_DAYS * 86400, Ordering::SeqCst);
        while !service.rows().is_empty() {
            service.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !service.busy(),
            "timer updates must not represent queued UI requests"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        let mut reloaded = temp.store();
        reloaded.load(clock.load(Ordering::SeqCst)).unwrap();
        assert!(reloaded.rows.is_empty());
    }
}
