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
use tauri_plugin_log::log::{debug, error, info, warn};
use updater::models::Manifest;
use updater::ProductUpdater;
use crate::process::{get_local_exe_name, is_process_running, kill_process};
use crate::state::{build_app_state_response, AppStateResponse, ProgressPayload, UpdaterConfig};

/// Helper to create a standardized progress callback for the updater
fn create_progress_callback(app: AppHandle, product_name: String) -> impl Fn(usize, usize) + Send + Sync + Clone + 'static {
    move |current: usize, total: usize| {
        let percent = if total > 0 {
            (current as f64 / total as f64) * 100.0
        } else {
            100.0
        };
        let payload = ProgressPayload { product_name: product_name.clone(), current, total, percent };
        let _ = app.emit("progress", payload);
    }
}

#[tauri::command]
pub(crate) async fn get_cached_app_state(state: tauri::State<'_, UpdaterConfig>) -> Result<AppStateResponse, String> {
    let url = state.server_url.lock().map_err(|_| {
        error!("Failed to lock server_url mutex in get_cached_app_state");
        "Internal state error".to_string()
    })?.clone();

    let dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex in get_cached_app_state");
        "Internal state error".to_string()
    })?.clone();

    let updater = ProductUpdater::new(&url, dir);
    let root = updater.get_cached_root();

    Ok(build_app_state_response(&updater, root, true))
}

#[tauri::command]
pub(crate) async fn get_app_state(state: tauri::State<'_, UpdaterConfig>) -> Result<AppStateResponse, String> {
    let url = state.server_url.lock().map_err(|_| {
        error!("Failed to lock server_url mutex in get_app_state");
        "Internal state error".to_string()
    })?.clone();

    let dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex in get_app_state");
        "Internal state error".to_string()
    })?.clone();

    let updater = ProductUpdater::new(&url, dir);
    let (root, is_offline) = updater.fetch_root().await.map_err(|e| {
        warn!("Failed to fetch root state: {}", e);
        e.to_string()
    })?;

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
    let url = state.server_url.lock().map_err(|_| {
        error!("Failed to lock server_url mutex");
        "Internal state error".to_string()
    })?.clone();

    let dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex");
        "Internal state error".to_string()
    })?.clone();

    let product_dir = dir.join(&product_name);

    // Is running?
    if let Some(exe_name) = get_local_exe_name(&product_dir) {
        if is_process_running(&exe_name) {
            warn!("Attempted to update {} while it was running", product_name);
            return Err(format!("Cannot update: {} is currently running. Please close it first.", product_name));
        }
    }

    // Setup cancellation channel
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut lock = state.cancel_tx.lock().map_err(|_| "Internal state error".to_string())?;
        *lock = Some(cancel_tx);
    }

    let updater = ProductUpdater::new(&url, dir.clone());
    info!("Starting update for {} to v{}...", product_name, target_version);

    let progress_callback = create_progress_callback(app.clone(), product_name.clone());
    let temp_dir = dir.join(".temp");

    let update_future = updater.perform_update(&product_name, &target_version, &available_versions, progress_callback);

    tokio::select! {
        result = update_future => {
            let _ = state.cancel_tx.lock().map(|mut l| l.take());
            match result {
                Ok(_) => {
                    if let Some(exe_name) = get_local_exe_name(&product_dir) {
                        crate::process::add_windows_registry(&product_name, &product_dir, &exe_name, &target_version);
                        crate::process::create_start_menu_shortcut(&product_name, &product_dir, &exe_name);
                    }
                    info!("Update finished successfully for {}!", product_name);
                    Ok("Success".into())
                }
                Err(e) => {
                    error!("Update failed for {}: {}", product_name, e);
                    Err(format!("Update failed: {}", e))
                }
            }
        }
        _ = cancel_rx => {
            warn!("Installation for {} was cancelled by user.", product_name);
            let _ = std::fs::remove_dir_all(&temp_dir);
            // Fresh install (no version.json yet) => clean up the partial product directory.
            // Update (version.json already exists) => leave the existing install untouched.
            // But it is not possible to cancel an update.
            if !product_dir.join("version.json").exists() {
                let _ = std::fs::remove_dir_all(&product_dir);
            }
            Err("CANCELLED".to_string())
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
    let url = state.server_url.lock().map_err(|_| {
        error!("Failed to lock server_url mutex in repair_installation");
        "Internal state error".to_string()
    })?.clone();

    let dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex in repair_installation");
        "Internal state error".to_string()
    })?.clone();

    let product_dir = dir.join(&product_name);

    // Is running?
    if let Some(exe_name) = crate::process::get_local_exe_name(&product_dir) {
        if crate::process::is_process_running(&exe_name) {
            warn!("Attempted to repair {} while it was running", product_name);
            return Err(format!("Cannot repair: {} is currently running. Please close it first.", product_name));
        }
    }

    // Setup cancellation channel
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut lock = state.cancel_tx.lock().map_err(|_| "Internal state error".to_string())?;
        *lock = Some(cancel_tx);
    }

    let updater = ProductUpdater::new(&url, dir.clone());
    info!("Scanning and repairing files for {} v{}...", product_name, version);

    let progress_callback = create_progress_callback(app.clone(), product_name.clone());
    let temp_dir = dir.join(".temp");

    let repair_future = updater.repair_installation(&product_name, &version, progress_callback);

    tokio::select! {
        result = repair_future => {
            let _ = state.cancel_tx.lock().map(|mut l| l.take());
            match result {
                Ok(_) => {
                    info!("Repair complete! All files for {} are now 100% correct.", product_name);
                    Ok("Success".to_string())
                }
                Err(e) => {
                    error!("Repair failed for {}: {}", product_name, e);
                    Err(format!("Repair failed: {}", e))
                }
            }
        }
        _ = cancel_rx => {
            warn!("Repair for {} was cancelled by user.", product_name);
            let _ = std::fs::remove_dir_all(&temp_dir);
            Err("CANCELLED".to_string())
        }
    }
}

