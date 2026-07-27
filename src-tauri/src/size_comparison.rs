const BYTES_PER_GIB: f64 = 1_073_741_824.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstimateMode {
    Depot,
    Community,
    Compare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstimateSource {
    Local,
    Depot,
    Community,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeCandidates {
    pub local_bytes: Option<u64>,
    pub depot_bytes: Option<u64>,
    pub community_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SizeEstimate {
    pub lower_size_bytes: u64,
    pub upper_size_bytes: u64,
    pub lower_hours_per_gib: f64,
    pub upper_hours_per_gib: f64,
    pub sources: Vec<EstimateSource>,
    pub used_fallback: bool,
    pub size_ratio: Option<f64>,
}

pub fn estimate_size(
    playtime_minutes: u64,
    candidates: SizeCandidates,
    mode: EstimateMode,
) -> Option<SizeEstimate> {
    if let Some(local_bytes) = nonzero(candidates.local_bytes) {
        return Some(single_estimate(
            playtime_minutes,
            local_bytes,
            EstimateSource::Local,
            false,
        ));
    }

    let depot = nonzero(candidates.depot_bytes);
    let community = nonzero(candidates.community_bytes);
    match mode {
        EstimateMode::Depot => depot
            .map(|bytes| single_estimate(playtime_minutes, bytes, EstimateSource::Depot, false))
            .or_else(|| {
                community.map(|bytes| {
                    single_estimate(playtime_minutes, bytes, EstimateSource::Community, true)
                })
            }),
        EstimateMode::Community => community
            .map(|bytes| single_estimate(playtime_minutes, bytes, EstimateSource::Community, false))
            .or_else(|| {
                depot.map(|bytes| {
                    single_estimate(playtime_minutes, bytes, EstimateSource::Depot, true)
                })
            }),
        EstimateMode::Compare => match (depot, community) {
            (Some(depot), Some(community)) => {
                let lower_size_bytes = depot.min(community);
                let upper_size_bytes = depot.max(community);
                let (lower_hours_per_gib, upper_hours_per_gib) =
                    hours_per_gib_range(playtime_minutes, lower_size_bytes, upper_size_bytes);
                Some(SizeEstimate {
                    lower_size_bytes,
                    upper_size_bytes,
                    lower_hours_per_gib,
                    upper_hours_per_gib,
                    sources: vec![EstimateSource::Depot, EstimateSource::Community],
                    used_fallback: false,
                    size_ratio: Some(upper_size_bytes as f64 / lower_size_bytes as f64),
                })
            }
            (Some(bytes), None) => Some(single_estimate(
                playtime_minutes,
                bytes,
                EstimateSource::Depot,
                true,
            )),
            (None, Some(bytes)) => Some(single_estimate(
                playtime_minutes,
                bytes,
                EstimateSource::Community,
                true,
            )),
            (None, None) => None,
        },
    }
}

fn single_estimate(
    playtime_minutes: u64,
    bytes: u64,
    source: EstimateSource,
    used_fallback: bool,
) -> SizeEstimate {
    let hours_per_gib = hours_per_gib(playtime_minutes, bytes);
    SizeEstimate {
        lower_size_bytes: bytes,
        upper_size_bytes: bytes,
        lower_hours_per_gib: hours_per_gib,
        upper_hours_per_gib: hours_per_gib,
        sources: vec![source],
        used_fallback,
        size_ratio: None,
    }
}

fn hours_per_gib_range(
    playtime_minutes: u64,
    lower_size_bytes: u64,
    upper_size_bytes: u64,
) -> (f64, f64) {
    (
        hours_per_gib(playtime_minutes, upper_size_bytes),
        hours_per_gib(playtime_minutes, lower_size_bytes),
    )
}

fn hours_per_gib(playtime_minutes: u64, bytes: u64) -> f64 {
    (playtime_minutes as f64 / 60.0) / (bytes as f64 / BYTES_PER_GIB)
}

fn nonzero(value: Option<u64>) -> Option<u64> {
    value.filter(|bytes| *bytes > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> SizeCandidates {
        SizeCandidates {
            local_bytes: None,
            depot_bytes: Some(2 * 1_073_741_824),
            community_bytes: Some(4 * 1_073_741_824),
        }
    }

    #[test]
    fn local_size_is_authoritative_in_every_mode() {
        let mut sizes = candidates();
        sizes.local_bytes = Some(1_073_741_824);
        for mode in [
            EstimateMode::Depot,
            EstimateMode::Community,
            EstimateMode::Compare,
        ] {
            let estimate = estimate_size(120, sizes, mode).expect("local size exists");
            assert_eq!(estimate.sources, vec![EstimateSource::Local]);
            assert_eq!(estimate.lower_size_bytes, 1_073_741_824);
            assert_eq!(estimate.lower_hours_per_gib, 2.0);
            assert!(!estimate.used_fallback);
        }
    }

    #[test]
    fn compare_mode_reverses_the_hours_per_gib_range() {
        let estimate =
            estimate_size(600, candidates(), EstimateMode::Compare).expect("both sizes exist");
        assert_eq!(estimate.lower_size_bytes, 2 * 1_073_741_824);
        assert_eq!(estimate.upper_size_bytes, 4 * 1_073_741_824);
        assert_eq!(estimate.lower_hours_per_gib, 2.5);
        assert_eq!(estimate.upper_hours_per_gib, 5.0);
        assert_eq!(estimate.size_ratio, Some(2.0));
        assert!(!estimate.used_fallback);
    }

    #[test]
    fn source_modes_fall_back_explicitly() {
        let estimate = estimate_size(
            60,
            SizeCandidates {
                community_bytes: Some(1_073_741_824),
                ..Default::default()
            },
            EstimateMode::Depot,
        )
        .expect("community fallback exists");
        assert_eq!(estimate.sources, vec![EstimateSource::Community]);
        assert!(estimate.used_fallback);
    }

    #[test]
    fn missing_and_zero_sizes_do_not_become_zero_efficiency() {
        assert_eq!(
            estimate_size(
                600,
                SizeCandidates {
                    depot_bytes: Some(0),
                    ..Default::default()
                },
                EstimateMode::Compare,
            ),
            None
        );
    }
}
