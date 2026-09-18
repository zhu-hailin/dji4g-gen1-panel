//! Detailed, user-triggered support reports; independent of the controller's progress.
use dji4g_application::ControllerSnapshot;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SCRIPT: &str = include_str!("../../../packaging/scripts/collect-support-info.ps1");
const OUTPUT_LIMIT: usize = 512 * 1024;
const FILE_LIMIT: u64 = 256 * 1024;
const SECTIONS: &[(&str, &str)] = &[
    ("system", "系统及程序进程"),
    ("usb-and-problem-devices", "USB 与异常设备"),
    ("installed-drivers", "已绑定驱动与 INF"),
    ("serial-ports", "串口枚举"),
    ("network-adapters", "网卡与驱动"),
    ("ip-dns-routes", "IP、DNS 与默认路由"),
    ("security-products", "安全软件状态"),
    ("driver-install-events", "Windows 驱动安装记录"),
    ("bundle-integrity", "程序及驱动文件完整性"),
];

pub enum ReportEvent {
    Progress(String),
    Finished(Result<PathBuf, String>),
}

#[derive(Default)]
pub struct ReportState {
    receiver: Option<mpsc::Receiver<ReportEvent>>,
    pub status: String,
    pub path: Option<PathBuf>,
    history: std::collections::VecDeque<String>,
}

impl ReportState {
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn observe(&mut self, previous: &ControllerSnapshot, current: &ControllerSnapshot) {
        if previous.diagnostics != current.diagnostics
            || previous.app.availability != current.app.availability
        {
            let event = progress_detail(current);
            crate::logging::append_event(&format!("detection_state {event}"));
            self.history.push_back(event);
            while self.history.len() > 64 {
                self.history.pop_front();
            }
        }
    }

    pub fn request(&mut self, directory: Option<PathBuf>, snapshot: Arc<ControllerSnapshot>) {
        if self.busy() {
            return;
        }
        self.path = None;
        let Some(directory) = directory else {
            self.status = "无法导出：当前用户的日志目录不可用".into();
            return;
        };
        match start(
            directory,
            snapshot,
            self.history.iter().cloned().collect::<Vec<_>>().join("\n"),
        ) {
            Ok(receiver) => {
                self.receiver = Some(receiver);
                self.status = "正在准备详细日志…".into();
            }
            Err(error) => self.status = format!("无法启动导出：{error}"),
        }
    }

    pub fn poll(&mut self) {
        loop {
            let Some(receiver) = &self.receiver else {
                return;
            };
            match receiver.try_recv() {
                Ok(ReportEvent::Progress(text)) => {
                    self.status = format!("正在导出详细日志 · {text}")
                }
                Ok(ReportEvent::Finished(result)) => {
                    self.receiver = None;
                    match result {
                        Ok(path) => {
                            self.status = format!("详细日志已导出：{}", path.display());
                            self.path = Some(path);
                        }
                        Err(error) => {
                            self.status =
                                format!("导出未完成：{error}（已生成的部分报告保留在导出目录）")
                        }
                    }
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.status = "导出线程提前结束，部分报告保留在导出目录".into();
                    return;
                }
            }
        }
    }
}

pub fn open_report_folder(path: &Path) -> io::Result<()> {
    // Resolve Explorer from the same OS-owned Windows directory as the trusted shell.
    let shell = dji4g_windows_platform::driver_setup_powershell()?;
    let windows = shell
        .ancestors()
        .nth(4)
        .ok_or_else(|| io::Error::other("Windows path unavailable"))?;
    Command::new(windows.join("explorer.exe"))
        .arg(
            path.parent()
                .ok_or_else(|| io::Error::other("report directory unavailable"))?,
        )
        .spawn()?;
    Ok(())
}

/// Called on the UI thread, but performs all I/O on a separate worker. A stalled
/// controller is never asked to service this request.
pub fn start(
    directory: PathBuf,
    snapshot: Arc<ControllerSnapshot>,
    history: String,
) -> io::Result<mpsc::Receiver<ReportEvent>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("support-report".into())
        .spawn(move || {
            let result = collect(&directory, &snapshot, &history, |text| {
                let _ = tx.send(ReportEvent::Progress(text));
            })
            .map_err(|error| error.to_string());
            let _ = tx.send(ReportEvent::Finished(result));
        })?;
    Ok(rx)
}

