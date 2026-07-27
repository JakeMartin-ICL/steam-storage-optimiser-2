use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use steamroom::types::{KeyValue, KvValue};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDepot {
    pub depot_id: u32,
    pub manifest_id: u64,
    pub recorded_size_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub app_id: u32,
    pub name: String,
    pub size_on_disk_bytes: u64,
    pub language: Option<String>,
    pub depots: Vec<InstalledDepot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageTargetDefault {
    pub target_bytes: u64,
    pub filesystem_size_bytes: u64,
}

const GIB: u64 = 1024 * 1024 * 1024;
const TARGET_STEP_BYTES: u64 = 10 * GIB;
const MINIMUM_TARGET_BYTES: u64 = 100 * GIB;

#[tauri::command]
pub fn get_storage_target_default() -> Result<StorageTargetDefault, String> {
    let steam_root = default_steam_root()
        .ok_or_else(|| "Could not locate the Steam installation".to_string())?;
    let filesystem_size_bytes = fs4::total_space(&steam_root)
        .map_err(|error| format!("Could not read Steam filesystem capacity: {error}"))?;
    Ok(StorageTargetDefault {
        target_bytes: recommended_storage_target(filesystem_size_bytes),
        filesystem_size_bytes,
    })
}

fn recommended_storage_target(filesystem_size_bytes: u64) -> u64 {
    let half_capacity = filesystem_size_bytes / 2;
    let rounded =
        half_capacity.saturating_add(TARGET_STEP_BYTES / 2) / TARGET_STEP_BYTES * TARGET_STEP_BYTES;
    rounded
        .max(MINIMUM_TARGET_BYTES.min(filesystem_size_bytes))
        .min(filesystem_size_bytes)
}

pub fn discover_installed_apps() -> Result<Vec<InstalledApp>, String> {
    let steam_root = default_steam_root()
        .ok_or_else(|| "Could not locate the Steam installation".to_string())?;
    discover_installed_apps_at(&steam_root)
}

pub fn discover_local_playtimes(account_id: u32) -> Result<BTreeMap<u32, u32>, String> {
    let steam_root = default_steam_root()
        .ok_or_else(|| "Could not locate the Steam installation".to_string())?;
    let path = steam_root
        .join("userdata")
        .join(account_id.to_string())
        .join("config/localconfig.vdf");
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Could not read local Steam playtime data: {error}"))?;
    parse_local_playtimes(&text)
}

fn parse_local_playtimes(text: &str) -> Result<BTreeMap<u32, u32>, String> {
    let root = KeyValue::from_text(text).map_err(|error| error.to_string())?;
    let apps = node_named(&root, "UserLocalConfigStore")
        .get("Software")
        .and_then(|node| node.get("Valve"))
        .and_then(|node| node.get("Steam"))
        .and_then(|node| node.get("apps"))
        .ok_or_else(|| "Local Steam config has no app playtime section".to_string())?;
    let KvValue::Children(entries) = &apps.value else {
        return Err("Local Steam app playtime section is not a map".to_string());
    };
    Ok(entries
        .iter()
        .filter_map(|(app_id, app)| {
            Some((
                app_id.parse().ok()?,
                u32::try_from(integer_value(app.get("Playtime"))?).ok()?,
            ))
        })
        .collect())
}

fn discover_installed_apps_at(steam_root: &Path) -> Result<Vec<InstalledApp>, String> {
    let primary_steamapps = steam_root.join("steamapps");
    let mut libraries = BTreeSet::from([primary_steamapps.clone()]);
    let library_file = primary_steamapps.join("libraryfolders.vdf");

    if let Ok(text) = fs::read_to_string(library_file)
        && let Ok(root) = KeyValue::from_text(&text)
    {
        let folders = node_named(&root, "libraryfolders");
        if let KvValue::Children(entries) = &folders.value {
            for entry in entries.values() {
                if let Some(path) = string_value(entry.get("path")) {
                    libraries.insert(PathBuf::from(path).join("steamapps"));
                }
            }
        }
    }

    let mut apps = Vec::new();
    for library in libraries {
        let Ok(entries) = fs::read_dir(&library) else {
            continue;
        };
        for entry in entries.flatten() {
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            if filename.starts_with("appmanifest_") && filename.ends_with(".acf") {
                let Ok(text) = fs::read_to_string(entry.path()) else {
                    continue;
                };
                if let Ok(app) = parse_app_manifest(&text) {
                    apps.push(app);
                }
            }
        }
    }
    apps.sort_by_key(|app| app.app_id);
    Ok(apps)
}

fn parse_app_manifest(text: &str) -> Result<InstalledApp, String> {
    let root = KeyValue::from_text(text).map_err(|error| error.to_string())?;
    let app = node_named(&root, "AppState");
    let app_id = integer_value(app.get("appid"))
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "Manifest is missing appid".to_string())?;
    let name = string_value(app.get("name"))
        .ok_or_else(|| "Manifest is missing name".to_string())?
        .to_string();
    let size_on_disk_bytes = integer_value(app.get("SizeOnDisk"))
        .ok_or_else(|| "Manifest is missing SizeOnDisk".to_string())?;
    let language = app
        .get("UserConfig")
        .and_then(|config| config.get("language"))
        .and_then(|value| string_value(Some(value)))
        .map(ToOwned::to_owned);

    let mut depots = Vec::new();
    if let Some(installed) = app.get("InstalledDepots")
        && let KvValue::Children(entries) = &installed.value
    {
        for (id, depot) in entries {
            let Some(depot_id) = id.parse::<u32>().ok() else {
                continue;
            };
            let Some(manifest_id) = integer_value(depot.get("manifest")) else {
                continue;
            };
            depots.push(InstalledDepot {
                depot_id,
                manifest_id,
                recorded_size_bytes: integer_value(depot.get("size")),
            });
        }
    }
    depots.sort_by_key(|depot| depot.depot_id);

    Ok(InstalledApp {
        app_id,
        name,
        size_on_disk_bytes,
        language,
        depots,
    })
}

