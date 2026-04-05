/*
MIT License
Copyright (c) 2026 Gaetan Dezeiraud
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
use anyhow::Result;
use std::path::PathBuf;
use updater::models::{Manifest, RootJson};
use updater::ProductUpdater;

pub struct UpdateManager {
    updater: ProductUpdater,
    install_dir: PathBuf,
}

impl UpdateManager {
    pub fn new(base_url: &str, install_dir: impl Into<PathBuf>) -> Self {
        let dir: PathBuf = install_dir.into();
        Self {
            updater: ProductUpdater::new(base_url, &dir),
            install_dir: dir,
        }
    }

    // Local state queries
    pub fn get_local_version(&self, product: &str) -> Option<String> {
        self.updater.get_local_version(product)
    }
    pub fn get_cached_root(&self) -> RootJson {
        self.updater.get_cached_root()
    }
    // Remote data access
    pub async fn fetch_root(&self) -> Result<(RootJson, bool)> {
        self.updater.fetch_root().await
    }
    pub async fn fetch_manifest(&self, product_name: &str, version: &str) -> Result<Manifest> {
        self.updater.fetch_manifest(product_name, version).await
    }

    // Orchestrated operations
    /// Fetch the required manifests, decide the update strategy, drive the engine,
    /// and persist manifest.json on success.
    pub async fn run_update<F>(
        &self,
        product_name: &str,
        target_version: &str,
        available_versions: &[String],
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(usize, usize) + Send + Sync + Clone + 'static,
    {
        let current_version = self.updater
            .get_local_version(product_name)
            .unwrap_or_else(|| "0.0.0".to_string());
        if current_version == target_version {
            return Ok(());
        }
        let update_path = Self::compute_update_path(&current_version, target_version, available_versions);
        let target_manifest = self.updater.fetch_manifest(product_name, target_version).await?;
        let mut path_manifests: Vec<Manifest> = Vec::new();
        for ver in &update_path {
            let manifest = if ver == target_version {
                target_manifest.clone()
            } else {
                self.updater.fetch_manifest(product_name, ver).await?
            };
            path_manifests.push(manifest);
        }
        // Patch only if cheaper than a full download; 0 cost means no patches exist.
        let full_size: u64 = target_manifest.files.values().map(|e| e.size).sum();
        let total_patch_cost: u64 = path_manifests.iter().map(|m| m.total_patch_size).sum();
        let update_manifests = if total_patch_cost > 0 && total_patch_cost < full_size {
            path_manifests
        } else {
            vec![]
        };
        self.updater.perform_update(product_name, &target_manifest, update_manifests, on_progress).await?;
        // Persist the final manifest so the launcher can locate the executable.
        let product_dir = self.install_dir.join(product_name);
        if let Ok(json) = serde_json::to_string_pretty(&target_manifest) {
            if let Err(e) = std::fs::write(product_dir.join("manifest.json"), json) {
                eprintln!("Warning: update succeeded but failed to save manifest.json: {}", e);
            }
        }
        Ok(())
    }

    /// Fetch the manifest then drive the repair engine.
    pub async fn repair<F>(
        &self,
        product_name: &str,
        version: &str,
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(usize, usize) + Send + Sync + 'static,
    {
        let manifest = self.updater.fetch_manifest(product_name, version).await?;
        self.updater.repair_installation(product_name, &manifest, on_progress).await
    }

    fn compute_update_path(current: &str, target: &str, available: &[String]) -> Vec<String> {
        if let Some(cur_idx) = available.iter().position(|v| v == current) {
            if let Some(tgt_idx) = available.iter().position(|v| v == target) {
                if cur_idx < tgt_idx {
                    return available[cur_idx + 1..=tgt_idx].to_vec();
                }
            }
        }
        // Fresh install or downgrade: go directly to the target version.
        vec![target.to_string()]
    }
}
