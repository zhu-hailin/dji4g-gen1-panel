//! Windows resource embedding: the brand icon on the executable itself (Explorer, taskbar,
//! shortcuts).  The MSVC resource compiler is looked up by `winresource`; a checkout without it
//! still builds — the executable just keeps no embedded icon.

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/brand/logo.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=brand icon not embedded: {error}");
        }
    }
}