fn node_named<'a>(root: &'a KeyValue, name: &str) -> &'a KeyValue {
    if root.key.eq_ignore_ascii_case(name) {
        root
    } else {
        root.get(name).unwrap_or(root)
    }
}

fn string_value(value: Option<&KeyValue>) -> Option<&str> {
    value.and_then(KeyValue::as_str)
}

fn integer_value(value: Option<&KeyValue>) -> Option<u64> {
    value.and_then(|value| match &value.value {
        KvValue::String(text) => text.parse().ok(),
        KvValue::UInt64(number) => Some(*number),
        KvValue::Int64(number) => u64::try_from(*number).ok(),
        KvValue::Int32(number) => u64::try_from(*number).ok(),
        _ => None,
    })
}

#[cfg(target_os = "macos")]
fn default_steam_root() -> Option<PathBuf> {
    dirs_next::home_dir().map(|home| home.join("Library/Application Support/Steam"))
}

#[cfg(target_os = "windows")]
fn default_steam_root() -> Option<PathBuf> {
    std::env::var_os("PROGRAMFILES(X86)")
        .or_else(|| std::env::var_os("PROGRAMFILES"))
        .map(PathBuf::from)
        .map(|path| path.join("Steam"))
}

#[cfg(target_os = "linux")]
fn default_steam_root() -> Option<PathBuf> {
    let home = dirs_next::home_dir()?;
    [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
"AppState"
{
    "appid" "42"
    "name" "Fixture Game"
    "SizeOnDisk" "1024"
    "InstalledDepots"
    {
        "100"
        {
            "manifest" "9001"
            "size" "768"
        }
    }
    "UserConfig"
    {
        "language" "english"
    }
}
"#;

    #[test]
    fn parses_only_non_secret_install_metadata() {
        let app = parse_app_manifest(MANIFEST).expect("fixture should parse");
        assert_eq!(app.app_id, 42);
        assert_eq!(app.name, "Fixture Game");
        assert_eq!(app.size_on_disk_bytes, 1024);
        assert_eq!(app.language.as_deref(), Some("english"));
        assert_eq!(
            app.depots,
            vec![InstalledDepot {
                depot_id: 100,
                manifest_id: 9001,
                recorded_size_bytes: Some(768)
            }]
        );
    }

    #[test]
    fn recommends_half_the_primary_steam_filesystem_in_ten_gib_steps() {
        assert_eq!(recommended_storage_target(500 * GIB), 250 * GIB);
        assert_eq!(recommended_storage_target(505 * GIB), 250 * GIB);
        assert_eq!(recommended_storage_target(515 * GIB), 260 * GIB);
        assert_eq!(recommended_storage_target(150 * GIB), 100 * GIB);
    }

    #[test]
    fn parses_local_playtimes_without_other_account_config() {
        let fixture = r#"
"UserLocalConfigStore"
{
    "Software" { "Valve" { "Steam" { "apps"
    {
        "42" { "LastPlayed" "100" "Playtime" "321" }
        "43" { "cloud" { "used_bytes" "12" } }
    } } } }
}
"#;
        assert_eq!(
            parse_local_playtimes(fixture).expect("playtimes should parse"),
            BTreeMap::from([(42, 321)])
        );
    }
}
