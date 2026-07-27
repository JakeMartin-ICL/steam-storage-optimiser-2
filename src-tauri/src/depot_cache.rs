use crate::depot_probe::{DepotEstimate, DepotEstimateRequest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u32 = 1;
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CachedEstimate {
    stored_at_seconds: u64,
    language: String,
    estimate: DepotEstimate,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CacheFile {
    version: u32,
    steam_id: u64,
    entitlement_fingerprint: u64,
    platform: String,
    entries: BTreeMap<u32, CachedEstimate>,
}

pub struct DepotCache {
    path: PathBuf,
    file: CacheFile,
}

impl DepotCache {
    pub async fn load(steam_id: u64, entitlement_fingerprint: u64) -> Self {
        let path = cache_path().unwrap_or_else(|_| PathBuf::from("depot-cache-v1.json"));
        let expected_platform = platform_key();
        let loaded = {
            let path = path.clone();
            tauri::async_runtime::spawn_blocking(move || read_cache_file(&path))
                .await
                .ok()
                .and_then(Result::ok)
        };
        let file = loaded
            .filter(|file| {
                file.version == CACHE_VERSION
                    && file.steam_id == steam_id
                    && file.entitlement_fingerprint == entitlement_fingerprint
                    && file.platform == expected_platform
            })
            .unwrap_or_else(|| CacheFile {
                version: CACHE_VERSION,
                steam_id,
                entitlement_fingerprint,
                platform: expected_platform,
                entries: BTreeMap::new(),
            });
        Self { path, file }
    }

    pub fn get(
        &mut self,
        request: &DepotEstimateRequest,
        now: SystemTime,
    ) -> Option<DepotEstimate> {
        let now_seconds = unix_seconds(now);
        let cached = self.file.entries.get(&request.app_id)?;
        let fresh = cached.language == request.language.to_ascii_lowercase()
            && now_seconds.saturating_sub(cached.stored_at_seconds) < CACHE_TTL.as_secs();
        if fresh {
            Some(cached.estimate.clone())
        } else {
            self.file.entries.remove(&request.app_id);
            None
        }
    }

    pub fn insert(
        &mut self,
        request: &DepotEstimateRequest,
        estimate: &DepotEstimate,
        now: SystemTime,
    ) {
        self.file.entries.insert(
            request.app_id,
            CachedEstimate {
                stored_at_seconds: unix_seconds(now),
                language: request.language.to_ascii_lowercase(),
                estimate: estimate.clone(),
            },
        );
    }

    pub async fn save(&self) -> Result<(), String> {
        let path = self.path.clone();
        let encoded = serde_json::to_vec(&self.file)
            .map_err(|error| format!("Could not encode the depot cache: {error}"))?;
        tauri::async_runtime::spawn_blocking(move || write_cache_file(&path, &encoded))
            .await
            .map_err(|error| format!("Depot-cache task failed: {error}"))?
    }
}

fn read_cache_file(path: &Path) -> Result<CacheFile, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not decode the depot cache: {error}")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(CacheFile::default()),
        Err(error) => Err(format!("Could not read the depot cache: {error}")),
    }
}

fn write_cache_file(path: &Path, encoded: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Depot cache path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the depot cache directory: {error}"))?;
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("Could not open the depot cache: {error}"))?;
    file.write_all(encoded)
        .map_err(|error| format!("Could not save the depot cache: {error}"))
}

fn cache_path() -> Result<PathBuf, String> {
    dirs_next::data_local_dir()
        .map(|directory| {
            directory
                .join("Steam Storage Optimiser")
                .join("depot-cache-v1.json")
        })
        .ok_or_else(|| "Could not locate the local application-data directory".to_string())
}

fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estimate() -> DepotEstimate {
        DepotEstimate {
            app_id: 42,
            size_bytes: 1_024,
            depot_count: 2,
            target_os: "windows".to_string(),
            current_os_supported: false,
            warnings: vec!["fixture".to_string()],
        }
    }

    fn request(language: &str) -> DepotEstimateRequest {
        DepotEstimateRequest {
            app_id: 42,
            language: language.to_string(),
        }
    }

    #[test]
    fn serves_matching_entries_for_twenty_four_hours() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut cache = DepotCache {
            path: PathBuf::from("unused"),
            file: CacheFile::default(),
        };
        cache.insert(&request("English"), &estimate(), start);

        assert_eq!(
            cache.get(
                &request("english"),
                start + Duration::from_secs(24 * 60 * 60 - 1)
            ),
            Some(estimate())
        );
    }

    #[test]
    fn expires_old_or_different_language_entries() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut expired = DepotCache {
            path: PathBuf::from("unused"),
            file: CacheFile::default(),
        };
        expired.insert(&request("english"), &estimate(), start);
        assert_eq!(
            expired.get(
                &request("english"),
                start + Duration::from_secs(24 * 60 * 60)
            ),
            None
        );

        let mut different_language = DepotCache {
            path: PathBuf::from("unused"),
            file: CacheFile::default(),
        };
        different_language.insert(&request("english"), &estimate(), start);
        assert_eq!(different_language.get(&request("german"), start), None);
    }
}
