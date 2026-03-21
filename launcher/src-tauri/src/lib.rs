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