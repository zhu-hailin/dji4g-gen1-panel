//! Windows resource embedding: the brand icon on the executable itself (Explorer, taskbar,
//! shortcuts).  The MSVC resource compiler is looked up by `winresource`; a checkout without it
//! still builds — the executable just keeps no embedded icon.

fn main() {
    println!("cargo:rerun-if-env-changed=DJI4G_OFFLINE_ZIP");
    println!("cargo:rerun-if-env-changed=DJI4G_OFFLINE_SHA256");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let bundle = if let Some(path) = std::env::var_os("DJI4G_OFFLINE_ZIP") {
        println!(
            "cargo:rerun-if-changed={}",
            std::path::Path::new(&path).display()
        );
        let hash = std::env::var("DJI4G_OFFLINE_SHA256").expect("offline payload hash required");
        assert!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        let copied = out.join("offline-payload.zip");
        std::fs::copy(path, &copied).expect("copy offline payload");
        format!(
            "const PAYLOAD: &[u8] = include_bytes!({:?});\nconst HASH: &str = {:?};\n",
            copied, hash
        )
    } else {
        "const PAYLOAD: &[u8] = &[];\nconst HASH: &str = \"\";\n".to_owned()
    };
    std::fs::write(out.join("offline_bundle.rs"), bundle).unwrap();
    println!("cargo:rerun-if-env-changed=DJI4G_STANDALONE_DIR");
    println!("cargo:rerun-if-env-changed=DJI4G_STANDALONE_ID");
    let mut standalone = String::from("const FILES: &[(&str, &[u8])] = &[\n");
    let mut identity = String::new();
    if let Some(directory) = std::env::var_os("DJI4G_STANDALONE_DIR") {
        let directory = std::path::PathBuf::from(directory).canonicalize().unwrap();
        identity = std::env::var("DJI4G_STANDALONE_ID").expect("standalone identity required");
        assert!(identity.len() == 64 && identity.bytes().all(|b| b.is_ascii_hexdigit()));
        embed_directory(&directory, &directory, &mut standalone);
    }
    standalone.push_str(&format!("];\nconst BUNDLE_ID: &str = {identity:?};\n"));
    std::fs::write(out.join("standalone_bundle.rs"), standalone).unwrap();
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/brand/logo.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=brand icon not embedded: {error}");
        }
    }
}

fn embed_directory(root: &std::path::Path, dir: &std::path::Path, source: &mut String) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink(), "payload cannot contain symlinks");
        if kind.is_dir() {
            embed_directory(root, &path, source);
        } else {
            assert!(kind.is_file());
            println!("cargo:rerun-if-changed={}", path.display());
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            source.push_str(&format!("({name:?}, include_bytes!({path:?})),\n"));
        }
    }
}
