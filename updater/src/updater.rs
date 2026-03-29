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
use crate::models::{FileEntry, Manifest, RootJson};
use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration};
use hdiffpatch_rs::patchers::HDiff;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Files larger than this are written to disk via streaming instead of being
/// loaded entirely into RAM first.
const STREAM_THRESHOLD: u64 = 30 * 1024 * 1024; // 30 MB

/// Maximum number of concurrent file operations.
const CONCURRENCY: usize = 8;

pub struct ProductUpdater {
    base_url: String,
    client: Client,
    install_dir: PathBuf,
}

impl ProductUpdater {
    pub fn new(base_url: &str, install_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_url: base_url.to_string(),
            client: Client::new(),
            install_dir: install_dir.into()
        }
    }

    /// Fetch the server's root manifest listing all available products.
    pub async fn fetch_root(&self) -> Result<(RootJson, bool)> {
        let url = format!("{}root.json", self.base_url);
        let cache_path = self.install_dir.join("root_cache.json");

        let _ = fs::create_dir_all(&self.install_dir);

        match self.client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                if let Ok(json) = response.json::<RootJson>().await {
                    // Save to cache for future offline use
                    if let Ok(str_data) = serde_json::to_string_pretty(&json) {
                        let _ = fs::write(&cache_path, str_data);
                    }
                    return Ok((json, false)); // false = not offline
                }
            }
            _ => {} // Network failed or non-200 status, fall through to cache
        }

        // Fallback to cache
        if cache_path.exists() {
            if let Ok(data) = fs::read_to_string(&cache_path) {
                if let Ok(json) = serde_json::from_str::<RootJson>(&data) {
                    return Ok((json, true)); // true = offline mode active
                }
            }
        }

        Err(anyhow::anyhow!("Failed to fetch root.json from server and no local cache is available."))
    }

    /// Read the locally installed version for a product, if any.
    pub fn get_local_version(&self, product: &str) -> Option<String> {
        let path = self.install_dir.join(product).join("version.json");
        let data = fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&data).ok()?;
        json["version"].as_str().map(str::to_string)
    }

    /// Fetch a specific version's manifest for a product.
    pub async fn fetch_manifest(&self, product_name: &str, version: &str) -> Result<Manifest> {
        let url = format!("{}products/{}/{}/manifest.json", self.base_url, product_name, version);
        self.client.get(&url).send().await?.json().await.context("Failed to parse manifest.json")
    }

    /// Instantly read the cached root manifest from disk, if it exists.
    pub fn get_cached_root(&self) -> RootJson {
        let cache_path = self.install_dir.join("root_cache.json");
        if let Ok(data) = fs::read_to_string(&cache_path) {
            if let Ok(json) = serde_json::from_str::<RootJson>(&data) {
                return json;
            }
        }
        RootJson::default() // Return empty if no cache exists, at very first launch
    }

    /// Initiates an update to a specific target version.
    /// It calculates the optimal sequential patch path based on the root.json versions array.
    pub async fn perform_update<F>(
        &self,
        product_name: &str,
        target_version: &str,
        available_versions: &[String],
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(usize, usize) + Send + Sync + Clone + 'static,
    {
        let current_version = self.get_local_version(product_name).unwrap_or_else(|| "0.0.0".to_string());
        if current_version == target_version { return Ok(()); } // Already up to date

        // Calculate the update path
        let mut update_path = Vec::new();
        if let Some(current_idx) = available_versions.iter().position(|v| v == &current_version) {
            if let Some(target_idx) = available_versions.iter().position(|v| v == target_version) {
                if current_idx < target_idx {
                    // Get all versions AFTER current up to the target
                    update_path = available_versions[current_idx + 1..=target_idx].to_vec();
                }
            }
        }

        // If sequential path calculation failed (e.g., fresh install or downgrade),
        // just target the final version directly for a full download.
        if update_path.is_empty() {
            update_path = vec![target_version.to_string()];
        }

        self.perform_update_path(product_name, &update_path, on_progress).await
    }

    /// Internal method that executes the determined update path
    async fn perform_update_path<F>(
        &self,
        product_name: &str,
        update_path: &[String],
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(usize, usize) + Send + Sync + Clone + 'static,
    {
        let target_version = update_path.last().expect("update_path is guaranteed non-empty");
        let target_manifest = self.fetch_manifest(product_name, target_version).await?;

        let product_dir = self.install_dir.join(product_name);
        fs::create_dir_all(&product_dir).context("Failed to create product directory")?;

        // Define and create a dynamic temp directory inside the install_dir
        let temp_dir = self.install_dir.join(".temp");
        fs::create_dir_all(&temp_dir).context("Failed to create temp directory")?;

        // Fetch all manifests in the update path
        let mut manifests = Vec::new();
        for ver in update_path {
            if ver == target_version {
                manifests.push(target_manifest.clone());
            } else {
                manifests.push(self.fetch_manifest(product_name, ver).await?);
            }
        }

        // Calculate the size of a complete full download of the target version
        let full_size: u64 = target_manifest.files.values().map(|e| e.size).sum();

        // Calculate the cumulative cost of intermediate patches using the new struct field
        let total_patch_cost: u64 = manifests.iter().map(|m| m.total_patch_size).sum();

        // Find the size of the largest single file
        let largest_file_size: u64 = target_manifest.files.values().map(|f| f.size).max().unwrap_or(0);

        // Check if space is available
        let is_installed = self.get_local_version(product_name).is_some();

        let required_space = if !is_installed {
            // Scenario A: Fresh install. We need the full size + 100MB buffer
            full_size + (100 * 1024 * 1024)
        } else if total_patch_cost > 0 && total_patch_cost < full_size {
            // Scenario B: Patching. We need space for the downloaded patches + room to write the largest temporary file
            total_patch_cost + largest_file_size + (100 * 1024 * 1024)
        } else {
            // Scenario C: Update falling back to full downloads.
            // It skips files that already exist, so we estimate space for the largest file to download + 1GB buffer.
            largest_file_size + (1024 * 1024 * 1024)
        };

        let dir = self.install_dir.clone();
        let available_space = tokio::task::spawn_blocking(move || {
            fs4::available_space(&dir)
        }).await.context("Thread panicked")?.context("Failed to read disk space")?;

        if available_space < required_space {
            return Err(anyhow::anyhow!(
                "INSUFFICIENT_SPACE:{}:{}",
                required_space,
                available_space
            ));
        }

        // Calculate total files for the progress bar across all manifests
        let total_files: usize = if total_patch_cost > 0 && total_patch_cost < full_size {
            manifests.iter().map(|m| m.files.len()).sum()
        } else {
            target_manifest.files.len()
        };

        let completed_files = Arc::new(AtomicUsize::new(0));

        // Evaluate strategy
        // We only patch if the total patch cost is strictly less than a full download.
        // Also if total_patch_cost is 0, it means no patches exist, so we force a full download.
        let update_result: Result<()> = async {
            if total_patch_cost > 0 && total_patch_cost < full_size {
                for manifest in manifests {
                    self.apply_manifest(product_name, &manifest, &product_dir, &temp_dir, true, completed_files.clone(), total_files, on_progress.clone()).await?;
                    // Save intermediate version progression in case of unexpected closure
                    Self::save_local_version(&product_dir, &manifest.version)?;
                    Self::save_local_manifest(&product_dir, &manifest)?;
                }
            } else {
                // Apply the final manifest directly with patching disabled to force a fresh download
                self.apply_manifest(product_name, &target_manifest, &product_dir, &temp_dir, false, completed_files, total_files, on_progress).await?;
                Self::save_local_version(&product_dir, target_version)?;
                Self::save_local_manifest(&product_dir, &target_manifest)?;
            }
            Ok(())
        }.await;

        let _ = fs::remove_dir_all(&temp_dir);

        update_result?;

        Ok(())
    }

    /// Process a single manifest concurrently
    async fn apply_manifest<F>(
        &self,
        product_name: &str,
        manifest: &Manifest,
        product_dir: &Path,
        temp_dir: &Path,
        allow_patch: bool,
        completed_files: Arc<AtomicUsize>,
        total_files: usize,
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(usize, usize) + Send + Sync + 'static,
    {
        // Handle file deletions first if the manifest specifies them
        for deleted_file in &manifest.deleted_files {
            let file_path = product_dir.join(deleted_file);
            if file_path.exists() {
                let _ = fs::remove_file(file_path);
            }
        }

        let owned_files = manifest.files.clone().into_iter();
        let on_progress = Arc::new(on_progress);

        let results = stream::iter(owned_files)
            .map(|(rel_path, file_entry)| {
                let client = self.client.clone();
                let base_url = self.base_url.clone();
                let product_name = product_name.to_string();
                let version = manifest.version.clone();
                let product_dir = product_dir.to_path_buf();
                let temp_dir = temp_dir.to_path_buf();

                // Clone our progress trackers for this specific async task
                let completed_clone = completed_files.clone();
                let prog_clone = Arc::clone(&on_progress);

                async move {
                    let res = update_file(&client, &base_url, &product_name, &version, &product_dir, &temp_dir, &rel_path, &file_entry, allow_patch).await;

                    // Atomically increment the completed files counter and trigger the callback
                    let current = completed_clone.fetch_add(1, Ordering::Relaxed) + 1;
                    prog_clone(current, total_files);

                    res
                }
            })
            .buffer_unordered(CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        for result in results { result?; } // Propagate the first error encountered, if any
        Ok(())
    }

    fn save_local_version(product_dir: &Path, version: &str) -> Result<()> {
        let version_json = serde_json::to_string_pretty(&serde_json::json!({ "version": version }))?;
        fs::write(product_dir.join("version.json"), version_json).context("Failed to write version.json")?;
        Ok(())
    }

    fn save_local_manifest(product_dir: &Path, manifest: &Manifest) -> Result<()> {
        let manifest_json = serde_json::to_string_pretty(manifest)?;
        fs::write(product_dir.join("manifest.json"), manifest_json).context("Failed to write manifest.json")?;
        Ok(())
    }

    /// Verify the integrity of a locally installed product against its manifest
    pub async fn repair_installation<F>(&self, product_name: &str, version: &str, on_progress: F) -> Result<()>
    where
        F: Fn(usize, usize) + Send + Sync + 'static,
    {
        let manifest = self.fetch_manifest(product_name, version).await?;
        let product_dir = self.install_dir.join(product_name);

        if !product_dir.exists() { return Err(anyhow::anyhow!("Product directory does not exist.")); }

        let temp_dir = self.install_dir.join(".temp");
        fs::create_dir_all(&temp_dir).context("Failed to create temp directory")?;

        let total_files = manifest.files.len();
        let completed_files = Arc::new(AtomicUsize::new(0));

        // Apply the manifest with patching disabled to force a fresh download of any corrupted files.
        // Skips files with a matching hash.
        let result = self.apply_manifest(product_name, &manifest, &product_dir, &temp_dir, false, completed_files, total_files, on_progress).await;

        // Always clean the temp dir
        let _ = fs::remove_dir_all(&temp_dir);

        result?;

        Ok(())
    }
}

/// Compute the BLAKE3 hash of a file on disk.
async fn file_hash_async(path: PathBuf) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let mut file = fs::File::open(&path).with_context(|| format!("Failed to open file for hashing: {}", path.display()))?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hasher.finalize().to_hex().to_string())
    })
        .await
        .context("Thread panicked during hashing")?
}

