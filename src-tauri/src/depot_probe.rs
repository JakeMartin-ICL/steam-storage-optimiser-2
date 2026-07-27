use crate::depot_metadata::{AppDepotMetadata, parse_app_depot_metadata};
use crate::depot_selection::{
    SelectionContext, SelectionOutcome, TargetArchitecture, TargetOs, select_depots,
};
use crate::local_steam::{InstalledApp, discover_installed_apps};
use crate::package_entitlements::{GameEntitlements, PackageEntitlements};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use steamroom::apps::AccessToken;
use steamroom::cdn::CdnClient;
use steamroom::client::{LoggedIn, SteamClient};
use steamroom::depot::{AppId, CellId, DepotId, ManifestId};
use steamroom_client::manifest::parse_cdn_manifest;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepotResult {
    depot_id: u32,
    manifest_id: u64,
    compressed_bytes: u64,
    uncompressed_bytes: u64,
    file_count: usize,
    selection_reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepotProbe {
    app_id: u32,
    app_name: String,
    local_size_bytes: u64,
    product_metadata_bytes: usize,
    merged_manifest_bytes: u64,
    selection_warnings: Vec<String>,
    depots: Vec<DepotResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepotEstimateRequest {
    pub app_id: u32,
    pub language: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct DepotEstimate {
    pub app_id: u32,
    pub size_bytes: u64,
    pub depot_count: usize,
    pub target_os: String,
    pub current_os_supported: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepotEstimateOutcome {
    Available(DepotEstimate),
    Unavailable { app_id: u32, error: String },
}

impl DepotProbe {
    pub fn app_id(&self) -> u32 {
        self.app_id
    }

    pub fn merged_manifest_bytes(&self) -> u64 {
        self.merged_manifest_bytes
    }
}

pub async fn estimate_depot_batch(
    client: &SteamClient<LoggedIn>,
    entitlements: &PackageEntitlements,
    requests: &[DepotEstimateRequest],
) -> Result<Vec<DepotEstimateOutcome>, String> {
    let app_ids = requests
        .iter()
        .map(|request| AppId(request.app_id))
        .collect::<Vec<_>>();
    let mut access_tokens = client
        .pics_get_access_tokens(&app_ids)
        .await
        .map_err(|error| error.to_string())?;
    let token_app_ids = access_tokens
        .iter()
        .map(|token| token.app_id)
        .collect::<HashSet<_>>();
    access_tokens.extend(
        app_ids
            .iter()
            .filter(|app_id| !token_app_ids.contains(app_id))
            .map(|app_id| AccessToken {
                app_id: *app_id,
                token: 0,
            }),
    );
    let product_info = client
        .pics_get_product_info(&access_tokens)
        .await
        .map_err(|error| error.to_string())?;
    let mut product_buffers = product_info
        .into_iter()
        .filter_map(|info| Some((info.app_id?.0, info.kv_data?)))
        .collect::<BTreeMap<_, _>>();

    // Steam can split large PICS replies across messages. steamroom 0.3 returns
    // the first response, so retry any omitted app alone rather than treating
    // it as genuinely unavailable.
    let missing_app_ids = app_ids
        .iter()
        .filter(|app_id| !product_buffers.contains_key(&app_id.0))
        .copied()
        .collect::<Vec<_>>();
    for app_id in missing_app_ids {
        let token = access_tokens
            .iter()
            .find(|token| token.app_id == app_id)
            .cloned()
            .unwrap_or(AccessToken { app_id, token: 0 });
        if let Ok(infos) = client.pics_get_product_info(&[token]).await {
            product_buffers.extend(
                infos
                    .into_iter()
                    .filter_map(|info| Some((info.app_id?.0, info.kv_data?))),
            );
        }
    }

    Ok(requests
        .iter()
        .map(|request| {
            let result = product_buffers
                .get(&request.app_id)
                .ok_or_else(|| "Steam returned no PICS depot metadata".to_string())
                .and_then(|buffer| parse_app_depot_metadata(request.app_id, buffer))
                .and_then(|metadata| {
                    estimate_from_metadata(
                        &metadata,
                        &request.language,
                        &entitlements.for_game(request.app_id),
                    )
                });
            match result {
                Ok(estimate) => DepotEstimateOutcome::Available(estimate),
                Err(error) => DepotEstimateOutcome::Unavailable {
                    app_id: request.app_id,
                    error,
                },
            }
        })
        .collect())
}

fn estimate_from_metadata(
    metadata: &AppDepotMetadata,
    language: &str,
    game_entitlements: &GameEntitlements,
) -> Result<DepotEstimate, String> {
    estimate_from_metadata_for_os(metadata, language, game_entitlements, TargetOs::current())
}

fn estimate_from_metadata_for_os(
    metadata: &AppDepotMetadata,
    language: &str,
    game_entitlements: &GameEntitlements,
    current_os: TargetOs,
) -> Result<DepotEstimate, String> {
    let current_selection = select_for_game(metadata, language, game_entitlements, current_os);
    if has_compatible_base_game_depot(metadata, &current_selection) {
        return summarise_selection(metadata.app_id, current_selection, current_os, true);
    }

    if current_os != TargetOs::Windows {
        let windows_selection =
            select_for_game(metadata, language, game_entitlements, TargetOs::Windows);
        if has_compatible_base_game_depot(metadata, &windows_selection) {
            return summarise_selection(
                metadata.app_id,
                windows_selection,
                TargetOs::Windows,
                false,
            );
        }
    }

    ensure_compatible_base_game_depot(metadata, &current_selection)?;
    unreachable!("a valid base-game depot returned early")
}

fn summarise_selection(
    app_id: u32,
    selection: SelectionOutcome,
    target_os: TargetOs,
    current_os_supported: bool,
) -> Result<DepotEstimate, String> {
    let mut size_bytes = 0_u64;
    let mut depot_count = 0;
    for depot in &selection.selected {
        let bytes = depot.manifest.uncompressed_bytes.ok_or_else(|| {
            format!(
                "Depot {} does not publish an uncompressed manifest size",
                depot.depot_id
            )
        })?;
        if bytes == 0 {
            continue;
        }
        size_bytes = size_bytes
            .checked_add(bytes)
            .ok_or_else(|| "Selected depot sizes overflowed".to_string())?;
        depot_count += 1;
    }
    if size_bytes == 0 {
        return Err("Selected depots contain no installable content".to_string());
    }
    Ok(DepotEstimate {
        app_id,
        size_bytes,
        depot_count,
        target_os: target_os.steam_name().to_string(),
        current_os_supported,
        warnings: selection.warnings,
    })
}

pub async fn run_depot_probe(
    client: &SteamClient<LoggedIn>,
    entitlements: &PackageEntitlements,
) -> Result<DepotProbe, String> {
    let installed_apps = discover_installed_apps()?;
    let app = choose_probe_app(installed_apps)
        .ok_or_else(|| "No installed Steam app with depot metadata was found".to_string())?;
    let app_id = AppId(app.app_id);

    let mut access_tokens = client
        .pics_get_access_tokens(&[app_id])
        .await
        .map_err(|error| error.to_string())?;
    if access_tokens.is_empty() {
        access_tokens.push(AccessToken { app_id, token: 0 });
    }
    let product_info = client
        .pics_get_product_info(&access_tokens)
        .await
        .map_err(|error| error.to_string())?;
    let product_buffer = product_info
        .iter()
        .find(|info| info.app_id == Some(app_id))
        .and_then(|info| info.kv_data.as_ref())
        .ok_or_else(|| {
            "Steam returned no PICS product metadata for the installed app".to_string()
        })?;
    let product_metadata_bytes = product_buffer.len();
    let metadata = parse_app_depot_metadata(app.app_id, product_buffer)?;
    let game_entitlements = entitlements.for_game(app.app_id);
    let mut selection = select_for_game(
        &metadata,
        &app.language
            .clone()
            .unwrap_or_else(|| "english".to_string()),
        &game_entitlements,
        TargetOs::current(),
    );
    append_local_reconciliation_warnings(&app, &metadata, &mut selection);
    ensure_compatible_base_game_depot(&metadata, &selection)?;

    let servers = client
        .get_cdn_servers(CellId(0), Some(24))
        .await
        .map_err(|error| error.to_string())?;
    if servers.is_empty() {
        return Err("Steam returned no content servers".to_string());
    }

    let cdn = CdnClient::new().map_err(|error| error.to_string())?;
    let mut merged_files = BTreeMap::<String, u64>::new();
    let mut depot_results = Vec::new();

    for selected_depot in &selection.selected {
        let depot_id = DepotId(selected_depot.depot_id);
        let manifest_id = ManifestId(selected_depot.manifest.manifest_id);
        let source_app_id = AppId(selected_depot.source_app_id);
        let key = match client
            .get_depot_decryption_key(depot_id, source_app_id)
            .await
        {
            Ok(key) => key,
            Err(error) => {
                selection.warnings.push(format!(
                    "Depot {} was selected but Steam denied its key; it was omitted ({error})",
                    selected_depot.depot_id
                ));
                continue;
            }
        };
        let request_code = client
            .get_manifest_request_code(source_app_id, depot_id, manifest_id, Some("public"), None)
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or(0);

        let raw = download_manifest(
            client,
            &cdn,
            &servers,
            source_app_id,
            depot_id,
            manifest_id,
            request_code,
        )
        .await?;
        let mut manifest = parse_cdn_manifest(&raw).map_err(|error| error.to_string())?;
        manifest
            .decrypt_filenames(&key)
            .map_err(|error| error.to_string())?;

        let uncompressed_bytes = manifest
            .total_uncompressed_size
            .unwrap_or_else(|| manifest.files.iter().map(|file| file.size).sum());
        let compressed_bytes = manifest.total_compressed_size.unwrap_or_else(|| {
            manifest
                .files
                .iter()
                .flat_map(|file| &file.chunks)
                .filter_map(|chunk| chunk.compressed_size)
                .map(u64::from)
                .sum()
        });
        for file in &manifest.files {
            merge_manifest_file(&mut merged_files, file.normalized_path(), file.size);
        }
        let mut selection_reasons = selected_depot.reasons.clone();
        if let Some(dlc_app_id) = selected_depot.dlc_app_id {
            selection_reasons.retain(|reason| !reason.contains("entitlement requires"));
            selection_reasons.push(format!("DLC {dlc_app_id} access verified by Steam"));
            let package_ids = entitlements
                .app_packages
                .get(&dlc_app_id)
                .or_else(|| entitlements.depot_packages.get(&selected_depot.depot_id))
                .map(|packages| {
                    packages
                        .iter()
                        .filter(|package_id| game_entitlements.package_ids.contains(package_id))
                        .map(|package_id| {
                            let notes = entitlements
                                .package_notes
                                .get(package_id)
                                .map(|notes| notes.iter().cloned().collect::<Vec<_>>().join("; "))
                                .unwrap_or_else(|| "no licence metadata".to_string());
                            format!("{package_id} ({notes})")
                        })
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_else(|| "unknown".to_string());
            selection_reasons.push(format!("license package {package_ids}"));
        }
        depot_results.push(DepotResult {
            depot_id: selected_depot.depot_id,
            manifest_id: selected_depot.manifest.manifest_id,
            compressed_bytes,
            uncompressed_bytes,
            file_count: manifest.files.len(),
            selection_reasons,
        });
    }

    if depot_results.is_empty() {
        return Err("Steam denied access to every selected depot".to_string());
    }
    let installed_depot_ids = app
        .depots
        .iter()
        .map(|depot| depot.depot_id)
        .collect::<HashSet<_>>();
    for depot in &depot_results {
        if !installed_depot_ids.contains(&depot.depot_id) {
            selection.warnings.push(format!(
                "Current PICS depot {} was selected but is not mounted in the local app manifest",
                depot.depot_id
            ));
        }
    }

    Ok(DepotProbe {
        app_id: app.app_id,
        app_name: app.name,
        local_size_bytes: app.size_on_disk_bytes,
        product_metadata_bytes,
        merged_manifest_bytes: merged_files.values().sum(),
        selection_warnings: selection.warnings,
        depots: depot_results,
    })
}

fn select_for_game(
    metadata: &AppDepotMetadata,
    language: &str,
    game_entitlements: &GameEntitlements,
    os: TargetOs,
) -> SelectionOutcome {
    let mut selection = select_depots(
        metadata,
        &SelectionContext {
            os,
            architecture: TargetArchitecture::current(),
            language: language.to_ascii_lowercase(),
            entitled_app_ids: game_entitlements.app_ids.clone(),
            entitled_depot_ids: game_entitlements.depot_ids.clone(),
        },
    );
    if game_entitlements.shared_only {
        if game_entitlements.borrowed_owner.is_some() {
            selection.warnings.push(
                "This is a shared-only game; DLC follows Steam's selected family owner".to_string(),
            );
        } else {
            selection.warnings.push(format!(
                "This is a shared-only game with {} possible family owners; Steam did not identify one preferred owner",
                game_entitlements.ambiguous_borrowed_owners.len()
            ));
        }
    }
    selection
}

fn has_compatible_base_game_depot(
    metadata: &AppDepotMetadata,
    selection: &SelectionOutcome,
) -> bool {
    let selected_base_ids = selection
        .selected
        .iter()
        .filter(|depot| depot.dlc_app_id.is_none() && depot.manifest.uncompressed_bytes != Some(0))
        .map(|depot| depot.depot_id)
        .collect::<HashSet<_>>();
    if selected_base_ids.is_empty() {
        return false;
    }

    let has_os_specific_base = metadata.depots.iter().any(|depot| {
        depot.dlc_app_id.is_none()
            && depot.public_manifest.is_some()
            && !depot.config.os_list.is_empty()
    });
    !has_os_specific_base
        || metadata.depots.iter().any(|depot| {
            selected_base_ids.contains(&depot.depot_id) && !depot.config.os_list.is_empty()
        })
}

fn ensure_compatible_base_game_depot(
    metadata: &AppDepotMetadata,
    selection: &SelectionOutcome,
) -> Result<(), String> {
    if has_compatible_base_game_depot(metadata, selection) {
        return Ok(());
    }
    let reasons = selection
        .excluded
        .iter()
        .take(4)
        .map(|depot| format!("{}: {}", depot.depot_id, depot.reason))
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!(
        "No compatible OS-specific base-game public depot was selected ({reasons})"
    ))
}

fn append_local_reconciliation_warnings(
    app: &InstalledApp,
    metadata: &AppDepotMetadata,
    selection: &mut SelectionOutcome,
) {
    let metadata_depot_ids = metadata
        .depots
        .iter()
        .map(|depot| depot.depot_id)
        .collect::<HashSet<_>>();
    let installed_depot_ids = app
        .depots
        .iter()
        .map(|depot| depot.depot_id)
        .collect::<HashSet<_>>();

    for depot_id in installed_depot_ids.difference(&metadata_depot_ids) {
        selection.warnings.push(format!(
            "Locally mounted depot {depot_id} is absent from current PICS metadata"
        ));
    }
    for excluded in &selection.excluded {
        if installed_depot_ids.contains(&excluded.depot_id) {
            selection.warnings.push(format!(
                "Locally mounted depot {} was excluded: {}",
                excluded.depot_id, excluded.reason
            ));
        }
    }
}

async fn download_manifest(
    client: &SteamClient<LoggedIn>,
    cdn: &CdnClient,
    servers: &[steamroom::cdn::CdnServer],
    app_id: AppId,
    depot_id: DepotId,
    manifest_id: ManifestId,
    request_code: u64,
) -> Result<bytes::Bytes, String> {
    let mut last_error = None;
    for server in servers.iter().take(12) {
        let host = if server.vhost.is_empty() {
            &server.host
        } else {
            &server.vhost
        };
        let auth_token = client
            .get_cdn_auth_token(app_id, depot_id, host)
            .await
            .ok()
            .and_then(|token| token.token);
        match cdn
            .download_manifest(
                server,
                depot_id,
                manifest_id,
                request_code,
                auth_token.as_deref(),
            )
            .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    Err(last_error.unwrap_or_else(|| "Manifest download failed".to_string()))
}

fn merge_manifest_file(files: &mut BTreeMap<String, u64>, normalized_path: String, size: u64) {
    files.insert(normalized_path, size);
}

fn choose_probe_app(apps: Vec<InstalledApp>) -> Option<InstalledApp> {
    apps.into_iter().find(|app| !app.depots.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depot_metadata::{DepotConfig, DepotMetadata, ManifestReference};

    fn summary_depot(id: u32, size: Option<u64>) -> DepotMetadata {
        DepotMetadata {
            depot_id: id,
            name: None,
            config: DepotConfig::default(),
            public_manifest: Some(ManifestReference {
                manifest_id: u64::from(id),
                uncompressed_bytes: size,
                compressed_bytes: None,
            }),
            dlc_app_id: None,
            depot_from_app: None,
            shared_install: false,
        }
    }

    #[test]
    fn probe_prefers_an_app_with_installed_depots() {
        let chosen = choose_probe_app(vec![
            InstalledApp {
                app_id: 1,
                name: "No depots".to_string(),
                size_on_disk_bytes: 0,
                language: None,
                depots: vec![],
            },
            InstalledApp {
                app_id: 2,
                name: "Probe".to_string(),
                size_on_disk_bytes: 10,
                language: None,
                depots: vec![crate::local_steam::InstalledDepot {
                    depot_id: 3,
                    manifest_id: 4,
                    recorded_size_bytes: Some(10),
                }],
            },
        ])
        .expect("an app should be selected");
        assert_eq!(chosen.app_id, 2);
    }

    #[test]
    fn merged_size_counts_overlapping_paths_once() {
        let mut files = BTreeMap::new();
        merge_manifest_file(&mut files, "shared/data.bin".to_string(), 100);
        merge_manifest_file(&mut files, "platform/mac.bin".to_string(), 40);
        merge_manifest_file(&mut files, "shared/data.bin".to_string(), 100);

        assert_eq!(files.values().sum::<u64>(), 140);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn library_estimate_sums_steam_uncompressed_manifest_summaries() {
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![summary_depot(1, Some(100)), summary_depot(2, Some(40))],
        };

        let estimate = estimate_from_metadata(&metadata, "english", &Default::default())
            .expect("summary sizes should produce an estimate");

        assert_eq!(estimate.app_id, 42);
        assert_eq!(estimate.size_bytes, 140);
        assert_eq!(estimate.depot_count, 2);
        assert!(estimate.current_os_supported);
        assert_eq!(estimate.target_os, TargetOs::current().steam_name());
    }

    #[test]
    fn platform_neutral_dlc_cannot_masquerade_as_a_base_game_install() {
        let mut windows_base = summary_depot(1, Some(100));
        windows_base.config.os_list = vec!["windows".to_string()];
        let mut neutral_dlc = summary_depot(2, Some(40));
        neutral_dlc.dlc_app_id = Some(500);
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![windows_base, neutral_dlc],
        };
        let entitlements = GameEntitlements {
            app_ids: HashSet::from([500]),
            ..Default::default()
        };

        let estimate =
            estimate_from_metadata_for_os(&metadata, "english", &entitlements, TargetOs::MacOs)
                .expect("Windows base game should provide the fallback estimate");

        assert_eq!(estimate.size_bytes, 140);
        assert_eq!(estimate.target_os, "windows");
        assert!(!estimate.current_os_supported);
    }

    #[test]
    fn neutral_helper_depot_cannot_replace_an_os_specific_base_depot() {
        let neutral_helper = summary_depot(1, Some(13 * 1024));
        let mut windows_base = summary_depot(2, Some(100 * 1024 * 1024));
        windows_base.config.os_list = vec!["windows".to_string()];
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![neutral_helper, windows_base],
        };

        let estimate = estimate_from_metadata_for_os(
            &metadata,
            "english",
            &Default::default(),
            TargetOs::MacOs,
        )
        .expect("neutral helper plus Windows base should use Windows");

        assert_eq!(estimate.target_os, "windows");
        assert!(!estimate.current_os_supported);
        assert_eq!(estimate.size_bytes, 100 * 1024 * 1024 + 13 * 1024);
    }

    #[test]
    fn zero_byte_windows_marker_does_not_hide_installable_windows_depots() {
        let mut content = summary_depot(1, Some(52_363_996_299));
        content.config.os_list = vec!["windows".to_string()];
        let mut supporting_content = summary_depot(2, Some(1_489_009_011));
        supporting_content.config.os_list = vec!["windows".to_string()];
        let mut zero_byte_marker = summary_depot(3, Some(0));
        zero_byte_marker.config.os_list = vec!["windows".to_string()];
        let metadata = AppDepotMetadata {
            app_id: 2_215_430,
            depots: vec![zero_byte_marker, content, supporting_content],
        };

        let estimate = estimate_from_metadata_for_os(
            &metadata,
            "english",
            &Default::default(),
            TargetOs::MacOs,
        )
        .expect("non-zero Windows content depots should provide the fallback estimate");

        assert_eq!(estimate.target_os, "windows");
        assert!(!estimate.current_os_supported);
        assert_eq!(estimate.size_bytes, 53_853_005_310);
        assert_eq!(estimate.depot_count, 2);
    }

    #[test]
    fn zero_byte_os_marker_is_not_compatible_installable_content() {
        let mut zero_byte_marker = summary_depot(1, Some(0));
        zero_byte_marker.config.os_list = vec!["windows".to_string()];
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![zero_byte_marker],
        };

        let error = estimate_from_metadata_for_os(
            &metadata,
            "english",
            &Default::default(),
            TargetOs::MacOs,
        )
        .expect_err("a zero-byte marker must not establish Windows compatibility");

        assert!(error.contains("No compatible OS-specific base-game"));
    }

    #[test]
    fn library_estimate_does_not_turn_missing_summary_sizes_into_zero() {
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![summary_depot(1, None)],
        };

        let error = estimate_from_metadata(&metadata, "english", &Default::default())
            .expect_err("missing summary size should remain unavailable");

        assert!(error.contains("does not publish"));
    }

    #[test]
    fn local_reconciliation_explains_absent_and_excluded_depots() {
        let app = InstalledApp {
            app_id: 42,
            name: "Fixture".to_string(),
            size_on_disk_bytes: 100,
            language: Some("english".to_string()),
            depots: vec![
                crate::local_steam::InstalledDepot {
                    depot_id: 1,
                    manifest_id: 10,
                    recorded_size_bytes: Some(50),
                },
                crate::local_steam::InstalledDepot {
                    depot_id: 2,
                    manifest_id: 20,
                    recorded_size_bytes: Some(50),
                },
            ],
        };
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![crate::depot_metadata::DepotMetadata {
                depot_id: 1,
                name: None,
                config: Default::default(),
                public_manifest: None,
                dlc_app_id: None,
                depot_from_app: None,
                shared_install: false,
            }],
        };
        let mut selection = SelectionOutcome {
            excluded: vec![crate::depot_selection::ExcludedDepot {
                depot_id: 1,
                reason: "no public-branch manifest".to_string(),
            }],
            ..Default::default()
        };

        append_local_reconciliation_warnings(&app, &metadata, &mut selection);

        assert!(
            selection
                .warnings
                .iter()
                .any(|warning| warning.contains("depot 2") && warning.contains("absent"))
        );
        assert!(
            selection
                .warnings
                .iter()
                .any(|warning| warning.contains("depot 1") && warning.contains("excluded"))
        );
    }
}
