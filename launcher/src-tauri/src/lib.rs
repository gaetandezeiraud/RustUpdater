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
pub mod state;
pub mod process;
pub mod commands;

use std::sync::Mutex;
use state::UpdaterConfig;
use commands::*;
use tauri::{Manager, Emitter};
use tauri_plugin_log::log::LevelFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let default_url = "http://192.168.1.29:3000/".to_string();
    let mut default_install_dir = std::env::current_dir().unwrap().join("products");

    // Intercept the hidden --dir argument from Windows Add/Remove programs
    // Useful only if default_install_dir is relative and not absolute
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--dir") {
        if let Some(dir_path) = args.get(index + 1) {
            default_install_dir = std::path::PathBuf::from(dir_path);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().level(LevelFilter::Info).build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Bring the existing launcher window to the front
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }

            // Check if the second instance was trying to uninstall something
            if let Some(index) = args.iter().position(|arg| arg == "--uninstall") {
                if index + 1 < args.len() {
                    let product_name = &args[index + 1];
                    let _ = app.emit("uninstall-intent", product_name);
                }
            }
        }))
        .manage(UpdaterConfig {
            server_url: Mutex::new(default_url),
            install_dir: Mutex::new(default_install_dir),
        })
        .invoke_handler(tauri::generate_handler![
            get_startup_intent,
            get_cached_app_state,
            get_app_state,
            run_update,
            repair_installation,
            launch_product,
            uninstall_product,
            force_kill_product
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}