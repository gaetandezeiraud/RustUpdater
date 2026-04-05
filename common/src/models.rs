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
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Root index listing all available products and their latest version.
/// Uses BTreeMap for deterministic (sorted) JSON serialisation.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct RootJson {
    #[serde(default)]
    pub products: BTreeMap<String, ProductEntry>,
}

/// Per-product metadata stored in root.json.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProductEntry {
    pub latest_version: String,
    /// Relative path to the latest manifest inside the CDN.
    pub manifest: String,
    #[serde(default)]
    pub versions: Vec<String>,
}

/// Version manifest describing every file that belongs to a product release.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub version: String,
    /// Main executable path, relative to the product install directory.
    #[serde(default)]
    pub exe: String,
    pub files: HashMap<String, FileEntry>,
    #[serde(default)]
    pub deleted_files: Vec<String>,
    /// Total uncompressed size of all files in this release (bytes).
    #[serde(default)]
    pub full_size: u64,
    /// Total size of all patch files for this release (bytes).
    /// Zero when no patches are available (e.g. first release).
    #[serde(default)]
    pub total_patch_size: u64,
}

/// Metadata for a single file tracked by a manifest.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    /// BLAKE3 hex digest of the file content.
    pub hash: String,
    /// Uncompressed file size in bytes.
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<PatchInfo>,
}

/// Optional incremental patch information for a file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatchInfo {
    /// CDN-relative path to the HDiffPatch binary patch file.
    pub file: String,
    /// Size of the patch file in bytes (0 when not provided by the server).
    #[serde(default)]
    pub size: u64,
}

