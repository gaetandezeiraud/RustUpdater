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
use std::sync::Mutex;
use serde::Serialize;
use updater::models::RootJson;
use updater::ProductUpdater;

pub(crate) struct UpdaterConfig {
    pub(crate) server_url: Mutex<String>,
    pub(crate) install_dir: Mutex<PathBuf>,
}

#[derive(Clone, Serialize)]
pub(crate) struct ProgressPayload {
    pub(crate) current: usize,
    pub(crate) total: usize,
    pub(crate) percent: f64,
}

#[derive(Serialize)]
pub(crate) struct ProductState {
    latest_version: String,
    manifest: String,
    versions: Vec<String>,
    local_version: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct AppStateResponse {
    products: std::collections::BTreeMap<String, ProductState>,
    offline: bool,
}

pub(crate) fn build_app_state_response(updater: &ProductUpdater, root: RootJson, is_offline: bool) -> AppStateResponse {
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