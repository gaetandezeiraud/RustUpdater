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