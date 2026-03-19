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
use crate::patchers::HDiff;
use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration};
use tokio::io::AsyncWriteExt;

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
    pub async fn fetch_root(&self) -> Result<RootJson> {
        let url = format!("{}root.json", self.base_url);
        self.client.get(&url).send().await?.json().await.context("Failed to parse root.json")
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
        if total_patch_cost > 0 && total_patch_cost < full_size {
            println!("-> Patch cost ({} bytes) is cheaper than full download ({} bytes). Applying sequentially...", total_patch_cost, full_size);
            for manifest in manifests {
                println!("-> Applying update for version {}...", manifest.version);
                self.apply_manifest(product_name, &manifest, &product_dir, &temp_dir, true, completed_files.clone(), total_files, on_progress.clone()).await?;
                // Save intermediate version progression in case of unexpected closure
                Self::save_local_version(&product_dir, &manifest.version)?;
            }
        } else {
            println!("-> Patch cost ({} bytes) exceeds full download ({} bytes) or patches are missing. Forcing full download...", total_patch_cost, full_size);
            // Apply the final manifest directly with patching disabled to force a fresh download
            self.apply_manifest(product_name, &target_manifest, &product_dir, &temp_dir, false, completed_files, total_files, on_progress).await?;
            Self::save_local_version(&product_dir, target_version)?;
        }

        let _ = fs::remove_dir_all(&temp_dir);
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

    /// Verify the integrity of a locally installed product against its manifest
    pub async fn verify_integrity<F>(&self, product_name: &str, version: &str, on_progress: F) -> Result<Vec<String>>
    where
        F: Fn(usize, usize) + Send + Sync + 'static,
    {
        println!("Fetching manifest to verify {} v{}...", product_name, version);
        let manifest = self.fetch_manifest(product_name, version).await?;
        let product_dir = self.install_dir.join(product_name);

        if !product_dir.exists() { return Err(anyhow::anyhow!("Product directory does not exist.")); }

        println!("Verifying {} files. This may take a moment...", manifest.files.len());

        let total_files = manifest.files.len();
        let completed_files = Arc::new(AtomicUsize::new(0));
        let on_progress = Arc::new(on_progress);

        // Process file hashing concurrently
        let corrupted_files: Vec<String> = stream::iter(manifest.files)
            .map(|(rel_path, entry)| {
                let path = product_dir.join(&rel_path);
                let expected_hash = entry.hash.clone();
                let rel_path_clone = rel_path.clone();

                let completed_clone = completed_files.clone();
                let prog_clone = Arc::clone(&on_progress);

                async move {
                    let res = tokio::task::spawn_blocking(move || {
                        if !path.exists() { return Some(rel_path_clone); }

                        // Hash the file and compare
                        match file_hash(&path) {
                            Ok(hash) if hash == expected_hash => None,
                            _ => Some(rel_path_clone),
                        }
                    }).await.expect("Blocking task panicked");

                    // Atomically increment and report progress
                    let current = completed_clone.fetch_add(1, Ordering::Relaxed) + 1;
                    prog_clone(current, total_files);

                    res
                }
            })
            .buffer_unordered(CONCURRENCY)
            .filter_map(|result| async { result })
            .collect()
            .await;

        Ok(corrupted_files)
    }
}

/// Compute the BLAKE3 hash of a file on disk.
fn file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("Failed to open file for hashing: {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
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
        let mut file = tokio::fs::File::create(dest).await.with_context(|| format!("Failed to create {}", dest.display()))?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("Stream error from {}", url))?;
            file.write_all(&chunk).await.with_context(|| format!("Failed to write chunk to {}", dest.display()))?;
        }
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
    if dest.exists() && file_hash(&dest).unwrap_or_default() == entry.hash {
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
                let patch_url = format!("{}products/{}/{}/{}", base_url, product_name, version, patch_info.file);
                let safe_temp_name = rel_path.replace("/", "_").replace("\\", "_");
                let patch_dest = temp_dir.join(format!("{}.patch", safe_temp_name));

                // If download succeeds, try to apply it
                if download_to(client, &patch_url, &patch_dest, 0).await.is_ok() {
                    if apply_patch(dest.clone(), patch_dest.clone(), dest.clone()).await.is_ok() {
                        if file_hash(&dest).unwrap_or_default() == entry.hash {
                            patch_successful = true;
                        }
                    }
                }
                let _ = fs::remove_file(&patch_dest); // Always clean up the patch file

                if !patch_successful {
                    eprintln!("Patch failed or hash mismatch for {}. Falling back to full download...", rel_path);
                }
            }
        }

        if patch_successful {
            return Ok(());
        }

        // Full download fallback
        let full_url = format!("{}products/{}/{}/full/{}", base_url, product_name, version, rel_path);
        let safe_temp_name = rel_path.replace("/", "_").replace("\\", "_");
        let download_temp_dest = temp_dir.join(format!("{}.download", safe_temp_name));

        let download_result = async {
            download_to(client, &full_url, &download_temp_dest, entry.size).await?;

            let downloaded_hash = file_hash(&download_temp_dest).unwrap_or_default();
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
                eprintln!("Error updating {}: {}. Retrying {}/{}...", rel_path, e, attempts, MAX_RETRIES);

                // Wait 1 second before retrying to give the network a chance to stabilize
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}