fn read_bounded(mut input: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let count = match input.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if count == 0 {
            break;
        }
        let keep = count.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    Ok((bytes, truncated))
}

pub fn progress_detail(snapshot: &ControllerSnapshot) -> String {
    format!(
        "captured_unix_ms={} publication_revision={} observed_at={:?}\navailability={:?} freshness={:?} command={:?}\nchecks={:#?}\n",
        millis(),
        snapshot.publication_revision,
        snapshot.app.observed_at,
        snapshot.app.availability,
        snapshot.app.freshness,
        snapshot.command_state,
        snapshot.diagnostics
    )
}

pub fn snapshot_detail(snapshot: &ControllerSnapshot) -> String {
    // Explicit whitelist: never Debug/Serialize the whole snapshot (it contains SMS).
    format!(
        "{}\ndevice={:#?}\nnetwork={:#?}\nissues={:#?}\nreadiness={:#?}\nfeedback={:#?}\nfirmware={:?}\n",
        progress_detail(snapshot),
        snapshot.app.device,
        snapshot.app.network,
        snapshot.app.issues,
        snapshot.action_readiness,
        snapshot.feedback,
        snapshot
            .app
            .cellular
            .as_ref()
            .and_then(|cellular| cellular.firmware.as_ref())
    )
}

fn millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Runs only a fixed, read-only script. Captures exit status, elapsed time, stderr,
/// truncation and timeout independently; pipe output is drained concurrently.
fn run_bounded(command: &mut Command, timeout: Duration) -> String {
    let started = Instant::now();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return format!(
                "status=SPAWN_FAILED os_error={:?}\n{error}\n",
                error.raw_os_error()
            );
        }
    };
    let (tx, rx) = mpsc::channel();
    for (label, stream) in [
        (
            "stdout",
            Box::new(child.stdout.take().expect("piped stdout")) as Box<dyn Read + Send>,
        ),
        (
            "stderr",
            Box::new(child.stderr.take().expect("piped stderr")) as Box<dyn Read + Send>,
        ),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((label, read_bounded(stream, OUTPUT_LIMIT)));
        });
    }
    drop(tx);
    let status = loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                break format!(
                    "{} exit={exit}",
                    if exit.success() { "OK" } else { "FAILED" }
                );
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break format!("WAIT_FAILED {error}");
            }
            Ok(None) if started.elapsed() >= timeout => {
                let killed = child.kill();
                if killed.is_ok() {
                    let _ = child.wait();
                }
                break format!("TIMED_OUT limit_ms={} kill={killed:?}", timeout.as_millis());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let mut report = format!(
        "status={status} elapsed_ms={}\n",
        started.elapsed().as_millis()
    );
    for _ in 0..2 {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok((label, Ok((bytes, truncated)))) => report.push_str(&format!(
                "{label} truncated={truncated}:\n{}\n",
                String::from_utf8_lossy(&bytes)
            )),
            Ok((label, Err(error))) => report.push_str(&format!("{label}: READ_FAILED {error}\n")),
            Err(error) => {
                report.push_str(&format!("PIPE_INCOMPLETE: {error}\n"));
                break;
            }
        }
    }
    report
}