/// Cancel the currently running update or repair operation
#[tauri::command]
pub(crate) async fn cancel_update(
    state: tauri::State<'_, UpdaterConfig>,
) -> Result<String, String> {
    let tx = state.cancel_tx.lock()
        .map_err(|_| "Internal state error".to_string())?
        .take();

    if let Some(tx) = tx {
        let _ = tx.send(());
        info!("Cancel signal sent to active operation.");
        Ok("Cancel signal sent".into())
    } else {
        warn!("cancel_update called but no active operation found.");
        Err("No active operation to cancel".into())
    }
}

#[tauri::command]
pub(crate) async fn launch_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let url = state.server_url.lock().map_err(|_| {
        error!("Failed to lock server_url mutex");
        "Internal state error".to_string()
    })?.clone();

    let dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex");
        "Internal state error".to_string()
    })?.clone();

    let product_dir = dir.join(&product_name);
    let manifest_path = product_dir.join("manifest.json");

    let updater = ProductUpdater::new(&url, &dir);

    let local_ver = updater.get_local_version(&product_name).ok_or_else(|| {
        warn!("Attempted to launch {} but it is not installed (version.json missing).", product_name);
        "Product is not installed (version.json missing)."
    })?;

    let manifest = if manifest_path.exists() {
        // Read the local manifest instantly
        let manifest_data = std::fs::read_to_string(&manifest_path).map_err(|e| {
            error!("Failed to read local manifest for {}: {}", product_name, e);
            format!("Failed to read local manifest: {}", e)
        })?;

        let local_manifest: Manifest = serde_json::from_str(&manifest_data).map_err(|e| {
            error!("Failed to parse local manifest for {}: {}", product_name, e);
            format!("Failed to parse local manifest: {}", e)
        })?;

        local_manifest
    } else {
        // Missing local manifest, fetch it, block the launch, and save it
        info!("Local manifest missing for {}, fetching from server...", product_name);
        let fetched_manifest = updater.fetch_manifest(&product_name, &local_ver).await.map_err(|e| {
            error!("Offline manifest missing, and failed to fetch from server for {}: {}", product_name, e);
            format!("Offline manifest missing, and failed to fetch from server: {}", e)
        })?;

        if let Ok(manifest_json) = serde_json::to_string_pretty(&fetched_manifest) {
            if let Err(e) = std::fs::write(&manifest_path, manifest_json) {
                warn!("Fetched manifest for {}, but failed to save it locally: {}", product_name, e);
            }
        }

        fetched_manifest
    };

    if manifest.exe.is_empty() {
        error!("Manifest for {} specifies no executable", product_name);
        return Err("No executable specified in the manifest.".into());
    }

    // Launch the executable
    let exe_path = product_dir.join(&manifest.exe);

    if !exe_path.exists() {
        error!("Executable not found at: {}", exe_path.display());
        return Err(format!("Executable not found at: {}", exe_path.display()));
    }

    let parent_dir = exe_path.parent().ok_or_else(|| {
        error!("Invalid executable path (no parent directory): {}", exe_path.display());
        "Invalid executable path".to_string()
    })?;

    info!("Launching {} from {}...", product_name, exe_path.display());
    Command::new(&exe_path)
        .current_dir(parent_dir)
        .spawn()
        .map_err(|e| {
            error!("Failed to spawn process for {}: {}", product_name, e);
            format!("Failed to launch product: {}", e)
        })?;

    Ok("Launched successfully!".into())
}

