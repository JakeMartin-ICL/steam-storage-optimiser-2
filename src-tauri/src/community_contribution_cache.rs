use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

const CACHE_VERSION: u32 = 1;
const LOCAL_SIZE_CHANGE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CacheFile {
    version: u32,
    sizes: BTreeMap<u32, u64>,
}

pub struct CommunityContributionCache {
    path: PathBuf,
    file: CacheFile,
}

impl CommunityContributionCache {
    pub async fn load() -> Self {
        let path =
            cache_path().unwrap_or_else(|_| PathBuf::from("community-contributions-v1.json"));
        let loaded = {
            let path = path.clone();
            tauri::async_runtime::spawn_blocking(move || read_cache_file(&path))
                .await
                .ok()
                .and_then(Result::ok)
        };
        let file = loaded
            .filter(|file| file.version == CACHE_VERSION)
            .unwrap_or_else(|| CacheFile {
                version: CACHE_VERSION,
                sizes: BTreeMap::new(),
            });
        Self { path, file }
    }

    pub fn has_changed(&self, app_id: u32, size_bytes: u64) -> bool {
        self.file.sizes.get(&app_id).is_none_or(|previous_size| {
            previous_size.abs_diff(size_bytes) >= LOCAL_SIZE_CHANGE_THRESHOLD_BYTES
        })
    }

    pub fn record(&mut self, app_id: u32, size_bytes: u64) {
        self.file.sizes.insert(app_id, size_bytes);
    }

    pub async fn save(&self) -> Result<(), String> {
        let path = self.path.clone();
        let encoded = serde_json::to_vec(&self.file)
            .map_err(|error| format!("Could not encode the contribution cache: {error}"))?;
        tauri::async_runtime::spawn_blocking(move || write_cache_file(&path, &encoded))
            .await
            .map_err(|error| format!("Contribution-cache task failed: {error}"))?
    }
}

fn read_cache_file(path: &Path) -> Result<CacheFile, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not decode the contribution cache: {error}")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(CacheFile::default()),
        Err(error) => Err(format!("Could not read the contribution cache: {error}")),
    }
}

fn write_cache_file(path: &Path, encoded: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Contribution cache path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the contribution cache directory: {error}"))?;
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("Could not open the contribution cache: {error}"))?;
    file.write_all(encoded)
        .map_err(|error| format!("Could not save the contribution cache: {error}"))
}

fn cache_path() -> Result<PathBuf, String> {
    dirs_next::data_local_dir()
        .map(|directory| {
            directory
                .join("Steam Storage Optimiser")
                .join("community-contributions-v1.json")
        })
        .ok_or_else(|| "Could not locate the local application-data directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cache() -> CommunityContributionCache {
        CommunityContributionCache {
            path: PathBuf::from("unused"),
            file: CacheFile {
                version: CACHE_VERSION,
                sizes: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn only_marks_new_or_meaningfully_changed_install_sizes_for_contribution() {
        let mut cache = empty_cache();
        let original_size = 1024 * 1024 * 1024;
        assert!(cache.has_changed(42, original_size));

        cache.record(42, original_size);
        assert!(!cache.has_changed(42, original_size));
        assert!(!cache.has_changed(42, original_size + LOCAL_SIZE_CHANGE_THRESHOLD_BYTES - 1));
        assert!(cache.has_changed(42, original_size + LOCAL_SIZE_CHANGE_THRESHOLD_BYTES));
        assert!(cache.has_changed(42, original_size - LOCAL_SIZE_CHANGE_THRESHOLD_BYTES));
    }
}