/// Apply an HDiffPatch binary patch using the native Rust library.
async fn apply_patch(old_path: PathBuf, patch_path: PathBuf, out_path: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let is_in_place = old_path == out_path;
        let actual_out_path = if is_in_place {
            // Append the tmp_patch, don't remove the existing file extension,
            // can be an issue with multithreading and files with same name (but different extension)
            let mut p = out_path.clone().into_os_string();
            p.push(".tmp_patch");
            PathBuf::from(p)
        } else {
            out_path.clone()
        };

        let old_str = old_path.to_string_lossy().to_string();
        let patch_str = patch_path.to_string_lossy().to_string();
        let out_str = actual_out_path.to_string_lossy().to_string();

        let mut patcher = HDiff::new(old_str, patch_str, out_str);
        let success = patcher.apply();

        if success && is_in_place {
            fs::rename(&actual_out_path, &out_path)
            .with_context(|| format!(
                "[apply_patch] Failed to rename temp file over original: {:?} -> {:?}",
                actual_out_path, out_path
            ))?;
        } else if !success && is_in_place {
            fs::remove_file(&actual_out_path)?;
        }

        Ok(())
    }).await.context("Patch task panicked")?
}

/// Download a URL and write it to `dest`.
async fn download_to(client: &Client, url: &str, dest: &Path, known_size: u64) -> Result<()> {
    let response = client.get(url).send().await.with_context(|| format!("Failed to download {}", url))?;
    let streamed = known_size >= STREAM_THRESHOLD;

    if !streamed {
        let bytes = response.bytes().await.with_context(|| format!("Failed to read response body from {}", url))?;
        tokio::fs::write(dest, bytes).await.with_context(|| format!("Failed to write {}", dest.display()))?;
    } else {
        let file = tokio::fs::File::create(dest).await.with_context(|| format!("Failed to create {}", dest.display()))?;

        // Use a 1 MB buffer for large files (streamed so >= STREAM_THRESHOLD) instead of the default 8 KB
        let mut writer = BufWriter::with_capacity(1024 * 1024, file);

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("Stream error from {}", url))?;
            writer.write_all(&chunk).await.with_context(|| format!("Failed to write chunk to {}", dest.display()))?;
        }
        writer.flush().await.with_context(|| format!("Failed to flush buffer for {}", dest.display()))?;
    }
    Ok(())
}

