use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::task::JoinSet;

const COMMUNITY_API_BASE: &str = "https://eu5di55p9a.execute-api.eu-west-2.amazonaws.com/default";
const BATCH_SIZE: usize = 100;
const UPDATE_THRESHOLD_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContributionAction {
    None,
    Create,
    Update,
}

#[derive(Clone)]
pub struct CommunitySizeClient {
    http: Client,
}

#[derive(Debug, Serialize)]
struct BatchRequest<'a> {
    ids: &'a [u32],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct CommunitySize {
    pub app_id: u32,
    pub size: u64,
    #[serde(default)]
    pub name: Option<String>,
}

impl CommunitySizeClient {
    pub fn new() -> Result<Self, String> {
        let http = Client::builder()
            .user_agent(concat!("SteamStorageOptimiser/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| format!("Could not create community API client: {error}"))?;
        Ok(Self { http })
    }

    pub async fn get_sizes(&self, app_ids: &[u32]) -> Result<BTreeMap<u32, CommunitySize>, String> {
        let mut requests = JoinSet::new();
        for ids in app_ids.chunks(BATCH_SIZE) {
            let client = self.clone();
            let ids = ids.to_vec();
            requests.spawn(async move { client.get_size_batch(&ids).await });
        }

        let mut sizes = BTreeMap::new();
        while let Some(result) = requests.join_next().await {
            let records = result
                .map_err(|error| format!("Community size request task failed: {error}"))??;
            for record in records {
                sizes.insert(record.app_id, record);
            }
        }
        Ok(sizes)
    }

    async fn get_size_batch(&self, ids: &[u32]) -> Result<Vec<CommunitySize>, String> {
        let response = self
            .http
            .get(format!("{COMMUNITY_API_BASE}/apps"))
            .json(&BatchRequest { ids })
            .send()
            .await
            .map_err(|error| format!("Community size request failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("Community size service returned an error: {error}"))?;
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("Could not read community size response: {error}"))?;
        decode_sizes(&body)
    }

    pub async fn contribute_installed_size(
        &self,
        app_id: u32,
        size_bytes: u64,
        name: &str,
        current_community_size: Option<u64>,
    ) -> Result<bool, String> {
        let request = match contribution_action(size_bytes, current_community_size) {
            ContributionAction::Create => {
                self.http.post(format!("{COMMUNITY_API_BASE}/app/{app_id}"))
            }
            ContributionAction::Update => {
                self.http.put(format!("{COMMUNITY_API_BASE}/app/{app_id}"))
            }
            ContributionAction::None => return Ok(false),
        };
        request
            .query(&[("size", size_bytes.to_string()), ("name", name.to_string())])
            .send()
            .await
            .map_err(|error| format!("Community size contribution failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("Community size service rejected a contribution: {error}"))?;
        Ok(true)
    }
}

fn contribution_action(
    observed_size: u64,
    current_community_size: Option<u64>,
) -> ContributionAction {
    match current_community_size {
        None => ContributionAction::Create,
        Some(current) if current.abs_diff(observed_size) > UPDATE_THRESHOLD_BYTES => {
            ContributionAction::Update
        }
        Some(_) => ContributionAction::None,
    }
}

fn decode_sizes(body: &[u8]) -> Result<Vec<CommunitySize>, String> {
    serde_json::from_slice(body)
        .map_err(|error| format!("Community size response had an unexpected shape: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_legacy_pascal_case_contract() {
        let records = decode_sizes(
            br#"[
                {"AppId": 400, "Size": 12756049920, "Name": "Portal"},
                {"AppId": 42, "Size": 1024}
            ]"#,
        )
        .expect("legacy response should decode");

        assert_eq!(
            records,
            vec![
                CommunitySize {
                    app_id: 400,
                    size: 12_756_049_920,
                    name: Some("Portal".to_string()),
                },
                CommunitySize {
                    app_id: 42,
                    size: 1024,
                    name: None,
                },
            ]
        );
    }

    #[test]
    fn rejects_a_changed_response_shape() {
        let error = decode_sizes(br#"{"games":[]}"#).expect_err("object is not the contract");
        assert!(error.contains("unexpected shape"));
    }

    #[test]
    fn legacy_batch_limit_is_one_hundred() {
        let ids = (0..201).collect::<Vec<_>>();
        assert_eq!(
            ids.chunks(BATCH_SIZE).map(<[u32]>::len).collect::<Vec<_>>(),
            vec![100, 100, 1]
        );
    }

    #[test]
    fn preserves_the_legacy_one_gib_update_threshold() {
        assert_eq!(contribution_action(2_000, None), ContributionAction::Create);
        assert_eq!(
            contribution_action(2_000, Some(2_000 + UPDATE_THRESHOLD_BYTES)),
            ContributionAction::None
        );
        assert_eq!(
            contribution_action(2_000, Some(2_001 + UPDATE_THRESHOLD_BYTES)),
            ContributionAction::Update
        );
    }
}
