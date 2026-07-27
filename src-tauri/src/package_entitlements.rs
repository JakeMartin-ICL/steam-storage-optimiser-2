use crate::steam_library::{AccountPackages, PackageLicenseSources};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;
use steamroom::client::{LoggedIn, SteamClient};
use steamroom::types::{KeyValue, KvValue};

const PACKAGE_BATCH_SIZE: usize = 50;

#[derive(Clone, Debug, Default)]
pub struct PackageEntitlements {
    pub app_ids: HashSet<u32>,
    pub depot_ids: HashSet<u32>,
    pub app_packages: BTreeMap<u32, BTreeSet<u32>>,
    pub depot_packages: BTreeMap<u32, BTreeSet<u32>>,
    pub package_notes: BTreeMap<u32, BTreeSet<String>>,
    pub package_sources: BTreeMap<u32, PackageLicenseSources>,
    package_app_ids: BTreeMap<u32, HashSet<u32>>,
    package_depot_ids: BTreeMap<u32, HashSet<u32>>,
}

#[derive(Clone, Debug, Default)]
pub struct GameEntitlements {
    pub app_ids: HashSet<u32>,
    pub depot_ids: HashSet<u32>,
    pub package_ids: BTreeSet<u32>,
    pub shared_only: bool,
    pub borrowed_owner: Option<u32>,
    pub ambiguous_borrowed_owners: BTreeSet<u32>,
}

pub async fn resolve_package_entitlements(
    client: &SteamClient<LoggedIn>,
    packages: &AccountPackages,
) -> Result<PackageEntitlements, String> {
    let mut entitlements = PackageEntitlements {
        package_notes: packages.notes.clone(),
        package_sources: packages.sources.clone(),
        ..Default::default()
    };
    for package_batch in packages.ids.chunks(PACKAGE_BATCH_SIZE) {
        let infos = tokio::time::timeout(
            Duration::from_secs(20),
            client.pics_get_package_info(package_batch),
        )
        .await
        .map_err(|_| "Steam package metadata timed out".to_string())?
        .map_err(|error| error.to_string())?;
        for info in infos {
            if let Some(buffer) = info.kv_data {
                let mut parsed = PackageEntitlements::default();
                parse_package_entitlements(&buffer, &mut parsed)?;
                let package_id = info.package_id.map(|package| package.0).unwrap_or(0);
                entitlements
                    .package_app_ids
                    .insert(package_id, parsed.app_ids.clone());
                entitlements
                    .package_depot_ids
                    .insert(package_id, parsed.depot_ids.clone());
                for app_id in parsed.app_ids {
                    entitlements.app_ids.insert(app_id);
                    entitlements
                        .app_packages
                        .entry(app_id)
                        .or_default()
                        .insert(package_id);
                }
                for depot_id in parsed.depot_ids {
                    entitlements.depot_ids.insert(depot_id);
                    entitlements
                        .depot_packages
                        .entry(depot_id)
                        .or_default()
                        .insert(package_id);
                }
            }
        }
    }
    Ok(entitlements)
}

