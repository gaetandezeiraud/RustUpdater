/*
MIT License

Copyright (c) 2026 Gaëtan Dezeiraud

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
use std::path::Path;
use std::thread;
use std::env;
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

// Windows features
pub(crate) fn add_windows_registry(product_name: &str, product_dir: &Path, exe_name: &str, version: &str) {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}", product_name);

        if let Ok((key, _)) = hkcu.create_subkey(&path) {
            if let Ok(launcher_exe) = env::current_exe() {
                let base_dir = product_dir.parent().unwrap_or(product_dir);
                let uninstall_string = format!("\"{}\" --uninstall \"{}\" --dir \"{}\"",
                                               launcher_exe.display(),
                                               product_name,
                                               base_dir.display()
                );

                let exe_path = product_dir.join(exe_name);

                let _ = key.set_value("DisplayName", &product_name);
                let _ = key.set_value("DisplayVersion", &version);
                let _ = key.set_value("InstallLocation", &product_dir.to_string_lossy().into_owned());
                let _ = key.set_value("DisplayIcon", &exe_path.to_string_lossy().into_owned());
                let _ = key.set_value("UninstallString", &uninstall_string);
                let _ = key.set_value("Publisher", &"EDITOR"); // Todo: Adapt for your use case
            }
        }
    }
}

pub(crate) fn remove_windows_registry(product_name: &str) {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}", product_name);
        let _ = hkcu.delete_subkey_all(&path);
    }
}

pub(crate) fn create_start_menu_shortcut(product_name: &str, product_dir: &Path, exe_name: &str) {
    #[cfg(target_os = "windows")]
    {
        // dirs::data_dir() resolves to AppData\Roaming, where the Start Menu lives
        if let Some(mut start_menu) = dirs::data_dir() {
            start_menu.push("Microsoft\\Windows\\Start Menu\\Programs");
            if start_menu.exists() {
                let shortcut_path = start_menu.join(format!("{}.lnk", product_name));
                let target = product_dir.join(exe_name);

                let _ = mslnk::ShellLink::new(target.to_string_lossy().as_ref())
                    .and_then(|mut lnk| {
                        lnk.set_working_dir(Some(product_dir.to_string_lossy().into_owned()));
                        lnk.create_lnk(&shortcut_path)
                    });
            }
        }
    }
}

pub(crate) fn remove_start_menu_shortcut(product_name: &str) {
    #[cfg(target_os = "windows")]
    {
        if let Some(mut start_menu) = dirs::data_dir() {
            start_menu.push("Microsoft\\Windows\\Start Menu\\Programs");
            let shortcut_path = start_menu.join(format!("{}.lnk", product_name));
            if shortcut_path.exists() {
                let _ = std::fs::remove_file(shortcut_path);
            }
        }
    }
}