pub fn collect(
    directory: &Path,
    snapshot: &ControllerSnapshot,
    history: &str,
    mut progress: impl FnMut(String),
) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let name = format!("大疆4G详细诊断-{}-{}.txt", millis(), std::process::id());
    let path = directory.join(name);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let exe = std::env::current_exe()?;
    let app_root = exe
        .parent()
        .ok_or_else(|| io::Error::other("executable directory missing"))?;
    writeln!(
        output,
        "DJI 4G SUPPORT REPORT schema=1\nREPORT_STARTED unix_ms={}\napp_version={} exe={} pid={} architecture={}\n包含设备实例 ID、硬件 ID、驱动、网络配置及本程序日志。请仅发给排障人员。\n不读取短信正文、通讯录、SIM 号码或口令；不安装驱动、不修改网络、不发送 AT 命令。\n某节失败或超时不会阻止其他节导出。文件末尾 REPORT_COMPLETE 表示收集流程结束，不代表设备正常。\n",
        millis(),
        env!("CARGO_PKG_VERSION"),
        exe.display(),
        std::process::id(),
        std::env::consts::ARCH
    )?;
    let summary = crate::diagnostics_export::build(snapshot, SystemTime::now());
    section(&mut output, "界面诊断摘要", &summary.human)?;
    section(&mut output, "机器可读快照", &summary.json)?;
    section(
        &mut output,
        "检测阶段、时间、错误码和设备绑定",
        &snapshot_detail(snapshot),
    )?;
    section(
        &mut output,
        "最近检测状态变化（仅本次进程已观察到的变化）",
        history,
    )?;
    progress("收集程序及驱动安装日志".into());
    let log_root = directory.parent().map(|parent| parent.join("logs"));
    if let Some(root) = log_root {
        collect_logs(&mut output, &root, "dji4g-panel", 5)?;
    }
    collect_logs(&mut output, app_root, "driver-setup-", 8)?;
    // The user may have used an older standalone/setup build before this one.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for family in ["standalone", "versions"] {
            let root = Path::new(&local).join("Dji4GPanel").join(family);
            match fs::read_dir(&root) {
                Ok(entries) => {
                    let mut dirs: Vec<_> = entries
                        .filter_map(Result::ok)
                        .filter(|entry| {
                            entry
                                .file_type()
                                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                        })
                        .collect();
                    dirs.sort_by_key(|entry| {
                        std::cmp::Reverse(
                            entry
                                .metadata()
                                .and_then(|meta| meta.modified())
                                .unwrap_or(UNIX_EPOCH),
                        )
                    });
                    if dirs.len() > 12 {
                        section(
                            &mut output,
                            "历史安装日志范围",
                            "目录超过 12 个，仅收集最近 12 个版本",
                        )?;
                    }
                    for entry in dirs.into_iter().take(12) {
                        if entry.path() != app_root {
                            collect_logs(&mut output, &entry.path(), "driver-setup-", 2)?;
                        }
                    }
                }
                Err(error) => section(
                    &mut output,
                    "历史安装日志",
                    &format!("{}: {error}", root.display()),
                )?,
            }
        }
    }
    let shell = dji4g_windows_platform::driver_setup_powershell();
    for (index, (id, title)) in SECTIONS.iter().enumerate() {
        progress(format!("{}/{}：{title}", index + 1, SECTIONS.len()));
        let result = match &shell {
            Ok(shell) => run_bounded(
                Command::new(shell)
                    .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
                    .env_remove("PSModulePath")
                    .env("DJI4G_DIAG_SECTION", id)
                    .env("DJI4G_DIAG_APP_ROOT", app_root),
                Duration::from_secs(
                    if matches!(*id, "usb-and-problem-devices" | "installed-drivers") {
                        25
                    } else {
                        8
                    },
                ),
            ),
            Err(error) => format!("status=UNAVAILABLE {error}"),
        };
        section(&mut output, title, &result)?;
    }
    writeln!(output, "\nREPORT_COMPLETE unix_ms={}\n", millis())?;
    output.flush()?;
    Ok(path)
}

fn section(output: &mut impl Write, title: &str, content: &str) -> io::Result<()> {
    writeln!(output, "\n========== {title} ==========\n{content}")?;
    output.flush()
}