/// Ensure a single product file is at the correct version.
#[allow(clippy::too_many_arguments)]
async fn update_file(
    client: &Client,
    base_url: &str,
    product_name: &str,
    version: &str,
    product_dir: &Path,
    temp_dir: &Path,
    rel_path: &str,
    entry: &FileEntry,
    allow_patch: bool
) -> Result<()> {
    let dest = product_dir.join(rel_path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    // Already up to date?
    if dest.exists() && file_hash_async(dest.clone()).await.unwrap_or_default() == entry.hash {
        return Ok(());
    }

    const MAX_RETRIES: usize = 3;
    let mut attempts = 0;

    loop {
        attempts += 1;
        let mut patch_successful = false;

        // Try patching (We only try this on the first attempt to save time)
        if allow_patch && attempts == 1 {
            if let (Some(patch_info), true) = (&entry.patch, dest.exists()) {
                let url_patch_file = patch_info.file.replace('\\', "/");
                let patch_url = format!("{}products/{}/{}/{}", base_url, product_name, version, url_patch_file);
                let safe_temp_name = blake3::hash(rel_path.as_bytes()).to_hex().to_string();
                let patch_dest = temp_dir.join(format!("{}.patch", safe_temp_name));

                // If download succeeds, try to apply it
                if download_to(client, &patch_url, &patch_dest, 0).await.is_ok() {
                    if apply_patch(dest.clone(), patch_dest.clone(), dest.clone()).await.is_ok() {
                        if file_hash_async(dest.clone()).await.unwrap_or_default() == entry.hash {
                            patch_successful = true;
                        }
                    }
                }
                let _ = fs::remove_file(&patch_dest); // Always clean up the patch file

            }
        }

        if patch_successful {
            return Ok(());
        }

        // Full download fallback
        let url_rel_path = rel_path.replace('\\', "/");
        let full_url = format!("{}products/{}/{}/full/{}", base_url, product_name, version, url_rel_path);
        let safe_temp_name = blake3::hash(rel_path.as_bytes()).to_hex().to_string();
        let download_temp_dest = temp_dir.join(format!("{}.download", safe_temp_name));

        let download_result = async {
            download_to(client, &full_url, &download_temp_dest, entry.size).await?;

            let downloaded_hash = file_hash_async(download_temp_dest.clone()).await.unwrap_or_default();
            if downloaded_hash != entry.hash {
                let _ = fs::remove_file(&download_temp_dest); // Clean up bad file
                return Err(anyhow::anyhow!("Hash mismatch after downloading full file: {}", rel_path));
            }

            // Move it to final destination
            fs::rename(&download_temp_dest, &dest).with_context(|| format!("Failed to move downloaded file to {}", dest.display()))?;

            Ok::<(), anyhow::Error>(())
        }.await;

        // Evaluate the result
        match download_result {
            Ok(_) => return Ok(()),
            Err(e) => {
                if attempts >= MAX_RETRIES {
                    return Err(e.context(format!("Failed to update {} after {} attempts", rel_path, MAX_RETRIES)));
                }
                // Wait 1 second before retrying to give the network a chance to stabilize
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}