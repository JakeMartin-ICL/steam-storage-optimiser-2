use crate::depot_metadata::{AppDepotMetadata, DepotMetadata, ManifestReference};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    Windows,
    MacOs,
    Linux,
}

impl TargetOs {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }

    pub fn steam_name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetArchitecture {
    X86,
    X86_64,
    Arm64,
}

impl TargetArchitecture {
    pub fn current() -> Self {
        match std::env::consts::ARCH {
            "aarch64" => Self::Arm64,
            "x86" => Self::X86,
            _ => Self::X86_64,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SelectionContext {
    pub os: TargetOs,
    pub architecture: TargetArchitecture,
    pub language: String,
    pub entitled_app_ids: HashSet<u32>,
    pub entitled_depot_ids: HashSet<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedDepot {
    pub depot_id: u32,
    pub source_app_id: u32,
    pub manifest: ManifestReference,
    pub dlc_app_id: Option<u32>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcludedDepot {
    pub depot_id: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionOutcome {
    pub selected: Vec<SelectedDepot>,
    pub excluded: Vec<ExcludedDepot>,
    pub warnings: Vec<String>,
}

pub fn select_depots(metadata: &AppDepotMetadata, context: &SelectionContext) -> SelectionOutcome {
    let mut outcome = SelectionOutcome::default();
    for depot in &metadata.depots {
        match select_depot(metadata.app_id, depot, context, &mut outcome.warnings) {
            Ok(selected) => outcome.selected.push(selected),
            Err(reason) => outcome.excluded.push(ExcludedDepot {
                depot_id: depot.depot_id,
                reason,
            }),
        }
    }
    outcome
}

fn select_depot(
    app_id: u32,
    depot: &DepotMetadata,
    context: &SelectionContext,
    warnings: &mut Vec<String>,
) -> Result<SelectedDepot, String> {
    if let Some(content_role) = excluded_content_role(depot.name.as_deref()) {
        return Err(format!("identified as {content_role} content"));
    }

    let manifest = depot
        .public_manifest
        .clone()
        .ok_or_else(|| "no public-branch manifest".to_string())?;

    if !depot.config.os_list.is_empty()
        && !depot
            .config
            .os_list
            .iter()
            .any(|os| os == context.os.steam_name())
    {
        return Err(format!("not for {}", context.os.steam_name()));
    }

    if !architecture_matches(&depot.config.os_arch, context.architecture) {
        return Err("architecture filter does not match".to_string());
    }

    if !depot.config.languages.is_empty()
        && !depot
            .config
            .languages
            .iter()
            .any(|language| language == &context.language || language == "all")
    {
        return Err(format!("not for language {}", context.language));
    }

    if let Some(dlc_app_id) = depot.dlc_app_id {
        if !context.entitled_app_ids.contains(&dlc_app_id)
            && !context.entitled_depot_ids.contains(&depot.depot_id)
        {
            return Err(format!("DLC {dlc_app_id} is not in an owned package"));
        }
    } else if depot.config.optional && !context.entitled_depot_ids.contains(&depot.depot_id) {
        return Err("marked optional and not in an owned package".to_string());
    }

    let mut reasons = vec!["public branch".to_string()];
    reasons.push(if depot.config.os_list.is_empty() {
        "shared across operating systems".to_string()
    } else {
        format!("matches {}", context.os.steam_name())
    });
    if depot.config.languages.is_empty() {
        reasons.push("language-neutral".to_string());
    } else {
        reasons.push(format!("matches language {}", context.language));
    }
    if let Some(dlc_app_id) = depot.dlc_app_id {
        reasons.push(format!("DLC {dlc_app_id} is in an owned package"));
    } else if depot.config.optional {
        reasons.push("optional depot is in an owned package".to_string());
    }
    if depot.shared_install {
        reasons.push("Steam marks this as a shared install".to_string());
    }
    let source_app_id = depot.depot_from_app.unwrap_or(app_id);
    if source_app_id != app_id {
        warnings.push(format!(
            "Depot {} is sourced from app {}; ownership/key behavior requires validation",
            depot.depot_id, source_app_id
        ));
    }
    if depot.config.low_violence {
        warnings.push(format!(
            "Depot {} is low-violence content; country selection is not implemented",
            depot.depot_id
        ));
    }
    if context.architecture == TargetArchitecture::Arm64
        && depot.config.os_arch.iter().any(|arch| arch == "64")
    {
        warnings.push(format!(
            "Depot {} uses Steam's generic 64-bit filter on ARM64",
            depot.depot_id
        ));
    }

    Ok(SelectedDepot {
        depot_id: depot.depot_id,
        source_app_id,
        manifest,
        dlc_app_id: depot.dlc_app_id,
        reasons,
    })
}

fn excluded_content_role(name: Option<&str>) -> Option<&'static str> {
    let name = name?.to_ascii_lowercase();
    [
        ("soundtrack", "soundtrack"),
        ("dedicated server", "dedicated-server"),
        ("redistributable", "redistributable"),
        ("workshop", "workshop"),
    ]
    .into_iter()
    .find_map(|(needle, role)| name.contains(needle).then_some(role))
}

fn architecture_matches(filters: &[String], target: TargetArchitecture) -> bool {
    if filters.is_empty() {
        return true;
    }
    filters.iter().any(|filter| match target {
        TargetArchitecture::X86 => matches!(filter.as_str(), "32" | "x86"),
        TargetArchitecture::X86_64 => matches!(filter.as_str(), "64" | "x64" | "x86_64"),
        TargetArchitecture::Arm64 => {
            matches!(
                filter.as_str(),
                "64" | "arm64" | "aarch64" | "x64" | "x86_64"
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depot_metadata::{DepotConfig, DepotMetadata};

    fn depot(id: u32) -> DepotMetadata {
        DepotMetadata {
            depot_id: id,
            name: None,
            config: DepotConfig::default(),
            public_manifest: Some(ManifestReference {
                manifest_id: u64::from(id),
                uncompressed_bytes: None,
                compressed_bytes: None,
            }),
            dlc_app_id: None,
            depot_from_app: None,
            shared_install: false,
        }
    }

    fn context() -> SelectionContext {
        SelectionContext {
            os: TargetOs::MacOs,
            architecture: TargetArchitecture::Arm64,
            language: "english".to_string(),
            entitled_app_ids: HashSet::from([500]),
            entitled_depot_ids: HashSet::new(),
        }
    }

    #[test]
    fn selects_shared_and_matching_platform_depots() {
        let shared = depot(1);
        let mut mac = depot(2);
        mac.config.os_list = vec!["macos".to_string()];
        mac.config.os_arch = vec!["64".to_string()];
        let mut windows = depot(3);
        windows.config.os_list = vec!["windows".to_string()];
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![shared, mac, windows],
        };

        let outcome = select_depots(&metadata, &context());
        assert_eq!(
            outcome
                .selected
                .iter()
                .map(|depot| depot.depot_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(outcome.excluded[0].depot_id, 3);
        assert_eq!(outcome.warnings.len(), 1);
    }

    #[test]
    fn includes_only_dlc_present_in_owned_packages() {
        let mut first = depot(1);
        first.dlc_app_id = Some(500);
        let mut second = depot(2);
        second.dlc_app_id = Some(600);
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![first, second],
        };

        let outcome = select_depots(&metadata, &context());
        assert_eq!(outcome.selected.len(), 1);
        assert_eq!(outcome.selected[0].dlc_app_id, Some(500));
        assert!(outcome.selected[0].reasons[3].contains("owned package"));
        assert!(
            outcome.excluded[0]
                .reason
                .contains("not in an owned package")
        );
    }

    #[test]
    fn excludes_language_mismatches_and_optional_content() {
        let mut language = depot(1);
        language.config.languages = vec!["german".to_string()];
        let mut optional = depot(2);
        optional.config.optional = true;
        let metadata = AppDepotMetadata {
            app_id: 42,
            depots: vec![language, optional],
        };

        let outcome = select_depots(&metadata, &context());
        assert!(outcome.selected.is_empty());
        assert_eq!(outcome.excluded.len(), 2);
    }

    #[test]
    fn excludes_soundtrack_depots_even_when_steam_does_not_mark_them_optional() {
        let mut soundtrack = depot(726050);
        soundtrack.name = Some("Converted Soundtrack Depot".to_string());
        let metadata = AppDepotMetadata {
            app_id: 674940,
            depots: vec![soundtrack],
        };

        let outcome = select_depots(&metadata, &context());
        assert!(outcome.selected.is_empty());
        assert_eq!(
            outcome.excluded[0].reason,
            "identified as soundtrack content"
        );
    }
}