fn collect_logs(
    output: &mut impl Write,
    root: &Path,
    prefix: &str,
    max_files: usize,
) -> io::Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => return section(output, "日志目录", &format!("{}: {error}", root.display())),
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with(prefix)
                && name.ends_with(".log")
                && entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
        })
        .collect();
    files.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    section(
        output,
        "日志收集范围",
        &format!(
            "directory={} matched={} selected_limit={max_files} tail_byte_limit={FILE_LIMIT}",
            root.display(),
            files.len()
        ),
    )?;
    for entry in files.into_iter().take(max_files) {
        let result = (|| -> io::Result<String> {
            let mut file = File::open(entry.path())?;
            let size = file.metadata()?.len();
            file.seek(SeekFrom::Start(size.saturating_sub(FILE_LIMIT)))?;
            let (bytes, truncated) = read_bounded(file.take(FILE_LIMIT), FILE_LIMIT as usize)?;
            Ok(format!(
                "file_bytes={size} tail_only={} truncated={truncated}\n{}",
                size > FILE_LIMIT,
                crate::logging::redact_sensitive(&String::from_utf8_lossy(&bytes))
            ))
        })();
        section(
            output,
            &entry.path().display().to_string(),
            &result.unwrap_or_else(|error| format!("READ_FAILED: {error}")),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_application::{
        BackendEvent, CheckMask, CheckResult, PortError, ReducerState, RefreshCycleId, reduce_state,
    };
    use dji4g_domain::{DeviceEpoch, ErrorCode};
    use std::time::SystemTime;

    #[test]
    fn output_is_bounded_and_truncation_is_explicit() {
        let (bytes, truncated) = read_bounded(&b"123456789"[..], 4).unwrap();
        assert_eq!(bytes, b"1234");
        assert!(truncated);
        assert_eq!(
            read_bounded(&b"1234"[..], 4).unwrap(),
            (b"1234".to_vec(), false)
        );
    }

    #[test]
    fn failed_inventory_is_exported_even_without_a_device_snapshot() {
        let now = SystemTime::now();
        let state = ReducerState::new(now);
        let cycle = RefreshCycleId(1);
        let epoch = DeviceEpoch(0);
        let state = reduce_state(
            &state,
            BackendEvent::RefreshStarted {
                cycle,
                epoch,
                scheduled: CheckMask::all(),
            },
            now,
        );
        let state = reduce_state(
            &state,
            BackendEvent::InventoryFinished {
                cycle,
                epoch,
                result: CheckResult::Failed {
                    code: PortError::new(ErrorCode::PermissionDenied, "pnp:access_denied").code,
                    observed_at: now,
                },
            },
            now,
        );
        assert!(state.snapshot().app.device.is_none());
        let text = snapshot_detail(&state.snapshot());
        assert!(text.contains("pnp:access_denied"), "{text}");
        assert!(text.contains("UsbDevice"));
        assert!(text.contains("started_at"));
        assert!(!text.contains("sms_messages"));
    }

    #[cfg(windows)]
    #[test]
    fn failed_probe_keeps_exit_code_and_both_streams() {
        let shell = dji4g_windows_platform::driver_setup_powershell().unwrap();
        let result = run_bounded(Command::new(shell).args(["-NoProfile", "-NonInteractive", "-Command",
            "[Console]::Out.WriteLine('probe-started'); [Console]::Error.WriteLine('access-denied-detail'); exit 7"]), Duration::from_secs(8));
        assert!(result.contains("status=FAILED"), "{result}");
        assert!(result.contains('7'), "{result}");
        assert!(result.contains("probe-started"), "{result}");
        assert!(result.contains("access-denied-detail"), "{result}");
    }

    #[cfg(windows)]
    #[test]
    fn hung_probe_times_out_and_next_probe_still_runs() {
        let shell = dji4g_windows_platform::driver_setup_powershell().unwrap();
        let started = Instant::now();
        let result = run_bounded(
            Command::new(&shell).args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]),
            Duration::from_millis(300),
        );
        assert!(result.contains("TIMED_OUT"), "{result}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let next = run_bounded(
            Command::new(shell).args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output 'next-probe'; exit 0",
            ]),
            Duration::from_secs(8),
        );
        assert!(next.contains("status=OK"), "{next}");
        assert!(next.contains("next-probe"));
    }

    #[test]
    fn log_collection_excludes_message_store_and_limits_large_logs() {
        let root =
            std::env::temp_dir().join(format!("dji4g-support-{}-{}", std::process::id(), millis()));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("sms-messages.json"),
            "private-message-must-not-export",
        )
        .unwrap();
        fs::write(
            root.join("dji4g-panel.log"),
            "driver error=28 phone=13800138000 body=secret-message",
        )
        .unwrap();
        fs::write(
            root.join("dji4g-panel.1.log"),
            vec![b'x'; FILE_LIMIT as usize + 64],
        )
        .unwrap();
        let mut output = Vec::new();
        collect_logs(&mut output, &root, "dji4g-panel", 5).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("error=28"));
        assert!(text.contains("tail_only=true"));
        assert!(!text.contains("private-message-must-not-export"));
        assert!(!text.contains("13800138000"));
        assert!(!text.contains("secret-message"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_disconnection_becomes_visible_error_instead_of_staying_busy() {
        let (tx, rx) = mpsc::channel();
        let mut state = ReportState {
            receiver: Some(rx),
            ..Default::default()
        };
        drop(tx);
        state.poll();
        assert!(!state.busy());
        assert!(state.status.contains("提前结束"));
    }
}
