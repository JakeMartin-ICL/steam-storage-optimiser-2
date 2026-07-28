use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::time::{Duration, SystemTime};

const BASE_URL: &str = "https://howlongtobeat.com";
const TOKEN_URL: &str = "https://howlongtobeat.com/api/bleed/init";
const SEARCH_URL: &str = "https://howlongtobeat.com/api/bleed";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HltbEstimate {
    pub game_id: i64,
    pub game_name: String,
    pub main_seconds: Option<u32>,
    pub main_extra_seconds: Option<u32>,
    pub completionist_seconds: Option<u32>,
    pub steam_app_id: Option<u32>,
    pub match_method: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HltbCandidate {
    pub game_id: i64,
    pub game_name: String,
    pub main_seconds: Option<u32>,
    pub main_extra_seconds: Option<u32>,
    pub completionist_seconds: Option<u32>,
    pub steam_app_id: Option<u32>,
    pub platforms: String,
    pub similarity: f64,
}

#[derive(Debug)]
pub enum HltbError {
    RateLimited {
        retry_after: Option<Duration>,
        message: String,
    },
    Other(String),
}

impl std::fmt::Display for HltbError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited { message, .. } | Self::Other(message) => {
                formatter.write_str(message)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiData {
    token: String,
    hp_key: String,
    hp_val: String,
}

#[derive(Deserialize)]
struct SearchResponse {
    data: Vec<SearchResult>,
}

#[derive(Clone, Deserialize)]
struct SearchResult {
    game_id: i64,
    game_name: String,
    comp_main: i64,
    comp_plus: i64,
    comp_100: i64,
    #[serde(default)]
    profile_steam: i64,
    #[serde(default)]
    profile_steam_alt: i64,
    #[serde(default)]
    profile_platform: String,
}

pub struct HltbClient {
    client: reqwest::Client,
    api: ApiData,
}

impl HltbClient {
    pub async fn new() -> Result<Self, HltbError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| HltbError::Other(error.to_string()))?;
        let response = client
            .get(format!("{TOKEN_URL}?t={}", unix_millis(SystemTime::now())))
            .headers(default_headers()?)
            .send()
            .await
            .map_err(|error| HltbError::Other(error.to_string()))?;
        let response = checked_response(response).await?;
        let api = response.json().await.map_err(|error| {
            HltbError::Other(format!("Could not decode HLTB credentials: {error}"))
        })?;
        Ok(Self { client, api })
    }

    pub async fn search(
        &self,
        query: &str,
        steam_name: &str,
    ) -> Result<Vec<HltbCandidate>, HltbError> {
        let search_terms = search_terms(query);
        if search_terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut payload = serde_json::json!({
            "searchType": "games",
            "searchTerms": search_terms,
            "searchPage": 1,
            "size": 20,
            "searchOptions": {
                "games": {
                    "userId": 0,
                    "platform": "",
                    "sortCategory": "popular",
                    "rangeCategory": "main",
                    "rangeTime": { "min": null, "max": null },
                    "gameplay": {
                        "perspective": "",
                        "flow": "",
                        "genre": "",
                        "difficulty": ""
                    },
                    "rangeYear": { "min": "", "max": "" },
                    "modifier": "hide_dlc"
                },
                "users": { "sortCategory": "postcount" },
                "lists": { "sortCategory": "follows" },
                "filter": "",
                "sort": 0,
                "randomizer": 0
            },
            "useCache": true
        });
        payload[&self.api.hp_key] = serde_json::Value::String(self.api.hp_val.clone());
        let response = self
            .client
            .post(SEARCH_URL)
            .headers(default_headers()?)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "*/*")
            .header("x-auth-token", &self.api.token)
            .header("x-hp-key", &self.api.hp_key)
            .header("x-hp-val", &self.api.hp_val)
            .json(&payload)
            .send()
            .await
            .map_err(|error| HltbError::Other(error.to_string()))?;
        let response = checked_response(response).await?;
        let response: SearchResponse = response
            .json()
            .await
            .map_err(|error| HltbError::Other(format!("Could not decode HLTB search: {error}")))?;
        Ok(response
            .data
            .into_iter()
            .map(|result| result.into_candidate(steam_name))
            .collect())
    }
}

