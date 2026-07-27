use steamroom::types::{KeyValue, KvValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDepotMetadata {
    pub app_id: u32,
    pub depots: Vec<DepotMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppIdentity {
    pub app_id: u32,
    pub name: String,
    pub app_type: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepotConfig {
    pub os_list: Vec<String>,
    pub os_arch: Vec<String>,
    pub languages: Vec<String>,
    pub optional: bool,
    pub low_violence: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepotMetadata {
    pub depot_id: u32,
    pub name: Option<String>,
    pub config: DepotConfig,
    pub public_manifest: Option<ManifestReference>,
    pub dlc_app_id: Option<u32>,
    pub depot_from_app: Option<u32>,
    pub shared_install: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestReference {
    pub manifest_id: u64,
    pub uncompressed_bytes: Option<u64>,
    pub compressed_bytes: Option<u64>,
}

pub fn parse_app_depot_metadata(app_id: u32, buffer: &[u8]) -> Result<AppDepotMetadata, String> {
    let root = parse_product_root(buffer)?;
    parse_app_depot_tree(app_id, &root)
}

pub fn parse_app_identity(app_id: u32, buffer: &[u8]) -> Result<AppIdentity, String> {
    let root = parse_product_root(buffer)?;
    let app_info = if root.key.eq_ignore_ascii_case("appinfo") {
        &root
    } else {
        root.get("appinfo").unwrap_or(&root)
    };
    let common = app_info
        .get("common")
        .ok_or_else(|| "PICS metadata has no common section".to_string())?;
    Ok(AppIdentity {
        app_id,
        name: string(common.get("name"))
            .unwrap_or("Unknown game")
            .to_string(),
        app_type: string(common.get("type"))
            .unwrap_or("unknown")
            .to_ascii_lowercase(),
    })
}

fn parse_product_root(buffer: &[u8]) -> Result<KeyValue, String> {
    match KeyValue::from_binary(buffer) {
        Ok(root) => Ok(root),
        Err(binary_error) => {
            let text = std::str::from_utf8(buffer).map_err(|_| {
                format!("Could not parse binary PICS product metadata: {binary_error}")
            })?;
            KeyValue::from_text(text)
                .map_err(|error| format!("Could not parse PICS product metadata: {error}"))
        }
    }
}

fn parse_app_depot_tree(app_id: u32, root: &KeyValue) -> Result<AppDepotMetadata, String> {
    let app_info = if root.key.eq_ignore_ascii_case("appinfo") {
        root
    } else {
        root.get("appinfo").unwrap_or(root)
    };
    let depots = app_info
        .get("depots")
        .ok_or_else(|| "PICS metadata has no depot section".to_string())?;
    let KvValue::Children(entries) = &depots.value else {
        return Err("PICS depot section is not a map".to_string());
    };

    let mut parsed = entries
        .iter()
        .filter_map(|(key, value)| {
            let depot_id = key.parse::<u32>().ok()?;
            Some(parse_depot(depot_id, value))
        })
        .collect::<Vec<_>>();
    parsed.sort_by_key(|depot| depot.depot_id);
    Ok(AppDepotMetadata {
        app_id,
        depots: parsed,
    })
}

fn parse_depot(depot_id: u32, node: &KeyValue) -> DepotMetadata {
    let config = node.get("config");
    DepotMetadata {
        depot_id,
        name: string(node.get("name")).map(ToOwned::to_owned),
        config: DepotConfig {
            os_list: csv(config.and_then(|value| value.get("oslist"))),
            os_arch: csv(config.and_then(|value| value.get("osarch"))),
            languages: csv(
                config.and_then(|value| value.get("language").or_else(|| value.get("languages")))
            ),
            optional: truthy(
                config
                    .and_then(|value| value.get("optional"))
                    .or_else(|| node.get("optional")),
            ),
            low_violence: truthy(
                config
                    .and_then(|value| value.get("lowviolence"))
                    .or_else(|| node.get("lowviolence")),
            ),
        },
        public_manifest: node
            .get("manifests")
            .and_then(|manifests| manifests.get("public"))
            .and_then(parse_manifest_reference),
        dlc_app_id: integer(
            node.get("dlcappid")
                .or_else(|| config.and_then(|value| value.get("dlcappid"))),
        )
        .and_then(|value| u32::try_from(value).ok()),
        depot_from_app: integer(
            node.get("depotfromapp")
                .or_else(|| config.and_then(|value| value.get("depotfromapp"))),
        )
        .and_then(|value| u32::try_from(value).ok()),
        shared_install: truthy(
            node.get("sharedinstall")
                .or_else(|| config.and_then(|value| value.get("sharedinstall"))),
        ),
    }
}

fn parse_manifest_reference(node: &KeyValue) -> Option<ManifestReference> {
    let manifest_id = integer(Some(node)).or_else(|| integer(node.get("gid")))?;
    Some(ManifestReference {
        manifest_id,
        uncompressed_bytes: integer(node.get("size")),
        compressed_bytes: integer(node.get("download")),
    })
}

fn string(value: Option<&KeyValue>) -> Option<&str> {
    value.and_then(KeyValue::as_str)
}

fn integer(value: Option<&KeyValue>) -> Option<u64> {
    value.and_then(|value| match &value.value {
        KvValue::String(text) => text.parse().ok(),
        KvValue::UInt64(number) => Some(*number),
        KvValue::Int64(number) => u64::try_from(*number).ok(),
        KvValue::Int32(number) => u64::try_from(*number).ok(),
        _ => None,
    })
}

fn truthy(value: Option<&KeyValue>) -> bool {
    matches!(string(value), Some("1" | "true" | "yes"))
        || integer(value).is_some_and(|number| number != 0)
}

fn csv(value: Option<&KeyValue>) -> Vec<String> {
    string(value)
        .into_iter()
        .flat_map(|text| text.split([',', ';']))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCT_INFO: &str = r#"
"appinfo"
{
    "depots"
    {
        "branches" { "public" { "buildid" "99" } }
        "100"
        {
            "name" "shared"
            "manifests" { "public" { "gid" "9001" "size" "1000" "download" "700" } }
        }
        "101"
        {
            "config" { "oslist" "windows" "osarch" "64" }
            "manifests" { "public" "9002" }
        }
        "102"
        {
            "config" { "oslist" "macos" "language" "english,german" }
            "dlcappid" "500"
            "sharedinstall" "1"
            "depotfromapp" "400"
            "manifests" { "public" { "gid" "9003" } }
        }
    }
}
"#;

    #[test]
    fn parses_public_manifest_and_filters_non_depot_children() {
        let root = KeyValue::from_text(PRODUCT_INFO).expect("fixture should parse");
        let metadata = parse_app_depot_tree(42, &root).expect("depots should parse");
        assert_eq!(metadata.app_id, 42);
        assert_eq!(metadata.depots.len(), 3);
        assert_eq!(
            metadata.depots[0].public_manifest,
            Some(ManifestReference {
                manifest_id: 9001,
                uncompressed_bytes: Some(1000),
                compressed_bytes: Some(700),
            })
        );
        assert_eq!(metadata.depots[1].config.os_list, vec!["windows"]);
        assert_eq!(metadata.depots[1].config.os_arch, vec!["64"]);
        assert_eq!(
            metadata.depots[2].config.languages,
            vec!["english", "german"]
        );
        assert_eq!(metadata.depots[2].dlc_app_id, Some(500));
        assert_eq!(metadata.depots[2].depot_from_app, Some(400));
        assert!(metadata.depots[2].shared_install);
    }

    #[test]
    fn parses_app_identity_from_product_metadata() {
        let fixture = br#"
"appinfo"
{
    "common" { "name" "Fixture Game" "type" "Game" }
}
"#;
        assert_eq!(
            parse_app_identity(42, fixture).expect("identity should parse"),
            AppIdentity {
                app_id: 42,
                name: "Fixture Game".to_string(),
                app_type: "game".to_string(),
            }
        );
    }
}
