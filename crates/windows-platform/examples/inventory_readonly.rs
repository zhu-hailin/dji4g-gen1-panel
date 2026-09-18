//! Read-only native enumeration smoke test: no serial commands or device changes.
use dji4g_windows_platform::WindowsDeviceInventory;

fn main() {
    match WindowsDeviceInventory.scan_now() {
        Ok(snapshot) => println!("{snapshot:#?}"),
        Err(error) => {
            eprintln!("{error:?}");
            std::process::exit(1);
        }
    }
}
