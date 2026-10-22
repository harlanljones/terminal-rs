//! Explicit, stable fetch keys. Byte-for-byte stability is part of the contract: these strings appear
//! verbatim in the SQLite index and in user-facing panes, so changing the format must be a deliberate
//! migration, never a refactor accident.

use chrono::NaiveDate;
use std::fmt;

/// Baseball role of a player within a season.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Batter,
    Pitcher,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Batter => "batter",
            Role::Pitcher => "pitcher",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The request being built. `Fetch` is the user-facing form; `FetchKey::as_str`/`to_string` renders the
/// stable on-disk key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    /// One player's Statcast season, per role (the Savant CSV is fetched per role).
    StatcastPlayerSeason { mlbam: u32, role: Role, season: u16 },
    /// One season's odds/cut lines feed from the Stats API.
    SeasonLines { season: u16 },
    /// One day's transaction feed from the Stats API.
    TransactionsDay { day: NaiveDate },
    /// One team's MLB roster for a season.
    Roster { team: String, season: u16 },
    /// One minor-league sport level's schedule/lines.
    MilbSeasonLines { sport_id: u32, season: u16 },
    /// One draft year's picks feed.
    Draft { year: u16 },
    /// One Cot's Contracts google sheet for a season.
    Payroll { season: u16, sheet_id: String },
}

/// Stable cache keys built from [`Fetch`]. Explicit per-variant rendering: no string concatenation
/// by callers, no format drift between variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchKey {
    StatcastPlayerSeason { mlbam: u32, role: Role, season: u16 },
    SeasonLines { season: u16 },
    TransactionsDay { day: NaiveDate },
    Roster { team: String, season: u16 },
    MilbSeasonLines { sport_id: u32, season: u16 },
    Draft { year: u16 },
    Payroll { season: u16, sheet_id: String },
}

impl From<Fetch> for FetchKey {
    fn from(fetch: Fetch) -> Self {
        match fetch {
            Fetch::StatcastPlayerSeason { mlbam, role, season } => {
                FetchKey::StatcastPlayerSeason { mlbam, role, season }
            }
            Fetch::SeasonLines { season } => FetchKey::SeasonLines { season },
            Fetch::TransactionsDay { day } => FetchKey::TransactionsDay { day },
            Fetch::Roster { team, season } => FetchKey::Roster { team, season },
            Fetch::MilbSeasonLines { sport_id, season } => FetchKey::MilbSeasonLines { sport_id, season },
            Fetch::Draft { year } => FetchKey::Draft { year },
            Fetch::Payroll { season, sheet_id } => FetchKey::Payroll { season, sheet_id },
        }
    }
}

impl From<&Fetch> for FetchKey {
    fn from(fetch: &Fetch) -> Self {
        Self::from(fetch.clone())
    }
}

impl Fetch {
    /// The stable cache key for this request.
    pub fn key(&self) -> FetchKey {
        self.clone().into()
    }
}

impl FetchKey {
    /// The key string. Shape per variant:
    /// `statcast/player/{mlbam}/{role}/{season}`, `statsapi/season/{season}`,
    /// `statsapi/transactions/{day}`, `statsapi/roster/{team}/{season}`, `statsapi/milb/{sport}/{season}`,
    /// `statsapi/draft/{year}`, `cots/{season}/{sheet_id}`.
    pub fn as_str(&self) -> String {
        match self {
            FetchKey::StatcastPlayerSeason { mlbam, role, season } => {
                format!("statcast/player/{mlbam}/{}/{season}", role.as_str())
            }
            FetchKey::SeasonLines { season } => format!("statsapi/season/{season}"),
            FetchKey::TransactionsDay { day } => format!("statsapi/transactions/{}", day.format("%Y-%m-%d")),
            FetchKey::Roster { team, season } => format!("statsapi/roster/{team}/{season}"),
            FetchKey::MilbSeasonLines { sport_id, season } => format!("statsapi/milb/{sport_id}/{season}"),
            FetchKey::Draft { year } => format!("statsapi/draft/{year}"),
            FetchKey::Payroll { season, sheet_id } => format!("cots/{season}/{sheet_id}"),
        }
    }

    /// The source family this key belongs to, matching the budgets in [`Policy`] defaults.
    pub fn source_family(&self) -> &'static str {
        match self {
            FetchKey::StatcastPlayerSeason { .. } => "savant",
            FetchKey::SeasonLines { .. }
            | FetchKey::TransactionsDay { .. }
            | FetchKey::Roster { .. }
            | FetchKey::MilbSeasonLines { .. }
            | FetchKey::Draft { .. } => "statsapi",
            FetchKey::Payroll { .. } => "cots",
        }
    }
}

impl fmt::Display for FetchKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl From<FetchKey> for String {
    fn from(key: FetchKey) -> Self {
        key.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn all_seven() -> [Fetch; 7] {
        [
            Fetch::StatcastPlayerSeason { mlbam: 660271, role: Role::Pitcher, season: 2025 },
            Fetch::SeasonLines { season: 2025 },
            Fetch::TransactionsDay { day: NaiveDate::from_ymd_opt(2026, 3, 20).unwrap() },
            Fetch::Roster { team: "NYY".to_owned(), season: 2025 },
            Fetch::MilbSeasonLines { sport_id: 11, season: 2025 },
            Fetch::Draft { year: 2025 },
            Fetch::Payroll { season: 2025, sheet_id: "1LMPkFQpx7DMsp8fHHTG_xeAaWruiYpOq".to_owned() },
        ]
    }

    #[test]
    fn keys_match_the_documented_format() {
        let key_strings: Vec<String> = all_seven().iter().map(|f| f.key().as_str()).collect();
        assert!(key_strings.contains(&"statcast/player/660271/pitcher/2025".to_owned()));
        assert!(key_strings.contains(&"statsapi/season/2025".to_owned()));
        assert!(key_strings.contains(&"statsapi/transactions/2026-03-20".to_owned()));
        assert!(key_strings.contains(&"statsapi/roster/NYY/2025".to_owned()));
        assert!(key_strings.contains(&"statsapi/milb/11/2025".to_owned()));
        assert!(key_strings.contains(&"statsapi/draft/2025".to_owned()));
        assert!(key_strings.contains(&"cots/2025/1LMPkFQpx7DMsp8fHHTG_xeAaWruiYpOq".to_owned()));
    }

    #[test]
    fn keys_are_unique_nonempty_no_spaces_and_short() {
        let keys: Vec<String> = all_seven().iter().map(|f| f.key().as_str()).collect();
        for key in keys {
            assert!(!key.is_empty());
            assert!(!key.contains(' '), "spaces break path-shaped keys: {key}");
            assert!(!key.contains('\\'), "backslashes are path separators on windows: {key}");
            assert!(key.len() < 120, "keys must stay short: {key}");
        }
    }

    #[test]
    fn key_rendering_is_stable_across_repeat_calls() {
        let fetch = Fetch::StatcastPlayerSeason { mlbam: 660271, role: Role::Batter, season: 2025 };
        let first = fetch.key().as_str();
        assert_eq!(fetch.key().as_str(), first);
        assert_eq!(FetchKey::from(&fetch).as_str(), first);
    }

    #[test]
    fn roles_render_lowercased() {
        assert_eq!(Role::Batter.as_str(), "batter");
        assert_eq!(Role::Pitcher.as_str(), "pitcher");
    }
}
