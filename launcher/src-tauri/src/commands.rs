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
use std::process::Command;
use tauri::{AppHandle, Emitter};
use updater::models::Manifest;
use updater::ProductUpdater;
use crate::process::{get_local_exe_name, is_process_running, kill_process};
use crate::state::{build_app_state_response, AppStateResponse, ProgressPayload, UpdaterConfig};

/// Validates the server URL, ensuring it ends with '/'
#[tauri::command]
pub(crate) fn validate_server_url(mut url: String) -> Result<String, String> {
    if url.trim().is_empty() {
        return Err(String::from("server url is empty"));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".into());
    }
    if !url.ends_with('/') {
        url.push('/');
    }
    Ok(url)
}

#[tauri::command]
pub(crate) async fn get_cached_app_state(state: tauri::State<'_, UpdaterConfig>) -> Result<AppStateResponse, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();

    let updater = ProductUpdater::new(&url, dir);
    let root = updater.get_cached_root();

    Ok(build_app_state_response(&updater, root, true))
}

#[tauri::command]
pub(crate) async fn get_app_state(state: tauri::State<'_, UpdaterConfig>) -> Result<AppStateResponse, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();

    let updater = ProductUpdater::new(&url, dir);
    let (root, is_offline) = updater.fetch_root().await.map_err(|e| e.to_string())?;

    Ok(build_app_state_response(&updater, root, is_offline))
}

#[tauri::command]
pub(crate) async fn run_update(
    app: AppHandle,
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
    target_version: String,
    available_versions: Vec<String>,
) -> Result<String, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();
    let product_dir = dir.join(&product_name);

    // Is running?
    if let Some(exe_name) = get_local_exe_name(&product_dir) {
        if is_process_running(&exe_name) {
            return Err(format!("Cannot update: {} is currently running. Please close it first.", product_name));
        }
    }

    let updater = ProductUpdater::new(&url, dir);
    let _ = app.emit("log", format!("Starting update for {} to v{}...", product_name, target_version));

    let app_clone = app.clone();

    let progress_callback = move |current: usize, total: usize| {
        let percent = if total > 0 { (current as f64 / total as f64) * 100.0 } else { 100.0 };
        let payload = ProgressPayload { current, total, percent };
        let _ = app_clone.emit("progress", payload);
    };

    match updater.perform_update(&product_name, &target_version, &available_versions, progress_callback).await {
        Ok(_) => {
            // Create Windows registry entries and shortcuts
            if let Some(exe_name) = get_local_exe_name(&product_dir) {
                crate::process::add_windows_registry(&product_name, &product_dir, &exe_name, &target_version);
                crate::process::create_start_menu_shortcut(&product_name, &product_dir, &exe_name);
            }

            let _ = app.emit("log", "Update finished successfully!".to_string());
            Ok("Success".into())
        }
        Err(e) => {
            let err_msg = format!("Update failed: {}", e);
            let _ = app.emit("log", err_msg.clone());
            Err(err_msg)
        }
    }
}

#[tauri::command]
pub(crate) async fn repair_installation(
    app: AppHandle,
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
    version: String,
) -> Result<String, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();
    let product_dir = dir.join(&product_name);

    // Is running?
    if let Some(exe_name) = crate::process::get_local_exe_name(&product_dir) {
        if crate::process::is_process_running(&exe_name) {
            return Err(format!("Cannot repair: {} is currently running. Please close it first.", product_name));
        }
    }

    let updater = ProductUpdater::new(&url, dir);
    let _ = app.emit("log", format!("Scanning and repairing files for {} v{}...", product_name, version));

    let app_clone = app.clone();
    let progress_callback = move |current: usize, total: usize| {
        let percent = if total > 0 { (current as f64 / total as f64) * 100.0 } else { 100.0 };
        let payload = ProgressPayload { current, total, percent };
        let _ = app_clone.emit("progress", payload);
    };

    match updater.repair_installation(&product_name, &version, progress_callback).await {
        Ok(_) => {
            let _ = app.emit("log", "Repair complete! All files are now 100% correct.".to_string());
            Ok("Success".to_string())
        }
        Err(e) => {
            let err_msg = format!("Repair failed: {}", e);
            let _ = app.emit("log", err_msg.clone());
            Err(err_msg)
        }
    }
}

#[tauri::command]
pub(crate) async fn launch_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();
    let product_dir = dir.join(&product_name);
    let manifest_path = product_dir.join("manifest.json");

    let updater = ProductUpdater::new(&url, &dir);

    let local_ver = updater.get_local_version(&product_name)
        .ok_or("Product is not installed (version.json missing).")?;

    let manifest = if manifest_path.exists() {
        // Read the local manifest instantly
        let manifest_data = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read local manifest: {}", e))?;
        let local_manifest: Manifest = serde_json::from_str(&manifest_data)
            .map_err(|e| format!("Failed to parse local manifest: {}", e))?;

        local_manifest
    } else {
        // Missing local manifest, fetch it, block the launch, and save it
        let fetched_manifest = updater.fetch_manifest(&product_name, &local_ver).await
            .map_err(|e| format!("Offline manifest missing, and failed to fetch from server: {}", e))?;

        if let Ok(manifest_json) = serde_json::to_string_pretty(&fetched_manifest) {
            let _ = std::fs::write(&manifest_path, manifest_json);
        }

        fetched_manifest
    };

    if manifest.exe.is_empty() {
        return Err("No executable specified in the manifest.".into());
    }

    // Launch the executable
    let exe_path = product_dir.join(&manifest.exe);

    if !exe_path.exists() {
        return Err(format!("Executable not found at: {}", exe_path.display()));
    }

    Command::new(&exe_path)
        .current_dir(exe_path.parent().unwrap())
        .spawn()
        .map_err(|e| format!("Failed to launch product: {}", e))?;

    Ok("Launched successfully!".into())
}

#[tauri::command]
pub(crate) async fn uninstall_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let install_dir = state.install_dir.lock().unwrap().clone();
    let product_dir = install_dir.join(&product_name);

    // Is running?
    if let Some(exe_name) = get_local_exe_name(&product_dir) {
        if is_process_running(&exe_name) {
            return Err(format!("Cannot uninstall: {} is currently running. Please close it first.", product_name));
        }
    }

    // Clean up Windows registry and shortcuts
    crate::process::remove_windows_registry(&product_name);
    crate::process::remove_start_menu_shortcut(&product_name);

    if product_dir.exists() {
        std::fs::remove_dir_all(&product_dir)
            .map_err(|e| format!("Failed to uninstall directory: {}", e))?;
        Ok("Uninstalled successfully".into())
    } else {
        Err("Product is not installed".into())
    }
}

/// Kill a process
#[tauri::command]
pub(crate) async fn force_kill_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let dir = state.install_dir.lock().unwrap().clone();
    let product_dir = dir.join(&product_name);

    let exe_name = get_local_exe_name(&product_dir)
        .ok_or_else(|| "Could not find manifest to determine executable name.".to_string())?;

    if kill_process(&exe_name) {
        Ok("Process killed successfully".into())
    } else {
        Err("Could not find or kill the process (it might have already closed).".into())
    }
}

/// Look at startup params
#[tauri::command]
pub(crate) fn get_startup_intent() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();

    if let Some(index) = args.iter().position(|arg| arg == "--uninstall") {
        if index + 1 < args.len() {
            return Some(args[index + 1].clone());
        }
    }
    None
}