#[tauri::command]
pub(crate) async fn uninstall_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let install_dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex in uninstall_product");
        "Internal state error".to_string()
    })?.clone();

    let product_dir = install_dir.join(&product_name);

    // Is running?
    if let Some(exe_name) = get_local_exe_name(&product_dir) {
        if is_process_running(&exe_name) {
            warn!("Attempted to uninstall {} while it was running", product_name);
            return Err(format!("Cannot uninstall: {} is currently running. Please close it first.", product_name));
        }
    }

    info!("Uninstalling {}...", product_name);

    // Clean up Windows registry and shortcuts
    crate::process::remove_windows_registry(&product_name);
    crate::process::remove_start_menu_shortcut(&product_name);

    if product_dir.exists() {
        std::fs::remove_dir_all(&product_dir).map_err(|e| {
            error!("Failed to remove directory for {}: {}", product_name, e);
            format!("Failed to uninstall directory: {}", e)
        })?;
        info!("Uninstalled {} successfully", product_name);
        Ok("Uninstalled successfully".into())
    } else {
        warn!("Attempted to uninstall {}, but the directory does not exist", product_name);
        Err("Product is not installed".into())
    }
}

/// Kill a process
#[tauri::command]
pub(crate) async fn force_kill_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let dir = state.install_dir.lock().map_err(|_| {
        error!("Failed to lock install_dir mutex in force_kill_product");
        "Internal state error".to_string()
    })?.clone();

    let product_dir = dir.join(&product_name);

    let exe_name = get_local_exe_name(&product_dir).ok_or_else(|| {
        warn!("Could not find manifest to determine executable name for force kill on {}", product_name);
        "Could not find manifest to determine executable name.".to_string()
    })?;

    info!("Attempting to force kill process: {}", exe_name);
    if kill_process(&exe_name) {
        info!("Process {} killed successfully", exe_name);
        Ok("Process killed successfully".into())
    } else {
        warn!("Could not find or kill the process {} (it might have already closed)", exe_name);
        Err("Could not find or kill the process (it might have already closed).".into())
    }
}

/// Look at startup params
#[tauri::command]
pub(crate) fn get_startup_intent() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();

    if let Some(index) = args.iter().position(|arg| arg == "--uninstall") {
        if index + 1 < args.len() {
            debug!("Startup intent detected: Uninstalling {}", args[index + 1]);
            return Some(args[index + 1].clone());
        }
    }
    None
}