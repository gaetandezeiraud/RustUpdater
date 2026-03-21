use std::thread;
use std::time::Duration;
use sysinfo::System;
use updater::models::Manifest;

/// Checks if a specific executable is currently running
pub(crate) fn is_process_running(exe_name: &str) -> bool {
    let mut sys = System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let target_name = exe_name.to_lowercase();

    for process in sys.processes().values() {
        let process_name = process.name().to_string_lossy().to_lowercase();
        if process_name == target_name || process_name == format!("{}.exe", target_name) {
            return true;
        }
    }
    false
}

/// Helper to read the local manifest and get the exe name
pub(crate) fn get_local_exe_name(product_dir: &std::path::Path) -> Option<String> {
    let manifest_path = product_dir.join("manifest.json");
    if let Ok(data) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = serde_json::from_str::<Manifest>(&data) {
            if !manifest.exe.is_empty() {
                return Some(manifest.exe);
            }
        }
    }
    None
}

/// Helper to kill a proc
pub(crate) fn kill_process(exe_name: &str) -> bool
{
    let mut sys = System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let target_name = exe_name.to_lowercase();
    let mut killed_any = false;

    for process in sys.processes().values() {
        let process_name = process.name().to_string_lossy().to_lowercase();
        if process_name == target_name || process_name == format!("{}.exe", target_name) {
            if process.kill() {
                process.wait(); // Wait for OS confirmation the process is completely dead
                killed_any = true;
            }
        }
    }

    // Give Windows and Antivirus a moment to release the file handle
    if killed_any {
        thread::sleep(Duration::from_millis(1500));
    }

    killed_any
}