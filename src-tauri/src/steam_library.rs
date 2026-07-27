use crate::depot_metadata::{AppIdentity, parse_app_identity};
use prost::Message;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use steamroom::client::{LoggedIn, SteamClient};
use steamroom::depot::AppId;
use steamroom::depot::PackageId;
use steamroom::generated::{
    CMsgClientLicenseList, CPlayerGetOwnedGamesRequest, CPlayerGetOwnedGamesResponse,
    CPlayerGetPlayerLinkDetailsRequest, CPlayerGetPlayerLinkDetailsResponse,
};
use steamroom::messages::EMsg;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DepotEstimateStatus {
    #[default]
    Pending,
    Available,
    Unavailable,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePreview {
    pub app_id: u32,
    pub name: String,
    pub playtime_minutes: Option<i32>,
    pub shared_only: bool,
    pub installed: bool,
    pub local_size_bytes: Option<u64>,
    pub depot_size_bytes: Option<u64>,
    pub depot_status: DepotEstimateStatus,
    pub depot_exact: bool,
    pub depot_count: Option<usize>,
    pub depot_os: Option<String>,
    pub current_os_supported: Option<bool>,
    pub depot_warnings: Vec<String>,
    pub depot_error: Option<String>,
    pub community_size_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamProfile {
    pub display_name: String,
    pub avatar_url: Option<String>,
}

pub struct OwnedLibrary {
    pub count: usize,
    pub preview: Vec<GamePreview>,
    pub shared_only_count: usize,
}

pub async fn get_shared_candidate_identities(
    client: &SteamClient<LoggedIn>,
    candidate_app_ids: &std::collections::HashSet<u32>,
) -> Result<Vec<AppIdentity>, String> {
    let app_ids = candidate_app_ids
        .iter()
        .copied()
        .map(AppId)
        .collect::<Vec<_>>();
    let mut identities = Vec::new();
    for app_batch in app_ids.chunks(100) {
        let tokens = client
            .pics_get_access_tokens(app_batch)
            .await
            .map_err(|error| error.to_string())?;
        let infos = client
            .pics_get_product_info(&tokens)
            .await
            .map_err(|error| error.to_string())?;
        identities.extend(infos.into_iter().filter_map(|info| {
            let app_id = info.app_id?.0;
            parse_app_identity(app_id, info.kv_data.as_deref()?).ok()
        }));
    }
    identities.sort_by_key(|app| app.app_id);
    identities.dedup_by_key(|app| app.app_id);
    Ok(identities)
}

pub async fn get_player_profile(
    client: &SteamClient<LoggedIn>,
    steam_id: u64,
) -> Result<SteamProfile, String> {
    let request = CPlayerGetPlayerLinkDetailsRequest {
        steamids: vec![steam_id],
    };
    let response = client
        .call_service_method("Player.GetPlayerLinkDetails#1", &request.encode_to_vec())
        .await
        .map_err(|error| error.to_string())?;
    let response: CPlayerGetPlayerLinkDetailsResponse =
        response.decode().map_err(|error| error.to_string())?;
    let public = response
        .accounts
        .into_iter()
        .find_map(|account| account.public_data)
        .ok_or_else(|| "Steam returned no public profile data".to_string())?;
    let display_name = public
        .persona_name
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "Steam profile has no display name".to_string())?;
    Ok(SteamProfile {
        display_name,
        avatar_url: public.sha_digest_avatar.as_deref().and_then(avatar_url),
    })
}

fn avatar_url(hash: &[u8]) -> Option<String> {
    if hash.is_empty() || hash.iter().all(|byte| *byte == 0) {
        return None;
    }
    let hash = hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Some(format!("https://avatars.steamstatic.com/{hash}_medium.jpg"))
}

impl OwnedLibrary {
    pub fn merge_shared_games(
        &mut self,
        identities: &[AppIdentity],
        local_playtimes: &BTreeMap<u32, u32>,
    ) {
        let existing = self
            .preview
            .iter()
            .map(|game| game.app_id)
            .collect::<BTreeSet<_>>();
        self.preview.extend(
            identities
                .iter()
                .filter(|app| app.app_type == "game" && !existing.contains(&app.app_id))
                .map(|app| GamePreview {
                    app_id: app.app_id,
                    name: app.name.clone(),
                    playtime_minutes: Some(
                        local_playtimes
                            .get(&app.app_id)
                            .copied()
                            .unwrap_or_default()
                            .min(i32::MAX as u32) as i32,
                    ),
                    shared_only: true,
                    installed: false,
                    local_size_bytes: None,
                    depot_size_bytes: None,
                    depot_status: DepotEstimateStatus::Pending,
                    depot_exact: false,
                    depot_count: None,
                    depot_os: None,
                    current_os_supported: None,
                    depot_warnings: Vec::new(),
                    depot_error: None,
                    community_size_bytes: None,
                }),
        );
        self.shared_only_count = self.preview.iter().filter(|game| game.shared_only).count();
        self.count = self.preview.len();
        self.preview.sort_by_key(|game| {
            (
                game.playtime_minutes.is_none(),
                std::cmp::Reverse(game.playtime_minutes.unwrap_or(0)),
            )
        });
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccountPackages {
    pub ids: Vec<PackageId>,
    pub notes: BTreeMap<u32, BTreeSet<String>>,
    pub sources: BTreeMap<u32, PackageLicenseSources>,
}

#[derive(Clone, Debug, Default)]
pub struct PackageLicenseSources {
    pub direct: bool,
    pub borrowed_owners: BTreeSet<u32>,
    pub preferred_borrowed_owners: BTreeSet<u32>,
}

pub async fn get_account_packages(
    client: &SteamClient<LoggedIn>,
    account_id: u32,
) -> Result<AccountPackages, String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let incoming = client.recv_msg().await.map_err(|error| error.to_string())?;
            if incoming.emsg == EMsg::CLIENT_LICENSE_LIST {
                return decode_license_packages(&incoming.body, account_id);
            }
            if incoming.emsg == EMsg::MULTI {
                for message in steamroom::client::multi::unpack_multi(&incoming.body)
                    .map_err(|error| error.to_string())?
                {
                    if let Some(body) = protobuf_body(&message, EMsg::CLIENT_LICENSE_LIST) {
                        return decode_license_packages(body, account_id);
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| "Steam did not send the account package-license list".to_string())?
}

fn decode_license_packages(body: &[u8], account_id: u32) -> Result<AccountPackages, String> {
    let response = CMsgClientLicenseList::decode(body).map_err(|error| error.to_string())?;
    let mut packages = AccountPackages::default();
    for license in response.licenses {
        let Some(package_id) = license.package_id else {
            continue;
        };
        let flags = license.flags.unwrap_or(0);
        if flags & EXPIRED == 0 {
            packages.ids.push(PackageId(package_id));
            let source = packages.sources.entry(package_id).or_default();
            if flags & BORROWED == 0 {
                source.direct = true;
            } else if let Some(owner_id) = license.owner_id {
                source.borrowed_owners.insert(owner_id);
                if flags & PREFERRED_OWNER != 0 {
                    source.preferred_borrowed_owners.insert(owner_id);
                }
            }
        }
        let notes = packages.notes.entry(package_id).or_default();
        notes.insert(format!("flags 0x{flags:x}"));
        notes.insert(match license.owner_id {
            Some(owner) if owner == account_id => "owner is this account".to_string(),
            Some(0) => "owner field is zero".to_string(),
            Some(_) => "owner differs from this account".to_string(),
            None => "owner field absent".to_string(),
        });
        if let Some(type_) = license.license_type {
            notes.insert(format!("license type {type_}"));
        }
        if let Some(payment_method) = license.payment_method {
            notes.insert(format!("payment method {payment_method}"));
        }
        if let Some(limit) = license.minute_limit
            && limit > 0
        {
            notes.insert(format!(
                "timed license: {}/{} minutes used",
                license.minutes_used.unwrap_or(0),
                limit
            ));
        }
    }
    packages.ids.sort_by_key(|package| package.0);
    packages.ids.dedup();
    Ok(packages)
}

const EXPIRED: u32 = 0x08;
const BORROWED: u32 = 0x4000;
const PREFERRED_OWNER: u32 = 0x100000;

fn protobuf_body(message: &[u8], expected: EMsg) -> Option<&[u8]> {
    let raw_emsg = u32::from_le_bytes(message.get(0..4)?.try_into().ok()?);
    if raw_emsg & 0x7fff_ffff != expected.0 {
        return None;
    }
    let header_len = u32::from_le_bytes(message.get(4..8)?.try_into().ok()?) as usize;
    message.get(8 + header_len..)
}

pub async fn get_owned_games(
    client: &SteamClient<LoggedIn>,
    steam_id: u64,
    shared_only_app_ids: &std::collections::HashSet<u32>,
) -> Result<OwnedLibrary, String> {
    let request = CPlayerGetOwnedGamesRequest {
        steamid: Some(steam_id),
        include_appinfo: Some(true),
        include_played_free_games: Some(true),
        include_free_sub: Some(true),
        language: Some("english".to_string()),
        ..Default::default()
    };
    let response = client
        .call_service_method("Player.GetOwnedGames#1", &request.encode_to_vec())
        .await
        .map_err(|error| error.to_string())?;
    let response: CPlayerGetOwnedGamesResponse =
        response.decode().map_err(|error| error.to_string())?;
    let games: Vec<GamePreview> = response
        .games
        .into_iter()
        .filter_map(|game| {
            Some(GamePreview {
                app_id: u32::try_from(game.appid?).ok()?,
                name: game.name.unwrap_or_else(|| "Unknown game".to_string()),
                playtime_minutes: Some(game.playtime_forever.unwrap_or(0).max(0)),
                shared_only: shared_only_app_ids.contains(&u32::try_from(game.appid?).ok()?),
                installed: false,
                local_size_bytes: None,
                depot_size_bytes: None,
                depot_status: DepotEstimateStatus::Pending,
                depot_exact: false,
                depot_count: None,
                depot_os: None,
                current_os_supported: None,
                depot_warnings: Vec::new(),
                depot_error: None,
                community_size_bytes: None,
            })
        })
        .collect();
    let shared_only_count = games.iter().filter(|game| game.shared_only).count();
    let count = response
        .game_count
        .map(|count| count as usize)
        .unwrap_or(games.len());
    Ok(OwnedLibrary {
        count,
        preview: games,
        shared_only_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use steamroom::generated::c_msg_client_license_list::License;

    #[test]
    fn decodes_unique_sorted_package_ids_without_retaining_license_secrets() {
        let message = CMsgClientLicenseList {
            licenses: vec![
                License {
                    package_id: Some(20),
                    access_token: Some(123),
                    ..Default::default()
                },
                License {
                    package_id: Some(10),
                    ..Default::default()
                },
                License {
                    package_id: Some(20),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let packages =
            decode_license_packages(&message.encode_to_vec(), 42).expect("licenses should decode");
        assert_eq!(
            packages
                .ids
                .iter()
                .map(|package| package.0)
                .collect::<Vec<_>>(),
            vec![10, 20]
        );
        assert!(packages.notes[&20].contains("flags 0x0"));
    }

    #[test]
    fn excludes_expired_and_borrowed_licenses_but_not_foreign_owner_fields() {
        let message = CMsgClientLicenseList {
            licenses: vec![
                License {
                    package_id: Some(1),
                    flags: Some(0x08),
                    owner_id: Some(42),
                    ..Default::default()
                },
                License {
                    package_id: Some(2),
                    owner_id: Some(99),
                    ..Default::default()
                },
                License {
                    package_id: Some(3),
                    owner_id: Some(42),
                    ..Default::default()
                },
                License {
                    package_id: Some(4),
                    flags: Some(0x4200),
                    owner_id: Some(99),
                    ..Default::default()
                },
                License {
                    package_id: Some(5),
                    flags: Some(0x104000),
                    owner_id: Some(99),
                    ..Default::default()
                },
                License {
                    package_id: Some(5),
                    flags: Some(0x200),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let packages =
            decode_license_packages(&message.encode_to_vec(), 42).expect("licenses should decode");
        assert_eq!(
            packages
                .ids
                .iter()
                .map(|package| package.0)
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert!(packages.notes[&1].contains("flags 0x8"));
        assert!(packages.notes[&2].contains("owner differs from this account"));
        assert!(packages.notes[&4].contains("flags 0x4200"));
        assert!(packages.notes[&5].contains("flags 0x104000"));
        assert!(packages.notes[&5].contains("flags 0x200"));
        assert!(!packages.sources[&4].direct);
        assert_eq!(packages.sources[&4].borrowed_owners, BTreeSet::from([99]));
        assert!(packages.sources[&5].direct);
        assert_eq!(
            packages.sources[&5].preferred_borrowed_owners,
            BTreeSet::from([99])
        );
    }

    #[test]
    fn merges_shared_games_with_zero_playtime_when_no_local_record_exists() {
        let mut library = OwnedLibrary {
            count: 1,
            preview: vec![GamePreview {
                app_id: 1,
                name: "Owned".to_string(),
                playtime_minutes: Some(60),
                shared_only: false,
                installed: false,
                local_size_bytes: None,
                depot_size_bytes: None,
                depot_status: DepotEstimateStatus::Pending,
                depot_exact: false,
                depot_count: None,
                depot_os: None,
                current_os_supported: None,
                depot_warnings: Vec::new(),
                depot_error: None,
                community_size_bytes: None,
            }],
            shared_only_count: 0,
        };
        library.merge_shared_games(
            &[
                AppIdentity {
                    app_id: 2,
                    name: "Played shared game".to_string(),
                    app_type: "game".to_string(),
                },
                AppIdentity {
                    app_id: 3,
                    name: "Unplayed shared game".to_string(),
                    app_type: "game".to_string(),
                },
                AppIdentity {
                    app_id: 4,
                    name: "Shared DLC".to_string(),
                    app_type: "dlc".to_string(),
                },
            ],
            &BTreeMap::from([(2, 120)]),
        );

        assert_eq!(library.count, 3);
        assert_eq!(library.shared_only_count, 2);
        assert_eq!(library.preview[0].app_id, 2);
        assert_eq!(library.preview[0].playtime_minutes, Some(120));
        assert_eq!(library.preview[2].app_id, 3);
        assert_eq!(library.preview[2].playtime_minutes, Some(0));
    }

    #[test]
    fn builds_a_medium_avatar_url_from_steams_digest() {
        assert_eq!(
            avatar_url(&[0x0a, 0xff]),
            Some("https://avatars.steamstatic.com/0aff_medium.jpg".to_string())
        );
        assert_eq!(avatar_url(&[0; 20]), None);
    }
}
