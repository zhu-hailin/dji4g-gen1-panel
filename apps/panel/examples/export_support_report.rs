//! Read-only collection smoke test. The snapshot is explicitly an empty test snapshot.
fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .expect("output directory required");
    let snapshot = dji4g_application::ReducerState::new(std::time::SystemTime::now()).snapshot();
    let path = dji4g_panel::support_report::collect(
        std::path::Path::new(&directory), &snapshot,
        "SMOKE TEST: empty synthetic snapshot; OS/device sections below are live read-only observations.",
        |text| println!("{text}"),
    ).expect("write report");
    println!("REPORT={}", path.display());
}
