mod app_identity_cache;
mod auth_spike;
mod community_contribution_cache;
pub mod community_sizes;
mod credential_store;
mod depot_cache;
mod depot_metadata;
mod depot_probe;
mod depot_selection;
mod hltb;
mod hltb_cache;
mod local_steam;
mod package_entitlements;
pub mod size_comparison;
mod steam_library;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(auth_spike::SpikeState::default())
        .invoke_handler(tauri::generate_handler![
            auth_spike::get_auth_state,
            auth_spike::has_saved_login,
            auth_spike::start_qr_login,
            auth_spike::cancel_qr_login,
            auth_spike::forget_saved_login,
            auth_spike::search_hltb,
            auth_spike::set_hltb_match,
            local_steam::get_storage_target_default,
            local_steam::get_steam_location,
            local_steam::set_steam_location
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