impl SearchResult {
    fn into_candidate(self, steam_name: &str) -> HltbCandidate {
        let similarity = title_similarity(steam_name, &self.game_name);
        HltbCandidate {
            game_id: self.game_id,
            game_name: self.game_name,
            main_seconds: positive_seconds(self.comp_main),
            main_extra_seconds: positive_seconds(self.comp_plus),
            completionist_seconds: positive_seconds(self.comp_100),
            steam_app_id: positive_app_id(self.profile_steam)
                .or_else(|| positive_app_id(self.profile_steam_alt)),
            platforms: self.profile_platform,
            similarity,
        }
    }
}

pub fn select_match(
    candidates: &[HltbCandidate],
    app_id: u32,
    preferred_game_id: Option<i64>,
) -> Option<HltbEstimate> {
    if let Some(preferred) = preferred_game_id
        && let Some(candidate) = candidates.iter().find(|game| game.game_id == preferred)
    {
        return Some(candidate.to_estimate("manual"));
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|game| game.steam_app_id == Some(app_id))
    {
        return Some(candidate.to_estimate("steam_app_id"));
    }
    let mut eligible = candidates
        .iter()
        .filter(|game| {
            game.similarity >= 0.92
                && (game.platforms.is_empty() || game.platforms.split(", ").any(|p| p == "PC"))
        })
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(Ordering::Equal)
    });
    let best = *eligible.first()?;
    let runner_up = eligible.get(1).map(|game| game.similarity).unwrap_or(0.0);
    (best.similarity - runner_up >= 0.05 || best.similarity >= 0.99)
        .then(|| best.to_estimate("title"))
}

impl HltbCandidate {
    pub fn to_estimate(&self, match_method: &str) -> HltbEstimate {
        HltbEstimate {
            game_id: self.game_id,
            game_name: self.game_name.clone(),
            main_seconds: self.main_seconds,
            main_extra_seconds: self.main_extra_seconds,
            completionist_seconds: self.completionist_seconds,
            steam_app_id: self.steam_app_id,
            match_method: match_method.to_string(),
        }
    }
}

async fn checked_response(response: reqwest::Response) -> Result<reqwest::Response, HltbError> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::FORBIDDEN {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        return Err(HltbError::RateLimited {
            retry_after,
            message: format!("HowLongToBeat temporarily limited requests ({status})"),
        });
    }
    response
        .error_for_status()
        .map_err(|error| HltbError::Other(format!("HowLongToBeat returned an error: {error}")))
}

fn default_headers() -> Result<reqwest::header::HeaderMap, HltbError> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/147.0.0.0 Safari/537.36"
            .parse()
            .map_err(|error| HltbError::Other(format!("Invalid HLTB user agent: {error}")))?,
    );
    headers.insert("origin", BASE_URL.parse().unwrap());
    headers.insert("referer", format!("{BASE_URL}/").parse().unwrap());
    Ok(headers)
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let time = httpdate::parse_http_date(value).ok()?;
    Some(time.duration_since(SystemTime::now()).unwrap_or_default())
}

fn positive_seconds(value: i64) -> Option<u32> {
    u32::try_from(value).ok().filter(|value| *value > 0)
}

fn positive_app_id(value: i64) -> Option<u32> {
    u32::try_from(value).ok().filter(|value| *value > 0)
}

fn unix_millis(time: SystemTime) -> u128 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn title_similarity(left: &str, right: &str) -> f64 {
    let left_tokens = normalise_title_tokens(left);
    let right_tokens = normalise_title_tokens(right);
    let left_numbers = left_tokens
        .iter()
        .filter(|token| token.chars().all(|character| character.is_ascii_digit()))
        .collect::<Vec<_>>();
    let right_numbers = right_tokens
        .iter()
        .filter(|token| token.chars().all(|character| character.is_ascii_digit()))
        .collect::<Vec<_>>();
    if left_numbers != right_numbers {
        return 0.0;
    }
    let left = left_tokens.concat();
    let right = right_tokens.concat();
    if left == right {
        return 1.0;
    }
    let distance = levenshtein(&left, &right);
    let length = left.chars().count().max(right.chars().count()).max(1);
    1.0 - distance as f64 / length as f64
}

fn normalise_title_tokens(value: &str) -> Vec<String> {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let mut tokens = cleaned
        .split_whitespace()
        .map(|token| token.to_lowercase())
        .map(|token| match token.as_str() {
            "i" => "1".to_string(),
            "ii" => "2".to_string(),
            "iii" => "3".to_string(),
            "iv" => "4".to_string(),
            "v" => "5".to_string(),
            "vi" => "6".to_string(),
            _ => token,
        })
        .collect::<Vec<_>>();
    strip_edition_suffix(&mut tokens);
    tokens
}

