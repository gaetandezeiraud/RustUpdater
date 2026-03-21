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
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use tauri::{Emitter, AppHandle};
use updater::ProductUpdater;
use serde::Serialize;
use sysinfo::System;

struct UpdaterConfig {
    server_url: Mutex<String>,
    install_dir: Mutex<PathBuf>,
}

#[derive(Clone, Serialize)]
struct ProgressPayload {
    current: usize,
    total: usize,
    percent: f64,
}

#[derive(Serialize)]
struct ProductState {
    latest_version: String,
    manifest: String,
    versions: Vec<String>,
    local_version: Option<String>,
}

#[derive(Serialize)]
struct AppStateResponse {
    products: std::collections::BTreeMap<String, ProductState>,
    offline: bool,
}

/// Checks if a specific executable is currently running
fn is_process_running(exe_name: &str) -> bool {
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
fn get_local_exe_name(product_dir: &std::path::Path) -> Option<String> {
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

/// Validates the server URL, ensuring it ends with '/'
#[tauri::command]
fn validate_server_url(mut url: String) -> Result<String, String> {
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

fn build_app_state_response(updater: &ProductUpdater, root: RootJson, is_offline: bool) -> AppStateResponse {
    let mut products_state = std::collections::BTreeMap::new();

    for (name, entry) in root.products {
        let local_ver = updater.get_local_version(&name);

        products_state.insert(name, ProductState {
            latest_version: entry.latest_version,
            manifest: entry.manifest,
            versions: entry.versions,
            local_version: local_ver,
        });
    }

    AppStateResponse {
        products: products_state,
        offline: is_offline,
    }
}

#[tauri::command]
async fn get_cached_app_state(state: tauri::State<'_, UpdaterConfig>) -> Result<AppStateResponse, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();

    let updater = ProductUpdater::new(&url, dir);
    let root = updater.get_cached_root();

    Ok(build_app_state_response(&updater, root, true))
}

#[tauri::command]
async fn get_app_state(state: tauri::State<'_, UpdaterConfig>) -> Result<AppStateResponse, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();

    let updater = ProductUpdater::new(&url, dir);
    let (root, is_offline) = updater.fetch_root().await.map_err(|e| e.to_string())?;

    Ok(build_app_state_response(&updater, root, is_offline))
}

#[tauri::command]
async fn run_update(
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
async fn verify_integrity(
    app: AppHandle,
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
    version: String,
) -> Result<Vec<String>, String> {
    let url = state.server_url.lock().unwrap().clone();
    let dir = state.install_dir.lock().unwrap().clone();

    let updater = ProductUpdater::new(&url, dir);
    let _ = app.emit("log", format!("Verifying files for {} v{}...", product_name, version));

    let app_clone = app.clone();
    let progress_callback = move |current: usize, total: usize| {
        let percent = if total > 0 { (current as f64 / total as f64) * 100.0 } else { 100.0 };
        let payload = ProgressPayload { current, total, percent };
        let _ = app_clone.emit("progress", payload);
    };

    match updater.verify_integrity(&product_name, &version, progress_callback).await {
        Ok(corrupted) => {
            if corrupted.is_empty() {
                let _ = app.emit("log", "Integrity check passed! All files 100% correct.".to_string());
            } else {
                let _ = app.emit("log", format!("CRITICAL: Found {} corrupted files.", corrupted.len()));
            }
            Ok(corrupted)
        }
        Err(e) => Err(e.to_string()),
    }
}

use updater::models::{Manifest, RootJson}; // Make sure you have this import at the top

#[tauri::command]
async fn launch_product(
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
async fn uninstall_product(
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
async fn force_kill_product(
    state: tauri::State<'_, UpdaterConfig>,
    product_name: String,
) -> Result<String, String> {
    let dir = state.install_dir.lock().unwrap().clone();
    let product_dir = dir.join(&product_name);

    let exe_name = get_local_exe_name(&product_dir)
        .ok_or_else(|| "Could not find manifest to determine executable name.".to_string())?;

    let mut sys = System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let target_name = exe_name.to_lowercase();
    let mut killed_any = false;

    for process in sys.processes().values() {
        let process_name = process.name().to_string_lossy().to_lowercase();
        if process_name == target_name || process_name == format!("{}.exe", target_name) {
            if process.kill() {
                killed_any = true;
            }
        }
    }

    if killed_any {
        Ok("Process killed successfully".into())
    } else {
        Err("Could not find or kill the process (it might have already closed).".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let default_url = "http://192.168.1.29:3000/".to_string();
    let default_install_dir = std::env::current_dir().unwrap().join("products");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(UpdaterConfig {
            server_url: Mutex::new(default_url),
            install_dir: Mutex::new(default_install_dir),
        })
        .invoke_handler(tauri::generate_handler![
            validate_server_url,
            get_cached_app_state,
            get_app_state,
            run_update,
            verify_integrity,
            launch_product,
            uninstall_product,
            force_kill_product
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}