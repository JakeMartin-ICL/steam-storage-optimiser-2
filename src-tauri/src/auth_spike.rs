use crate::app_identity_cache::AppIdentityCache;
use crate::community_contribution_cache::CommunityContributionCache;
use crate::community_sizes::CommunitySizeClient;
use crate::credential_store::{
    SavedLogin, delete_saved_login, has_saved_login as session_is_saved, load_saved_login,
    save_login,
};
use crate::depot_cache::DepotCache;
use crate::depot_probe::{
    DepotEstimate, DepotEstimateOutcome, DepotEstimateRequest, DepotProbe, estimate_depot_batch,
    run_depot_probe,
};
use crate::depot_selection::TargetOs;
use crate::hltb::{HltbCandidate, HltbClient, HltbError, select_match};
use crate::hltb_cache::HltbCache;
use crate::local_steam::{InstalledApp, discover_installed_apps, discover_local_playtimes};
use crate::package_entitlements::resolve_package_entitlements;
use crate::steam_library::{
    DepotEstimateStatus, GamePreview, HltbStatus, SteamProfile, get_account_packages,
    get_app_identities, get_owned_games, get_player_profile,
};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::Serialize;
use std::sync::Arc;
use std::time::SystemTime;
use steamroom_client::login::LoginBuilder;
use tauri::State;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthView {
    phase: String,
    message: String,
    qr_image: Option<String>,
    library_count: Option<usize>,
    games: Vec<GamePreview>,
    probe: Option<DepotProbe>,
    error: Option<String>,
    saved_login: bool,
    community_error: Option<String>,
    depot_error: Option<String>,
    depot_progress: Option<DepotProgress>,
    hltb_error: Option<String>,
    hltb_progress: Option<HltbProgress>,
    profile: Option<SteamProfile>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepotProgress {
    completed: usize,
    total: usize,
    available: usize,
    unavailable: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HltbProgress {
    completed: usize,
    total: usize,
    matched: usize,
    unmatched: usize,
}

impl Default for AuthView {
    fn default() -> Self {
        Self {
            phase: "idle".to_string(),
            message: "Ready to test a secure local Steam sign-in.".to_string(),
            qr_image: None,
            library_count: None,
            games: Vec::new(),
            probe: None,
            error: None,
            saved_login: false,
            community_error: None,
            depot_error: None,
            depot_progress: None,
            hltb_error: None,
            hltb_progress: None,
            profile: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct SpikeState {
    view: Arc<RwLock<AuthView>>,
    task: Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    hltb_task: Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    hltb_cache_gate: Arc<Mutex<()>>,
}

#[tauri::command]
pub async fn get_auth_state(state: State<'_, SpikeState>) -> Result<AuthView, String> {
    Ok(state.view.read().await.clone())
}

#[tauri::command]
pub async fn has_saved_login() -> Result<bool, String> {
    Ok(session_is_saved().await)
}

#[tauri::command]
pub async fn start_qr_login(
    state: State<'_, SpikeState>,
    contribute_community_sizes: Option<bool>,
) -> Result<(), String> {
    let contribute_community_sizes = contribute_community_sizes.unwrap_or(true);
    let state = state.inner().clone();
    let mut task_slot = state.task.lock().await;
    if let Some(task) = task_slot.take() {
        task.abort();
    }
    if let Some(task) = state.hltb_task.lock().await.take() {
        task.abort();
    }
    drop(task_slot);
    set_view(&state, |view| {
        *view = AuthView {
            phase: "connecting".to_string(),
            message: "Connecting directly to Steam…".to_string(),
            ..Default::default()
        };
    })
    .await;

    let task_state = state.clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = run_login_spike(&task_state, contribute_community_sizes).await {
            #[cfg(debug_assertions)]
            eprintln!("STEAM_SPIKE_ERROR={}", sanitise_error(&error));
            set_view(&task_state, |view| {
                view.phase = "error".to_string();
                view.message = "The feasibility check stopped.".to_string();
                view.qr_image = None;
                view.error = Some(sanitise_error(&error));
            })
            .await;
        }
    });
    *state.task.lock().await = Some(task);
    Ok(())
}

#[tauri::command]
pub async fn cancel_qr_login(state: State<'_, SpikeState>) -> Result<(), String> {
    if let Some(task) = state.task.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.hltb_task.lock().await.take() {
        task.abort();
    }
    *state.view.write().await = AuthView::default();
    Ok(())
}

#[tauri::command]
pub async fn forget_saved_login(state: State<'_, SpikeState>) -> Result<(), String> {
    if let Some(task) = state.task.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.hltb_task.lock().await.take() {
        task.abort();
    }
    delete_saved_login().await?;
    *state.view.write().await = AuthView::default();
    Ok(())
}

#[tauri::command]
pub async fn search_hltb(query: String) -> Result<Vec<HltbCandidate>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let client = loop {
        match HltbClient::new().await {
            Ok(client) => break client,
            Err(HltbError::RateLimited {
                retry_after: Some(delay),
                ..
            }) => tokio::time::sleep(delay.max(std::time::Duration::from_secs(1))).await,
            Err(error) => return Err(error.to_string()),
        }
    };
    loop {
        match client.search(query, query).await {
            Ok(candidates) => return Ok(candidates),
            Err(HltbError::RateLimited {
                retry_after: Some(delay),
                ..
            }) => tokio::time::sleep(delay.max(std::time::Duration::from_secs(1))).await,
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[tauri::command]
pub async fn set_hltb_match(
    state: State<'_, SpikeState>,
    app_id: u32,
    candidate: Option<HltbCandidate>,
) -> Result<(), String> {
    let estimate = candidate.map(|candidate| candidate.to_estimate("manual"));
    {
        let _gate = state.hltb_cache_gate.lock().await;
        let mut cache = HltbCache::load().await;
        cache.insert(app_id, estimate.clone(), true, SystemTime::now());
        cache.save().await?;
    }
    set_view(state.inner(), |view| {
        if let Some(game) = view.games.iter_mut().find(|game| game.app_id == app_id) {
            game.hltb = estimate;
            game.hltb_status = if game.hltb.is_some() {
                HltbStatus::Matched
            } else {
                HltbStatus::Unmatched
            };
        }
    })
    .await;
    Ok(())
}

async fn run_login_spike(
    state: &SpikeState,
    contribute_community_sizes: bool,
) -> Result<(), String> {
    if let Some(saved) = load_saved_login().await? {
        set_view(state, |view| {
            view.phase = "approved".to_string();
            view.message = "Reusing the development session stored in local app data…".to_string();
            view.saved_login = true;
        })
        .await;
        let steam_id = steam_id_from_token(&saved.refresh_token);
        let login = LoginBuilder::new()
            .device_name("Steam Storage Optimiser")
            .prefer_protocol(steamroom::connection::Protocol::WebSocket)
            .allow_protocol_fallback(false)
            .with_refresh_token(saved.account_name, saved.refresh_token)
            .login()
            .await;
        if let (Ok(steam_id), Ok(client)) = (steam_id, login) {
            return run_authenticated_checks(state, client, steam_id, contribute_community_sizes)
                .await;
        }

        delete_saved_login().await?;
        set_view(state, |view| {
            view.message = "The saved login expired. Requesting a fresh QR approval…".to_string();
            view.saved_login = false;
        })
        .await;
    }

    let flow = LoginBuilder::new()
        .device_name("Steam Storage Optimiser")
        .prefer_protocol(steamroom::connection::Protocol::WebSocket)
        .allow_protocol_fallback(false)
        .with_qr()
        .begin()
        .await
        .map_err(|error| error.to_string())?;

    let qr_image = qr_data_url(flow.challenge_url())?;
    set_view(state, |view| {
        view.phase = "qr_ready".to_string();
        view.message = "Scan with Steam Mobile, then approve this device.".to_string();
        view.qr_image = Some(qr_image);
    })
    .await;

    let approved = flow
        .wait_for_scan()
        .await
        .map_err(|error| error.to_string())?;
    let steam_id = steam_id_from_token(&approved.tokens().access_token)?;
    let saved_login = SavedLogin {
        account_name: approved
            .tokens()
            .account_name
            .clone()
            .ok_or_else(|| "Steam did not return an account name".to_string())?,
        refresh_token: approved.tokens().refresh_token.clone(),
    };

    set_view(state, |view| {
        view.phase = "approved".to_string();
        view.message = "Approved. Establishing a short-lived client session…".to_string();
        view.qr_image = None;
    })
    .await;

    let client = approved.finish().await.map_err(|error| error.to_string())?;
    save_login(saved_login).await?;
    set_view(state, |view| {
        view.saved_login = true;
    })
    .await;

    run_authenticated_checks(state, client, steam_id, contribute_community_sizes).await
}

async fn run_authenticated_checks(
    state: &SpikeState,
    client: steamroom::client::SteamClient<steamroom::client::LoggedIn>,
    steam_id: u64,
    contribute_community_sizes: bool,
) -> Result<(), String> {
    set_view(state, |view| {
        view.phase = "fetching_library".to_string();
        view.message = "Loading account licenses and package entitlements…".to_string();
    })
    .await;
    let packages = get_account_packages(&client, steam_id as u32).await?;
    let entitlements = resolve_package_entitlements(&client, &packages).await?;
    let profile = get_player_profile(&client, steam_id).await.ok();
    #[cfg(debug_assertions)]
    eprintln!(
        "STEAM_PROFILE_DIAGNOSTIC={:?}",
        profile
            .as_ref()
            .map(|profile| (&profile.display_name, profile.avatar_url.is_some()))
    );

    set_view(state, |view| {
        view.message = "Loading owned games and lifetime playtime…".to_string();
    })
    .await;
    let shared_only_app_ids = entitlements.shared_only_app_ids();
    let mut library = get_owned_games(&client, steam_id, &shared_only_app_ids).await?;
    set_view(state, |view| {
        view.message = "Classifying Steam games, software, and tools…".to_string();
    })
    .await;
    let mut identity_app_ids = shared_only_app_ids.clone();
    identity_app_ids.extend(library.preview.iter().map(|game| game.app_id));
    let now = SystemTime::now();
    let mut app_identity_cache = AppIdentityCache::load().await;
    let mut app_identities = Vec::with_capacity(identity_app_ids.len());
    let mut uncached_app_ids = std::collections::HashSet::new();
    for app_id in &identity_app_ids {
        if let Some(identity) = app_identity_cache.get(*app_id) {
            app_identities.push(identity);
        } else {
            uncached_app_ids.insert(*app_id);
        }
    }
    let fetched_identities = get_app_identities(&client, &uncached_app_ids).await?;
    for identity in &fetched_identities {
        app_identity_cache.insert(identity, now);
    }
    if !fetched_identities.is_empty()
        && let Err(error) = app_identity_cache.save().await
    {
        #[cfg(debug_assertions)]
        eprintln!("APP_TYPE_CACHE_ERROR={error}");
    }
    app_identities.extend(fetched_identities);
    app_identities.sort_by_key(|identity| identity.app_id);
    library.apply_app_identities(&app_identities);
    #[cfg(debug_assertions)]
    let shared_games = app_identities
        .iter()
        .filter(|app| shared_only_app_ids.contains(&app.app_id) && app.app_type == "game")
        .collect::<Vec<_>>();
    let local_playtimes = discover_local_playtimes(steam_id as u32).unwrap_or_default();
    library.merge_shared_games(&app_identities, &local_playtimes);
    let installed_apps = discover_installed_apps().unwrap_or_default();
    for game in &mut library.preview {
        if let Some(installed) = installed_apps
            .iter()
            .find(|installed| installed.app_id == game.app_id)
        {
            game.installed = true;
            game.local_size_bytes = Some(installed.size_on_disk_bytes);
        }
    }
    #[cfg(debug_assertions)]
    {
        let pics_shared_sample = shared_games
            .iter()
            .take(12)
            .map(|app| {
                (
                    app.app_id,
                    app.name.as_str(),
                    local_playtimes.get(&app.app_id).copied(),
                )
            })
            .collect::<Vec<_>>();
        eprintln!(
            "STEAM_LIBRARY_DIAGNOSTIC={{\"mergedGameCount\":{},\"sharedOnlyGameCount\":{},\"borrowedAppCandidates\":{},\"picsSharedGames\":{},\"picsSharedSample\":{:?}}}",
            library.count,
            library.shared_only_count,
            shared_only_app_ids.len(),
            shared_games.len(),
            pics_shared_sample
        );
    }
    set_view(state, |view| {
        view.message = "Loading community size comparisons…".to_string();
    })
    .await;
    let app_ids = library
        .preview
        .iter()
        .map(|game| game.app_id)
        .collect::<Vec<_>>();
    let (community_sizes, community_error) = match CommunitySizeClient::new() {
        Ok(community) => match community.get_sizes(&app_ids).await {
            Ok(sizes) => (sizes, None),
            Err(error) => (Default::default(), Some(sanitise_error(&error))),
        },
        Err(error) => (Default::default(), Some(sanitise_error(&error))),
    };
    for game in &mut library.preview {
        game.community_size_bytes = community_sizes.get(&game.app_id).map(|size| size.size);
    }

    let hltb_cache = HltbCache::load().await;
    let now = SystemTime::now();
    let mut hltb_progress = HltbProgress {
        total: library
            .preview
            .iter()
            .filter(|game| game.app_type == "game" || game.app_type == "unknown")
            .count(),
        ..Default::default()
    };
    let mut pending_hltb = Vec::new();
    for game in &mut library.preview {
        if game.app_type != "game" && game.app_type != "unknown" {
            game.hltb_status = HltbStatus::NotApplicable;
            continue;
        }
        let cached = hltb_cache.get(game.app_id);
        if let Some(cached) = cached {
            game.hltb = cached.estimate.clone();
            if cached.estimate.is_some() {
                game.hltb_status = HltbStatus::Matched;
            }
            if HltbCache::is_fresh(cached, now) {
                hltb_progress.completed += 1;
                if cached.estimate.is_some() {
                    hltb_progress.matched += 1;
                } else {
                    hltb_progress.unmatched += 1;
                    game.hltb_status = HltbStatus::Unmatched;
                }
                continue;
            }
        }
        pending_hltb.push((
            game.app_id,
            game.name.clone(),
            cached
                .filter(|entry| entry.manual)
                .and_then(|entry| entry.estimate.as_ref())
                .map(|estimate| estimate.game_id),
        ));
    }

    let requests = library
        .preview
        .iter()
        .map(|game| DepotEstimateRequest {
            app_id: game.app_id,
            language: installed_apps
                .iter()
                .find(|installed| installed.app_id == game.app_id)
                .and_then(|installed| installed.language.clone())
                .unwrap_or_else(|| "english".to_string()),
        })
        .collect::<Vec<_>>();
    let mut depot_cache = DepotCache::load(steam_id, entitlements.cache_fingerprint()).await;
    let mut pending_requests = Vec::new();
    let mut progress = DepotProgress {
        total: requests.len(),
        ..Default::default()
    };
    for request in &requests {
        if let Some(estimate) = depot_cache.get(request, SystemTime::now()) {
            apply_depot_estimate(&mut library.preview, &estimate);
            progress.available += 1;
            progress.completed += 1;
        } else {
            pending_requests.push(request.clone());
        }
    }

    set_view(state, |view| {
        view.library_count = Some(library.count);
        view.games = library.preview.clone();
        view.community_error = community_error.clone();
        view.profile = profile.clone();
        view.phase = "complete".to_string();
        view.message = if pending_requests.is_empty() {
            "Library ready. Steam depot estimates loaded from the local cache.".to_string()
        } else if progress.completed > 0 {
            format!(
                "Library ready. Loaded {} cached depot estimates; refreshing {}.",
                progress.completed,
                pending_requests.len()
            )
        } else {
            "Library ready. Measuring Steam depot estimates…".to_string()
        };
        view.depot_progress = Some(progress.clone());
        view.hltb_progress = Some(hltb_progress.clone());
    })
    .await;

    if !pending_hltb.is_empty() {
        let task_state = state.clone();
        let task_progress = hltb_progress.clone();
        let task = tauri::async_runtime::spawn(async move {
            run_hltb_enrichment(&task_state, pending_hltb, task_progress).await;
        });
        *state.hltb_task.lock().await = Some(task);
    }

    if contribute_community_sizes && community_error.is_none() {
        let installed_apps = installed_apps.clone();
        let community_sizes = community_sizes.clone();
        tauri::async_runtime::spawn(async move {
            match contribute_changed_install_sizes(installed_apps, community_sizes).await {
                #[cfg(debug_assertions)]
                Ok((changed, submitted)) => eprintln!(
                    "STEAM_COMMUNITY_CONTRIBUTION={{\"changed\":{changed},\"submitted\":{submitted}}}"
                ),
                #[cfg(not(debug_assertions))]
                Ok(_) => {}
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "STEAM_COMMUNITY_CONTRIBUTION_ERROR={}",
                        sanitise_error(&error)
                    );
                }
            }
        });
    }

    #[cfg(debug_assertions)]
    let depot_started_at = std::time::Instant::now();
    let mut last_batch_error = None;
    for batch in pending_requests.chunks(10) {
        let outcomes = match estimate_depot_batch(&client, &entitlements, batch).await {
            Ok(outcomes) => outcomes,
            Err(error) => {
                last_batch_error = Some(sanitise_error(&error));
                batch
                    .iter()
                    .map(|request| DepotEstimateOutcome::Unavailable {
                        app_id: request.app_id,
                        error: sanitise_error(&error),
                    })
                    .collect()
            }
        };
        for outcome in outcomes {
            match outcome {
                DepotEstimateOutcome::Available(estimate) => {
                    if let Some(request) = batch
                        .iter()
                        .find(|request| request.app_id == estimate.app_id)
                    {
                        depot_cache.insert(request, &estimate, SystemTime::now());
                    }
                    apply_depot_estimate(&mut library.preview, &estimate);
                    progress.available += 1;
                }
                DepotEstimateOutcome::Unavailable { app_id, error } => {
                    if let Some(game) = library
                        .preview
                        .iter_mut()
                        .find(|game| game.app_id == app_id)
                    {
                        game.depot_status = DepotEstimateStatus::Unavailable;
                        game.depot_error = Some(sanitise_error(&error));
                    }
                    progress.unavailable += 1;
                }
            }
            progress.completed += 1;
        }
        if let Err(error) = depot_cache.save().await {
            #[cfg(debug_assertions)]
            eprintln!("STEAM_DEPOT_CACHE_ERROR={}", sanitise_error(&error));
        }
        set_view(state, |view| {
            merge_depot_fields(&mut view.games, &library.preview);
            view.depot_progress = Some(progress.clone());
            view.message = format!(
                "Measured Steam depot estimates for {} of {} games.",
                progress.completed, progress.total
            );
        })
        .await;
        #[cfg(debug_assertions)]
        eprintln!(
            "STEAM_DEPOT_PROGRESS={{\"completed\":{},\"total\":{},\"available\":{},\"unavailable\":{},\"elapsedMs\":{}}}",
            progress.completed,
            progress.total,
            progress.available,
            progress.unavailable,
            depot_started_at.elapsed().as_millis()
        );
    }
    #[cfg(debug_assertions)]
    {
        let unavailable_sample = library
            .preview
            .iter()
            .filter_map(|game| {
                game.depot_error
                    .as_deref()
                    .map(|error| (game.app_id, game.name.as_str(), error))
            })
            .take(12)
            .collect::<Vec<_>>();
        let installed_comparisons = library
            .preview
            .iter()
            .filter(|game| game.installed)
            .filter_map(|game| {
                Some((
                    game.app_id,
                    game.name.as_str(),
                    game.local_size_bytes?,
                    game.depot_size_bytes?,
                ))
            })
            .collect::<Vec<_>>();
        let windows_fallback_sample = library
            .preview
            .iter()
            .filter(|game| game.current_os_supported == Some(false))
            .map(|game| (game.app_id, game.name.as_str(), game.depot_size_bytes))
            .take(12)
            .collect::<Vec<_>>();
        let windows_fallback_count = library
            .preview
            .iter()
            .filter(|game| game.current_os_supported == Some(false))
            .count();
        let arkham_knight = library
            .preview
            .iter()
            .find(|game| game.app_id == 208650)
            .map(|game| {
                (
                    game.depot_os.as_deref(),
                    game.current_os_supported,
                    game.depot_size_bytes,
                    game.depot_count,
                )
            });
        let grounded = library
            .preview
            .iter()
            .find(|game| game.app_id == 962130)
            .map(|game| {
                (
                    game.depot_os.as_deref(),
                    game.current_os_supported,
                    game.depot_size_bytes,
                    game.depot_count,
                )
            });
        eprintln!(
            "STEAM_DEPOT_LIBRARY_DIAGNOSTIC={{\"available\":{},\"unavailable\":{},\"windowsFallbacks\":{},\"elapsedMs\":{},\"arkhamKnight\":{:?},\"grounded\":{:?},\"unavailableSample\":{:?},\"windowsFallbackSample\":{:?},\"installedComparisons\":{:?}}}",
            progress.available,
            progress.unavailable,
            windows_fallback_count,
            depot_started_at.elapsed().as_millis(),
            arkham_knight,
            grounded,
            unavailable_sample,
            windows_fallback_sample,
            installed_comparisons
        );
    }

    let (probe, probe_error) = match run_depot_probe(&client, &entitlements).await {
        Ok(probe) => {
            if let Some(game) = library
                .preview
                .iter_mut()
                .find(|game| game.app_id == probe.app_id())
            {
                game.depot_size_bytes = Some(probe.merged_manifest_bytes());
                game.depot_status = DepotEstimateStatus::Available;
                game.depot_exact = true;
                game.depot_os = Some(TargetOs::current().steam_name().to_string());
                game.current_os_supported = Some(true);
            }
            (Some(probe), None)
        }
        Err(error) => (None, Some(sanitise_error(&error))),
    };
    #[cfg(debug_assertions)]
    if let Some(probe) = &probe {
        eprintln!(
            "STEAM_PROVENANCE_DIAGNOSTIC={}",
            serde_json::to_string(probe)
                .unwrap_or_else(|_| "{\"error\":\"could not encode diagnostic\"}".to_string())
        );
    }
    set_view(state, |view| {
        view.message = format!(
            "Library updated. Steam depot estimates are available for {} of {} games.",
            progress.available, progress.total
        );
        merge_depot_fields(&mut view.games, &library.preview);
        view.probe = probe;
        view.depot_error = if progress.available == 0 {
            last_batch_error.or(probe_error)
        } else {
            None
        };
        view.depot_progress = Some(progress);
    })
    .await;
    Ok(())
}

fn apply_depot_estimate(games: &mut [GamePreview], estimate: &DepotEstimate) {
    if let Some(game) = games.iter_mut().find(|game| game.app_id == estimate.app_id) {
        game.depot_size_bytes = Some(estimate.size_bytes);
        game.depot_status = DepotEstimateStatus::Available;
        game.depot_count = Some(estimate.depot_count);
        game.depot_os = Some(estimate.target_os.clone());
        game.current_os_supported = Some(estimate.current_os_supported);
        game.depot_warnings = estimate.warnings.clone();
        game.depot_error = None;
    }
}

fn merge_depot_fields(target: &mut [GamePreview], source: &[GamePreview]) {
    for source_game in source {
        if let Some(target_game) = target
            .iter_mut()
            .find(|game| game.app_id == source_game.app_id)
        {
            target_game.depot_size_bytes = source_game.depot_size_bytes;
            target_game.depot_status = source_game.depot_status;
            target_game.depot_exact = source_game.depot_exact;
            target_game.depot_count = source_game.depot_count;
            target_game.depot_os = source_game.depot_os.clone();
            target_game.current_os_supported = source_game.current_os_supported;
            target_game.depot_warnings = source_game.depot_warnings.clone();
            target_game.depot_error = source_game.depot_error.clone();
        }
    }
}

async fn run_hltb_enrichment(
    state: &SpikeState,
    pending: Vec<(u32, String, Option<i64>)>,
    mut progress: HltbProgress,
) {
    let mut connection_attempts = 0_u32;
    let client = loop {
        match HltbClient::new().await {
            Ok(client) => break client,
            Err(HltbError::RateLimited {
                retry_after,
                message,
            }) => {
                connection_attempts += 1;
                let delay = retry_after.unwrap_or_else(|| {
                    std::time::Duration::from_secs(
                        60_u64.saturating_mul(2_u64.pow(connection_attempts - 1)),
                    )
                });
                set_view(state, |view| {
                    view.hltb_error = Some(format!(
                        "{message}. Resuming in {} seconds.",
                        delay.as_secs()
                    ));
                })
                .await;
                if retry_after.is_none() && connection_attempts >= 3 {
                    return;
                }
                tokio::time::sleep(delay.max(std::time::Duration::from_secs(1))).await;
            }
            Err(error) => {
                set_view(state, |view| {
                    view.hltb_error = Some(sanitise_error(&error.to_string()))
                })
                .await;
                return;
            }
        }
    };
    let mut first_request = true;
    for (app_id, name, queued_preferred_id) in pending {
        if !first_request {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        first_request = false;

        let mut rate_limit_attempts = 0_u32;
        let candidates = loop {
            match client.search(&name, &name).await {
                Ok(candidates) => {
                    set_view(state, |view| view.hltb_error = None).await;
                    break candidates;
                }
                Err(HltbError::RateLimited {
                    retry_after,
                    message,
                }) => {
                    rate_limit_attempts += 1;
                    let delay = retry_after.unwrap_or_else(|| {
                        std::time::Duration::from_secs(
                            60_u64.saturating_mul(2_u64.pow(rate_limit_attempts - 1)),
                        )
                    });
                    set_view(state, |view| {
                        view.hltb_error = Some(format!(
                            "{message}. Resuming in {} seconds.",
                            delay.as_secs()
                        ));
                    })
                    .await;
                    if retry_after.is_none() && rate_limit_attempts >= 3 {
                        return;
                    }
                    tokio::time::sleep(delay.max(std::time::Duration::from_secs(1))).await;
                }
                Err(error) => {
                    set_view(state, |view| {
                        view.hltb_error = Some(sanitise_error(&error.to_string()));
                    })
                    .await;
                    return;
                }
            }
        };

        let (preferred_id, preserve_manual) = {
            let _gate = state.hltb_cache_gate.lock().await;
            let cache = HltbCache::load().await;
            let current = cache.get(app_id);
            let current_preferred = current
                .filter(|entry| entry.manual)
                .and_then(|entry| entry.estimate.as_ref())
                .map(|estimate| estimate.game_id);
            let manual_changed = current.is_some_and(|entry| {
                entry.manual
                    && (entry.estimate.is_none() || current_preferred != queued_preferred_id)
            });
            (current_preferred.or(queued_preferred_id), manual_changed)
        };

        let mut estimate = select_match(&candidates, app_id, preferred_id);
        let mut manual = preferred_id.is_some();
        if preserve_manual {
            let _gate = state.hltb_cache_gate.lock().await;
            let cache = HltbCache::load().await;
            estimate = cache.get(app_id).and_then(|entry| entry.estimate.clone());
            manual = true;
        } else if preferred_id.is_some() && estimate.is_none() {
            let _gate = state.hltb_cache_gate.lock().await;
            let cache = HltbCache::load().await;
            estimate = cache.get(app_id).and_then(|entry| entry.estimate.clone());
        }

        let save_result = {
            let _gate = state.hltb_cache_gate.lock().await;
            let mut cache = HltbCache::load().await;
            cache.insert(app_id, estimate.clone(), manual, SystemTime::now());
            cache.save().await
        };
        if let Err(error) = save_result {
            set_view(state, |view| view.hltb_error = Some(sanitise_error(&error))).await;
        }

        progress.completed += 1;
        if estimate.is_some() {
            progress.matched += 1;
        } else {
            progress.unmatched += 1;
        }
        set_view(state, |view| {
            if let Some(game) = view.games.iter_mut().find(|game| game.app_id == app_id) {
                game.hltb = estimate.clone();
                game.hltb_status = if estimate.is_some() {
                    HltbStatus::Matched
                } else {
                    HltbStatus::Unmatched
                };
            }
            view.hltb_progress = Some(progress.clone());
        })
        .await;
    }
}

async fn contribute_changed_install_sizes(
    installed_apps: Vec<InstalledApp>,
    community_sizes: std::collections::BTreeMap<u32, crate::community_sizes::CommunitySize>,
) -> Result<(usize, usize), String> {
    let client = CommunitySizeClient::new()?;
    let mut cache = CommunityContributionCache::load().await;
    let mut changed = 0;
    let mut submitted = 0;
    let mut cache_changed = false;
    let mut first_error = None;
    for app in installed_apps {
        if !cache.has_changed(app.app_id, app.size_on_disk_bytes) {
            continue;
        }
        changed += 1;
        let current_size = community_sizes.get(&app.app_id).map(|record| record.size);
        match client
            .contribute_installed_size(app.app_id, app.size_on_disk_bytes, &app.name, current_size)
            .await
        {
            Ok(was_submitted) => {
                submitted += usize::from(was_submitted);
                cache.record(app.app_id, app.size_on_disk_bytes);
                cache_changed = true;
            }
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    if cache_changed {
        cache.save().await?;
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok((changed, submitted))
    }
}

fn steam_id_from_token(token: &str) -> Result<u64, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "Steam returned an unrecognised access token".to_string())?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "Steam returned an unrecognised access token".to_string())?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|_| "Steam returned an unrecognised access token".to_string())?;
    claims
        .get("sub")
        .and_then(serde_json::Value::as_str)
        .and_then(|subject| subject.parse().ok())
        .ok_or_else(|| "Steam access token did not identify an account".to_string())
}

fn qr_data_url(challenge_url: &str) -> Result<String, String> {
    let code = QrCode::new(challenge_url.as_bytes()).map_err(|error| error.to_string())?;
    let image = code
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#17212b"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        STANDARD.encode(image)
    ))
}

async fn set_view(state: &SpikeState, update: impl FnOnce(&mut AuthView)) {
    let mut view = state.view.write().await;
    update(&mut view);
}

fn sanitise_error(error: &str) -> String {
    let first_line = error.lines().next().unwrap_or("Unknown error");
    if first_line.len() > 240 {
        format!("{}…", &first_line[..240])
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_renderer_does_not_expose_challenge_in_data_url() {
        let challenge = "https://s.team/q/1/12345";
        let rendered = qr_data_url(challenge).expect("QR should render");
        assert!(rendered.starts_with("data:image/svg+xml;base64,"));
        assert!(!rendered.contains(challenge));
    }
}
