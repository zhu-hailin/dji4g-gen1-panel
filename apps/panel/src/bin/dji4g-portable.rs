#![cfg_attr(windows, windows_subsystem = "windows")]
//! One distributable EXE. Native resource extraction, no install script or shortcuts.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
    process::Command,
};
include!(concat!(env!("OUT_DIR"), "/standalone_bundle.rs"));

fn prepare_files(root: &Path, files: &[(&str, &[u8])]) -> Result<Vec<File>, String> {
    let mut locks = Vec::new();
    for (name, bytes) in files {
        if name.is_empty()
            || !Path::new(name)
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(format!("安装资源路径无效：{name}"));
        }
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().ok_or("资源目录无效")?).map_err(|e| e.to_string())?;
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => file
                .write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| e.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("无法释放资源 {name}：{error}")),
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // Allow reads/execution, deny modification and deletion.
        }
        let mut file = options
            .open(&path)
            .map_err(|e| format!("无法读取资源 {name}：{e}"))?;
        if file.metadata().map_err(|e| e.to_string())?.len() != bytes.len() as u64 {
            return Err(format!(
                "资源校验失败：{name}。请保留安全软件报告，不要关闭防护。"
            ));
        }
        let mut actual = Vec::with_capacity(bytes.len());
        file.read_to_end(&mut actual).map_err(|e| e.to_string())?;
        if actual != *bytes {
            return Err(format!(
                "资源校验失败：{name}。请保留安全软件报告，不要关闭防护。"
            ));
        }
        locks.push(file);
    }
    Ok(locks)
}

fn main() {
    if let Err(error) = run() {
        if std::env::args().any(|arg| arg == "--verify-bundle") {
            eprintln!("{error}");
        } else {
            dji4g_windows_platform::show_message_box("大疆 4G 面板启动失败", &error);
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    if FILES.is_empty() || BUNDLE_ID.len() != 64 {
        return Err("此构建未包含独立运行资源。".into());
    }
    let base = std::env::var_os("LOCALAPPDATA").ok_or("无法读取当前用户的应用数据目录")?;
    let directory = Path::new(&base)
        .join("Dji4GPanel/standalone")
        .join(BUNDLE_ID);
    let _locks = prepare_files(&directory, FILES)?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--verify-bundle" {
        println!(
            "Verified {} resources: {}",
            FILES.len(),
            directory.display()
        );
        return Ok(());
    }
    let status = Command::new(directory.join("dji4g-panel.exe"))
        .args(args)
        .current_dir(&directory)
        .status()
        .map_err(|e| format!("无法打开面板：{e}。如有安全软件拦截，请保留报告。"))?;
    if !status.success() {
        return Err(format!(
            "面板异常退出：{status}。请保留安全软件报告及程序日志。"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dji4g-portable-test-{}-{stamp}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn extracts_and_reuses_exact_payload_with_unicode_paths() {
        let root = scratch().join("中文 空格");
        let files: &[(&str, &[u8])] = &[("panel.exe", b"panel"), ("drivers/a.inf", b"driver")];
        let locks = prepare_files(&root, files).unwrap();
        assert_eq!(
            std::fs::read(root.join("drivers/a.inf")).unwrap(),
            b"driver"
        );
        assert_eq!(locks.len(), 2);
        drop(locks);
        assert_eq!(prepare_files(&root, files).unwrap().len(), 2);
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_changed_executable_instead_of_launching_it() {
        let root = scratch();
        std::fs::write(root.join("panel.exe"), b"changed").unwrap();
        assert!(prepare_files(&root, &[("panel.exe", b"expected")]).is_err());
        assert_eq!(std::fs::read(root.join("panel.exe")).unwrap(), b"changed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_paths_outside_resource_directory() {
        let root = scratch();
        for name in ["../escape.exe", "C:/escape.exe", "/escape.exe"] {
            assert!(prepare_files(&root, &[(name, b"bad")]).is_err(), "{name}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
