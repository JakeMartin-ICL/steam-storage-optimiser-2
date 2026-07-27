use serde::{Deserialize, Serialize};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SavedLogin {
    pub account_name: String,
    pub refresh_token: String,
}

pub async fn has_saved_login() -> bool {
    session_path().is_ok_and(|path| path.is_file())
}

pub async fn load_saved_login() -> Result<Option<SavedLogin>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let path = session_path()?;
        match std::fs::read_to_string(path) {
            Ok(value) => serde_json::from_str(&value)
                .map(Some)
                .map_err(|_| "The development Steam session is unreadable".to_string()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "Could not read the development Steam session: {error}"
            )),
        }
    })
    .await
    .map_err(|error| format!("Session-file task failed: {error}"))?
}

pub async fn save_login(login: SavedLogin) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = session_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| "Development session path has no parent directory".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the session directory: {error}"))?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Could not protect the session directory: {error}"))?;

        let encoded = serde_json::to_vec(&login)
            .map_err(|_| "Could not encode the development Steam session".to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| format!("Could not open the session file: {error}"))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not protect the session file: {error}"))?;
        file.write_all(&encoded)
            .map_err(|error| format!("Could not save the development Steam session: {error}"))
    })
    .await
    .map_err(|error| format!("Session-file task failed: {error}"))?
}

pub async fn delete_saved_login() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| match std::fs::remove_file(session_path()?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Could not remove the development Steam session: {error}"
        )),
    })
    .await
    .map_err(|error| format!("Session-file task failed: {error}"))?
}

fn session_path() -> Result<PathBuf, String> {
    dirs_next::data_local_dir()
        .map(|directory| {
            directory
                .join("Steam Storage Optimiser")
                .join("dev-session.json")
        })
        .ok_or_else(|| "Could not locate the local application-data directory".to_string())
}
