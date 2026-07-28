use crate::depot_metadata::AppIdentity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CachedIdentity {
    stored_at_seconds: u64,
    identity: AppIdentity,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CacheFile {
    version: u32,
    entries: BTreeMap<u32, CachedIdentity>,
}

pub struct AppIdentityCache {
    path: PathBuf,
    file: CacheFile,
}

impl AppIdentityCache {
    pub async fn load() -> Self {
        let path = cache_path().unwrap_or_else(|_| PathBuf::from("app-identities-v1.json"));
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
                entries: BTreeMap::new(),
            });
        Self { path, file }
    }

    pub fn get(&self, app_id: u32) -> Option<AppIdentity> {
        self.file
            .entries
            .get(&app_id)
            .map(|cached| cached.identity.clone())
    }

    pub fn insert(&mut self, identity: &AppIdentity, now: SystemTime) {
        self.file.entries.insert(
            identity.app_id,
            CachedIdentity {
                stored_at_seconds: unix_seconds(now),
                identity: identity.clone(),
            },
        );
    }

    pub async fn save(&self) -> Result<(), String> {
        let path = self.path.clone();
        let encoded = serde_json::to_vec(&self.file)
            .map_err(|error| format!("Could not encode the app-identity cache: {error}"))?;
        tauri::async_runtime::spawn_blocking(move || write_cache_file(&path, &encoded))
            .await
            .map_err(|error| format!("App-identity cache task failed: {error}"))?
    }
}

fn read_cache_file(path: &Path) -> Result<CacheFile, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not decode the app-identity cache: {error}")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(CacheFile::default()),
        Err(error) => Err(format!("Could not read the app-identity cache: {error}")),
    }
}

fn write_cache_file(path: &Path, encoded: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "App-identity cache path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the app-identity cache directory: {error}"))?;
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("Could not open the app-identity cache: {error}"))?;
    file.write_all(encoded)
        .map_err(|error| format!("Could not save the app-identity cache: {error}"))
}

fn cache_path() -> Result<PathBuf, String> {
    dirs_next::data_local_dir()
        .map(|directory| {
            directory
                .join("Steam Storage Optimiser")
                .join("app-identities-v1.json")
        })
        .ok_or_else(|| "Could not locate the local application-data directory".to_string())
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn identity() -> AppIdentity {
        AppIdentity {
            app_id: 223_850,
            name: "3DMark".to_string(),
            app_type: "application".to_string(),
        }
    }

    #[test]
    fn serves_entries_indefinitely() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut cache = AppIdentityCache {
            path: PathBuf::from("unused"),
            file: CacheFile::default(),
        };
        cache.insert(&identity(), start);

        assert_eq!(cache.get(223_850), Some(identity()));
    }
}
