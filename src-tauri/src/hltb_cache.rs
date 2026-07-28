use crate::hltb::HltbEstimate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u32 = 2;
const MATCH_TTL: Duration = Duration::from_secs(183 * 24 * 60 * 60);
const NO_MATCH_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
// One-off retry for automatic misses produced during the initial HLTB rollout.
// Newly retried misses have a later timestamp and return to the normal TTL.
const UNMATCHED_RETRY_CUTOFF_SECONDS: u64 = 1_785_184_789;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CachedHltbEntry {
    pub stored_at_seconds: u64,
    pub estimate: Option<HltbEstimate>,
    pub manual: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CacheFile {
    version: u32,
    entries: BTreeMap<u32, CachedHltbEntry>,
}

pub struct HltbCache {
    path: PathBuf,
    file: CacheFile,
}

impl HltbCache {
    pub async fn load() -> Self {
        let path = cache_path().unwrap_or_else(|_| PathBuf::from("hltb-cache-v2.json"));
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

    pub fn get(&self, app_id: u32) -> Option<&CachedHltbEntry> {
        self.file.entries.get(&app_id)
    }

    pub fn is_fresh(entry: &CachedHltbEntry, now: SystemTime) -> bool {
        if entry.manual && entry.estimate.is_none() {
            return true;
        }
        if !entry.manual
            && entry.estimate.is_none()
            && entry.stored_at_seconds <= UNMATCHED_RETRY_CUTOFF_SECONDS
        {
            return false;
        }
        let ttl = if entry.estimate.is_some() {
            MATCH_TTL
        } else {
            NO_MATCH_TTL
        };
        unix_seconds(now).saturating_sub(entry.stored_at_seconds) < ttl.as_secs()
    }

    pub fn insert(
        &mut self,
        app_id: u32,
        estimate: Option<HltbEstimate>,
        manual: bool,
        now: SystemTime,
    ) {
        self.file.entries.insert(
            app_id,
            CachedHltbEntry {
                stored_at_seconds: unix_seconds(now),
                estimate,
                manual,
            },
        );
    }

    pub async fn save(&self) -> Result<(), String> {
        let path = self.path.clone();
        let encoded = serde_json::to_vec(&self.file)
            .map_err(|error| format!("Could not encode the HLTB cache: {error}"))?;
        tauri::async_runtime::spawn_blocking(move || write_cache_file(&path, &encoded))
            .await
            .map_err(|error| format!("HLTB-cache task failed: {error}"))?
    }
}

fn read_cache_file(path: &Path) -> Result<CacheFile, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not decode the HLTB cache: {error}")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(CacheFile::default()),
        Err(error) => Err(format!("Could not read the HLTB cache: {error}")),
    }
}

fn write_cache_file(path: &Path, encoded: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "HLTB cache path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the HLTB cache directory: {error}"))?;
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("Could not open the HLTB cache: {error}"))?;
    file.write_all(encoded)
        .map_err(|error| format!("Could not save the HLTB cache: {error}"))
}

fn cache_path() -> Result<PathBuf, String> {
    dirs_next::data_local_dir()
        .map(|directory| {
            directory
                .join("Steam Storage Optimiser")
                .join("hltb-cache-v2.json")
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

    #[test]
    fn matched_entries_last_six_months_and_no_matches_one_month() {
        let start = UNIX_EPOCH + Duration::from_secs(10_000);
        let matched = CachedHltbEntry {
            stored_at_seconds: unix_seconds(start),
            estimate: Some(HltbEstimate {
                game_id: 1,
                game_name: "Fixture".to_string(),
                main_seconds: Some(1),
                main_extra_seconds: Some(2),
                completionist_seconds: Some(3),
                steam_app_id: Some(42),
                match_method: "steam_app_id".to_string(),
            }),
            manual: false,
        };
        let unmatched = CachedHltbEntry {
            stored_at_seconds: unix_seconds(start),
            estimate: None,
            manual: false,
        };
        assert!(HltbCache::is_fresh(
            &matched,
            start + MATCH_TTL - Duration::from_secs(1)
        ));
        assert!(!HltbCache::is_fresh(&matched, start + MATCH_TTL));
        assert!(!HltbCache::is_fresh(&unmatched, start + NO_MATCH_TTL));
    }

    #[test]
    fn retries_only_initial_automatic_misses_once() {
        let now = UNIX_EPOCH + Duration::from_secs(UNMATCHED_RETRY_CUTOFF_SECONDS + 100);
        let initial_miss = CachedHltbEntry {
            stored_at_seconds: UNMATCHED_RETRY_CUTOFF_SECONDS,
            estimate: None,
            manual: false,
        };
        let retried_miss = CachedHltbEntry {
            stored_at_seconds: UNMATCHED_RETRY_CUTOFF_SECONDS + 1,
            estimate: None,
            manual: false,
        };
        let manual_miss = CachedHltbEntry {
            manual: true,
            ..initial_miss.clone()
        };

        assert!(!HltbCache::is_fresh(&initial_miss, now));
        assert!(HltbCache::is_fresh(&retried_miss, now));
        assert!(HltbCache::is_fresh(&manual_miss, now));
    }
}