impl PackageEntitlements {
    pub fn cache_fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        let mut app_ids = self.app_ids.iter().copied().collect::<Vec<_>>();
        app_ids.sort_unstable();
        app_ids.hash(&mut hasher);
        let mut depot_ids = self.depot_ids.iter().copied().collect::<Vec<_>>();
        depot_ids.sort_unstable();
        depot_ids.hash(&mut hasher);
        for (package_id, source) in &self.package_sources {
            package_id.hash(&mut hasher);
            source.direct.hash(&mut hasher);
            source.borrowed_owners.hash(&mut hasher);
            source.preferred_borrowed_owners.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn shared_only_app_ids(&self) -> HashSet<u32> {
        self.app_ids
            .iter()
            .copied()
            .filter(|app_id| {
                let Some(packages) = self.app_packages.get(app_id) else {
                    return false;
                };
                !packages.iter().any(|package_id| {
                    self.package_sources
                        .get(package_id)
                        .is_some_and(|source| source.direct)
                }) && packages.iter().any(|package_id| {
                    self.package_sources
                        .get(package_id)
                        .is_some_and(|source| !source.borrowed_owners.is_empty())
                })
            })
            .collect()
    }

    pub fn for_game(&self, app_id: u32) -> GameEntitlements {
        let base_packages = self.app_packages.get(&app_id).cloned().unwrap_or_default();
        let directly_owned = base_packages.iter().any(|package_id| {
            self.package_sources
                .get(package_id)
                .is_some_and(|source| source.direct)
        });

        if directly_owned {
            return self.collect_matching_packages(
                |source| source.direct,
                false,
                None,
                BTreeSet::new(),
            );
        }

        let mut owners = BTreeSet::new();
        let mut preferred_owners = BTreeSet::new();
        for package_id in &base_packages {
            if let Some(source) = self.package_sources.get(package_id) {
                owners.extend(source.borrowed_owners.iter().copied());
                preferred_owners.extend(source.preferred_borrowed_owners.iter().copied());
            }
        }
        let selected_owner = if preferred_owners.len() == 1 {
            preferred_owners.first().copied()
        } else if owners.len() == 1 {
            owners.first().copied()
        } else {
            None
        };
        let ambiguous_owners = if selected_owner.is_none() {
            owners.clone()
        } else {
            BTreeSet::new()
        };

        self.collect_matching_packages(
            |source| selected_owner.is_some_and(|owner| source.borrowed_owners.contains(&owner)),
            !owners.is_empty(),
            selected_owner,
            ambiguous_owners,
        )
    }

    fn collect_matching_packages(
        &self,
        include: impl Fn(&PackageLicenseSources) -> bool,
        shared_only: bool,
        borrowed_owner: Option<u32>,
        ambiguous_borrowed_owners: BTreeSet<u32>,
    ) -> GameEntitlements {
        let package_ids = self
            .package_sources
            .iter()
            .filter_map(|(package_id, source)| include(source).then_some(*package_id))
            .collect::<BTreeSet<_>>();
        let mut selection = GameEntitlements {
            package_ids,
            shared_only,
            borrowed_owner,
            ambiguous_borrowed_owners,
            ..Default::default()
        };
        for package_id in &selection.package_ids {
            if let Some(app_ids) = self.package_app_ids.get(package_id) {
                selection.app_ids.extend(app_ids);
            }
            if let Some(depot_ids) = self.package_depot_ids.get(package_id) {
                selection.depot_ids.extend(depot_ids);
            }
        }
        selection
    }
}

fn parse_package_entitlements(
    buffer: &[u8],
    entitlements: &mut PackageEntitlements,
) -> Result<(), String> {
    let mut last_binary_error = None;
    for offset in [0, 4, 8, 12] {
        let Some(candidate) = buffer.get(offset..) else {
            continue;
        };
        for wrapped in [false, true] {
            let bytes = if wrapped {
                let mut bytes = Vec::with_capacity(candidate.len() + 7);
                bytes.extend_from_slice(&[0]);
                bytes.extend_from_slice(b"root\0");
                bytes.extend_from_slice(candidate);
                bytes
            } else {
                candidate.to_vec()
            };
            match KeyValue::from_binary(&bytes) {
                Ok(root) => {
                    let mut parsed = PackageEntitlements::default();
                    collect_named_ids(&root, "appids", &mut parsed.app_ids);
                    collect_named_ids(&root, "depotids", &mut parsed.depot_ids);
                    if !parsed.app_ids.is_empty() || !parsed.depot_ids.is_empty() {
                        entitlements.app_ids.extend(parsed.app_ids);
                        entitlements.depot_ids.extend(parsed.depot_ids);
                        return Ok(());
                    }
                }
                Err(error) => last_binary_error = Some(error.to_string()),
            }
        }
    }

    if let Ok(text) = std::str::from_utf8(buffer)
        && let Ok(root) = KeyValue::from_text(text.trim_end_matches('\0'))
    {
        collect_named_ids(&root, "appids", &mut entitlements.app_ids);
        collect_named_ids(&root, "depotids", &mut entitlements.depot_ids);
        if !entitlements.app_ids.is_empty() || !entitlements.depot_ids.is_empty() {
            return Ok(());
        }
    }

    let fingerprint = buffer
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("");
    Err(format!(
        "Could not decode PICS package metadata ({} bytes, prefix {fingerprint}, last parser error: {})",
        buffer.len(),
        last_binary_error.unwrap_or_else(|| "none".to_string())
    ))
}

fn collect_named_ids(node: &KeyValue, name: &str, output: &mut HashSet<u32>) {
    if node.key.eq_ignore_ascii_case(name) {
        collect_child_values(node, output);
        return;
    }
    if let KvValue::Children(children) = &node.value {
        for child in children.values() {
            collect_named_ids(child, name, output);
        }
    }
}

fn collect_child_values(node: &KeyValue, output: &mut HashSet<u32>) {
    let KvValue::Children(children) = &node.value else {
        return;
    };
    for child in children.values() {
        let value = match &child.value {
            KvValue::String(value) => value.parse().ok(),
            KvValue::Int32(value) => u32::try_from(*value).ok(),
            KvValue::Int64(value) => u32::try_from(*value).ok(),
            KvValue::UInt64(value) => u32::try_from(*value).ok(),
            _ => None,
        };
        if let Some(value) = value {
            output.insert(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam_library::PackageLicenseSources;

    #[test]
    fn parses_apps_and_depots_from_sanitized_package_metadata() {
        let fixture = br#"
"packageinfo"
{
    "123"
    {
        "appids" { "0" "8930" "1" "210450" }
        "depotids" { "0" "34481" "1" "210450" }
    }
}
"#;
        let mut entitlements = PackageEntitlements::default();
        parse_package_entitlements(fixture, &mut entitlements)
            .expect("package fixture should parse");
        assert_eq!(entitlements.app_ids, HashSet::from([8930, 210450]));
        assert_eq!(entitlements.depot_ids, HashSet::from([34481, 210450]));
    }

    #[test]
    fn chooses_direct_dlc_for_owned_games_and_one_lenders_dlc_for_shared_games() {
        let mut entitlements = PackageEntitlements::default();
        entitlements.app_packages.insert(10, BTreeSet::from([1, 2]));
        entitlements.app_packages.insert(11, BTreeSet::from([3]));
        entitlements.app_packages.insert(20, BTreeSet::from([4]));
        entitlements.app_packages.insert(21, BTreeSet::from([5]));
        entitlements.package_sources.insert(
            1,
            PackageLicenseSources {
                direct: true,
                ..Default::default()
            },
        );
        entitlements.package_sources.insert(
            2,
            PackageLicenseSources {
                borrowed_owners: BTreeSet::from([99]),
                ..Default::default()
            },
        );
        entitlements.package_sources.insert(
            3,
            PackageLicenseSources {
                direct: true,
                borrowed_owners: BTreeSet::from([99]),
                ..Default::default()
            },
        );
        entitlements.package_sources.insert(
            4,
            PackageLicenseSources {
                borrowed_owners: BTreeSet::from([99]),
                preferred_borrowed_owners: BTreeSet::from([99]),
                ..Default::default()
            },
        );
        entitlements.package_sources.insert(
            5,
            PackageLicenseSources {
                borrowed_owners: BTreeSet::from([99]),
                ..Default::default()
            },
        );
        entitlements.package_app_ids.insert(1, HashSet::from([10]));
        entitlements
            .package_app_ids
            .insert(2, HashSet::from([10, 12]));
        entitlements.package_app_ids.insert(3, HashSet::from([11]));
        entitlements.package_app_ids.insert(4, HashSet::from([20]));
        entitlements.package_app_ids.insert(5, HashSet::from([21]));

        let owned = entitlements.for_game(10);
        assert!(!owned.shared_only);
        assert_eq!(owned.app_ids, HashSet::from([10, 11]));
        assert_eq!(owned.package_ids, BTreeSet::from([1, 3]));

        let shared = entitlements.for_game(20);
        assert!(shared.shared_only);
        assert_eq!(shared.borrowed_owner, Some(99));
        assert_eq!(shared.app_ids, HashSet::from([10, 11, 12, 20, 21]));
        assert_eq!(shared.package_ids, BTreeSet::from([2, 3, 4, 5]));
    }

    #[test]
    fn prefers_steams_selected_family_owner_and_surfaces_unresolved_ambiguity() {
        let mut entitlements = PackageEntitlements::default();
        for (package_id, owner, preferred, app_ids) in [
            (1, 88, false, HashSet::from([30])),
            (2, 99, true, HashSet::from([30])),
            (3, 99, false, HashSet::from([31])),
            (4, 77, false, HashSet::from([40])),
            (5, 66, false, HashSet::from([40])),
        ] {
            entitlements.package_sources.insert(
                package_id,
                PackageLicenseSources {
                    borrowed_owners: BTreeSet::from([owner]),
                    preferred_borrowed_owners: if preferred {
                        BTreeSet::from([owner])
                    } else {
                        BTreeSet::new()
                    },
                    ..Default::default()
                },
            );
            for app_id in &app_ids {
                entitlements
                    .app_packages
                    .entry(*app_id)
                    .or_default()
                    .insert(package_id);
            }
            entitlements.package_app_ids.insert(package_id, app_ids);
        }

        let preferred = entitlements.for_game(30);
        assert_eq!(preferred.borrowed_owner, Some(99));
        assert_eq!(preferred.package_ids, BTreeSet::from([2, 3]));
        assert_eq!(preferred.app_ids, HashSet::from([30, 31]));

        let ambiguous = entitlements.for_game(40);
        assert_eq!(ambiguous.borrowed_owner, None);
        assert_eq!(
            ambiguous.ambiguous_borrowed_owners,
            BTreeSet::from([66, 77])
        );
        assert!(ambiguous.package_ids.is_empty());
        assert!(ambiguous.app_ids.is_empty());
    }
}
