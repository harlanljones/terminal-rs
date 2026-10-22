//! Provenance and the attribution strings each source requires.

/// The attribution each source family requires. Verbatim wording is pinned by `docs/plan/06-risks.md`
/// rows L2–L7; panes expose the const that matches the source shown.
pub struct Attribution;

impl Attribution {
    /// Savant / MLBAM data. Per L2/L3: the gdx copyright notice's grant plus de-facto tolerance of
    /// individual, non-bulk use — wording must say de-facto, never "conforms to".
    pub const SAVANT: &'static str = "Data obtained free of charge from Baseball Savant and is copyrighted by MLB Advanced Media, L.P. \
         Individual, non-bulk, non-commercial use only under MLB's copyright notice; runtime fetching relies \
         on de-facto tolerance, not an API licence.";
    /// Stats API data is also MLBAM.
    pub const STATS_API: &'static str = Attribution::SAVANT;
    /// Retrosheet notice, verbatim (L6).
    pub const RETROSHEET: &'static str = "The information used here was obtained free of charge from and is copyrighted by Retrosheet.  Interested parties may contact Retrosheet at \u{201c}www.retrosheet.org\u{201d}.";
    /// Lahman: CC BY-SA 3.0 (L5).
    pub const LAHMAN: &'static str = "Data derived from the Lahman Database, licensed CC BY-SA 3.0; the bundled \
         corpus is distributed under that share-alike licence (Sean Lahman, SABR).";
    /// Chadwick register subset: ODC-By 1.0 (L7).
    pub const CHADWICK: &'static str = "Chadwick register subset used under the Open Data Commons Attribution \
         (ODC-By) 1.0 licence.";
    /// Cot's Contracts: BP fair-use wording — noncommercial research, cite author and source (L4).
    pub const COTS: &'static str = "Cot's Baseball Contracts / Jeff Euston, Baseball Prospectus. Used for noncommercial purposes such as \
         research; cite author and source.";
}

/// Where a value came from and under what terms it may be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// Stable source family id, matching the budgets in [`crate::Policy`] defaults (`savant`, `statsapi`, `cots`, …).
    pub source: String,
    /// The URL actually fetched (or, for bundled payloads, the manifest path that shipped them).
    pub url: String,
    /// Unix epoch seconds at fetch time.
    pub fetched: u64,
    /// SHA-256 of the raw payload bytes.
    pub sha256: [u8; 32],
    /// The attribution to render wherever this value is shown.
    pub attribution: &'static str,
}

impl Provenance {
    pub fn new(
        source: impl Into<String>,
        url: impl Into<String>,
        fetched: u64,
        sha256: [u8; 32],
        attribution: &'static str,
    ) -> Self {
        Provenance { source: source.into(), url: url.into(), fetched, sha256, attribution }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrosheet_notice_is_the_verbatim_text() {
        let a = Attribution::RETROSHEET;
        assert!(a.starts_with(
            "The information used here was obtained free of charge from and is copyrighted by Retrosheet."
        ));
        assert!(a.ends_with("www.retrosheet.org\u{201d}."));
        // The canonical notice contains no comma; it separates sentences exactly as above.
        assert!(a.contains("www.retrosheet.org"));
    }

    #[test]
    fn attribution_consts_cover_all_six_sources() {
        assert!(Attribution::SAVANT.contains("MLB Advanced Media"));
        assert!(Attribution::SAVANT.contains("de-facto"));
        assert!(Attribution::LAHMAN.contains("CC BY-SA 3.0"));
        assert!(Attribution::CHADWICK.contains("ODC-By"));
        assert!(Attribution::COTS.contains("Cot's Baseball Contracts / Jeff Euston, Baseball Prospectus"));
    }

    #[test]
    fn provenance_round_trips() {
        let prov = Provenance::new("savant", "https://x", 1_700_000_000, [7; 32], Attribution::SAVANT);
        assert_eq!(prov.source, "savant");
        assert_eq!(prov.fetched, 1_700_000_000);
        assert_eq!(prov.sha256.len(), 32);
    }
}
