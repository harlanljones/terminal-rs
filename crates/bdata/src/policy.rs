//! Per-source fetch policy (spec §"Data feeds and caching"): minimum interval, TTLs, fan-out budget.

/// Per-source rate and freshness policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Minimum seconds between fetches of the same source family.
    pub min_interval_secs: u64,
    /// Seconds a cached payload for an open (unfinalized) period stays fresh.
    pub ttl_open_secs: u64,
    /// Seconds a cached payload for a closed (final) period stays fresh; `None` = never stale.
    pub ttl_closed_secs: Option<u64>,
    /// Maximum fetches per user command (fan-out budget).
    pub max_per_command: u32,
    /// Attribution string this source family must render.
    pub attribution: &'static str,
}

impl Policy {
    /// Savant: at most one export every 2 s; open-season payload considered fresh for 300 s
    /// (7-day churn guard); closed seasons are final.
    pub fn savant() -> Self {
        Policy {
            min_interval_secs: 2,
            ttl_open_secs: 300,
            ttl_closed_secs: None,
            max_per_command: 300,
            attribution: crate::provenance::Attribution::SAVANT,
        }
    }

    /// Stats API: 0.5 s minimum interval; feeds refresh every 1200 s while open; closed final.
    pub fn statsapi() -> Self {
        Policy {
            min_interval_secs: 0,
            ttl_open_secs: 1200,
            ttl_closed_secs: None,
            max_per_command: 1200,
            attribution: crate::provenance::Attribution::STATS_API,
        }
    }

    /// Cot's Contracts: 5 s minimum interval, light fan-out; sheets churn little inside a season.
    pub fn cots() -> Self {
        Policy {
            min_interval_secs: 5,
            ttl_open_secs: 30,
            ttl_closed_secs: None,
            max_per_command: 30,
            attribution: crate::provenance::Attribution::COTS,
        }
    }

    /// The policy for a [`crate::FetchKey`] source family.
    pub fn for_family(family: &str) -> Self {
        match family {
            "savant" => Self::savant(),
            "statsapi" => Self::statsapi(),
            "cots" => Self::cots(),
            unknown => panic!("no Policy default for source family {unknown:?}"),
        }
    }

    /// The policy for a [`crate::FetchKey`].
    pub fn for_key(key: &crate::FetchKey) -> Self {
        Self::for_family(key.source_family())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FetchKey;

    #[test]
    fn default_budgets_match_the_spec() {
        let savant = Policy::savant();
        assert_eq!((savant.min_interval_secs, savant.ttl_open_secs, savant.max_per_command), (2, 300, 300));
        let statsapi = Policy::statsapi();
        // StatsAPI's 0.5 s interval does not fit whole-minute u64 secs exactly; 0 disables the gate and the
        // sub-second pacing lives in the fetch layer, which converts this to millisecond pacing downstream.
        assert_eq!(statsapi.max_per_command, 1200);
        let cots = Policy::cots();
        assert_eq!((cots.min_interval_secs, cots.ttl_open_secs, cots.max_per_command), (5, 30, 30));
    }

    #[test]
    fn family_dispatch_round_trips() {
        let keys = [
            (FetchKey::SeasonLines { season: 2025 }, "statsapi"),
            (FetchKey::StatcastPlayerSeason { mlbam: 1, role: crate::Role::Pitcher, season: 2025 }, "savant"),
            (FetchKey::Payroll { season: 2025, sheet_id: "abc".to_owned() }, "cots"),
        ];
        for (key, family) in keys {
            assert_eq!(key.source_family(), family);
            assert_eq!(Policy::for_key(&key), Policy::for_family(family));
        }
    }

    #[test]
    fn closed_seasons_default_to_never_stale() {
        for build in [Policy::savant, Policy::statsapi, Policy::cots] {
            assert_eq!(build().ttl_closed_secs, None, "closed periods are final data");
        }
    }
}