fn search_terms(value: &str) -> Vec<String> {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else if character == '\'' || character == '’' {
                '\''
            } else {
                ' '
            }
        })
        .collect::<String>();
    let mut tokens = cleaned
        .split_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    strip_edition_suffix(&mut tokens);
    tokens
}

fn strip_edition_suffix(tokens: &mut Vec<String>) {
    const SUFFIXES: &[&[&str]] = &[
        &["game", "of", "the", "year", "edition"],
        &["definitive", "edition"],
        &["complete", "edition"],
        &["ultimate", "edition"],
        &["directors", "cut"],
        &["director's", "cut"],
        &["goty", "edition"],
        &["remastered"],
        &["legacy"],
    ];
    let lowercase = tokens
        .iter()
        .map(|token| token.to_lowercase())
        .collect::<Vec<_>>();
    if let Some(suffix) = SUFFIXES.iter().find(|suffix| {
        lowercase.len() >= suffix.len()
            && lowercase[lowercase.len() - suffix.len()..]
                .iter()
                .map(String::as_str)
                .eq(suffix.iter().copied())
    }) {
        tokens.truncate(tokens.len() - suffix.len());
    }
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_character) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_character) in right.iter().enumerate() {
            current.push(
                (previous[right_index + 1] + 1)
                    .min(current[right_index] + 1)
                    .min(previous[right_index] + usize::from(left_character != *right_character)),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, name: &str, app_id: Option<u32>, similarity: f64) -> HltbCandidate {
        HltbCandidate {
            game_id: id,
            game_name: name.to_string(),
            main_seconds: Some(3_600),
            main_extra_seconds: Some(7_200),
            completionist_seconds: Some(10_800),
            steam_app_id: app_id,
            platforms: "PC".to_string(),
            similarity,
        }
    }

    #[test]
    fn steam_app_id_wins_over_a_better_title_match() {
        let candidates = vec![
            candidate(1, "Exact title", None, 1.0),
            candidate(2, "Different edition", Some(42), 0.7),
        ];
        assert_eq!(select_match(&candidates, 42, None).unwrap().game_id, 2);
    }

    #[test]
    fn title_fallback_is_conservative() {
        assert!(select_match(&[candidate(1, "Fixture", None, 0.95)], 42, None).is_some());
        assert!(select_match(&[candidate(1, "Fixture", None, 0.85)], 42, None).is_none());
    }

    #[test]
    fn normalises_punctuation_and_trademark_symbols() {
        assert_eq!(
            title_similarity("Batman™: Arkham Knight", "Batman: Arkham Knight"),
            1.0
        );
    }

    #[test]
    fn does_not_treat_a_numbered_sequel_as_the_same_title() {
        assert!(title_similarity("Cities: Skylines", "Cities: Skylines II") < 0.92);
    }

    #[test]
    fn treats_roman_numerals_and_edition_suffixes_as_presentation_differences() {
        assert_eq!(
            title_similarity("Slay the Spire 2", "Slay the Spire II"),
            1.0
        );
        assert_eq!(title_similarity("BioShock Remastered", "BioShock"), 1.0);
    }

    #[test]
    fn cleans_steam_punctuation_from_search_terms() {
        assert_eq!(
            search_terms("Batman™: Arkham Knight"),
            ["Batman", "Arkham", "Knight"]
        );
        assert_eq!(
            search_terms("Marvel’s Spider-Man Remastered"),
            ["Marvel's", "Spider", "Man"]
        );
    }

    #[test]
    fn aliases_cannot_make_a_sequel_look_like_the_original() {
        let result = SearchResult {
            game_id: 1,
            game_name: "Cities: Skylines II".to_string(),
            comp_main: 1,
            comp_plus: 2,
            comp_100: 3,
            profile_steam: 0,
            profile_steam_alt: 0,
            profile_platform: "PC".to_string(),
        };
        assert!(result.into_candidate("Cities: Skylines").similarity < 0.92);
    }

    #[test]
    fn accepts_hltb_endless_game_results() {
        let result: SearchResult = serde_json::from_value(serde_json::json!({
            "game_id": 44128,
            "game_name": "Morphblade",
            "game_type": "endless",
            "comp_main": 7956,
            "comp_plus": 0,
            "comp_100": 54000,
            "profile_steam": 0,
            "profile_steam_alt": 0,
            "profile_platform": "PC"
        }))
        .unwrap();
        let candidate = result.into_candidate("Morphblade");
        assert_eq!(candidate.similarity, 1.0);
        assert_eq!(candidate.main_seconds, Some(7956));
    }
}
