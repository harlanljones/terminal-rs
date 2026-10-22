//! Player catalog: the bundled Chadwick register (`catalog.csv`, 135,208 rows) with accent- and
//! suffix-folding search, ID lookups and role-filtered ranking.
//!
//! v1 rules (spec 03 §4.3, DR-3.39, P5): the register is parsed once into [`CatRow`] and search is
//! a linear scan — no search index. Search folds accents (í→i, ó→o, é→e, á→a, ñ→n, ü→u), case and
//! suffix tokens (`jr`/`sr`/`ii`/`iii`), then ranks: (1) exact full-name token match, (2) exact
//! last name, (3) last-name prefix, (4) first/last substring. Ties inside a class break by a
//! ROLE-NEUTRAL weight — career-span length (`mlb_played` span + `pro_played` span), not PA+IP
//! volume, which P5 measured burying Edwin Díaz 6th of 10 `DIAZ` matches. A role filter applies
//! before ranking, cutting rows whose role evidence lacks the requested role.
//!
//! Role evidence: the register itself carries no role column, so v1 derives role from the bundled
//! Lahman `lahman/Appearances.csv` (`G_p > 0` ⇒ pitch, `G_batting > 0` ⇒ bat), keyed by bbref id,
//! when that file sits beside the register. Rows without evidence are excluded under a role
//! filter (an unknown role never satisfies a filter).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::bundled::BundledError;
use crate::key::Role;

/// Role-evidence bitmask stored per bbref id.
const BAT: u8 = 1;
const PITCH: u8 = 2;

/// One catalog row: the fourteen register columns (empty → `None`) plus per-row caches computed
/// once at load (folded names, career-span weight) so search stays cheap per keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatRow {
    pub key_uuid: String,
    pub key_mlbam: Option<u32>,
    pub key_retro: Option<String>,
    pub key_bbref: Option<String>,
    pub key_bbref_minors: Option<String>,
    pub key_fangraphs: Option<String>,
    pub name_last: String,
    pub name_first: Option<String>,
    pub name_given: Option<String>,
    pub birth_year: Option<u16>,
    pub mlb_played_first: Option<u16>,
    pub mlb_played_last: Option<u16>,
    pub pro_played_first: Option<u16>,
    pub pro_played_last: Option<u16>,

    /// Folded (ASCII, lowercase) last name.
    fold_last: String,
    /// Folded first name.
    fold_first: String,
    /// Folded full-name token sequence (`first last`, whitespace-normalised).
    fold_full: String,
    /// Role-neutral weight: `mlb_played` span + `pro_played` span, in seasons (a span counts only
    /// when both endpoints are present).
    weight: u32,
}

impl CatRow {
    /// Role-neutral picker weight (see module docs): an inclusive season span per source, summed.
    fn span_weight(mlb: (Option<u16>, Option<u16>), pro: (Option<u16>, Option<u16>)) -> u32 {
        let span = |(a, b): (Option<u16>, Option<u16>)| match (a, b) {
            (Some(a), Some(b)) => b.saturating_sub(a) as u32 + 1,
            _ => 0,
        };
        span(mlb) + span(pro)
    }

    fn parse(fields: &[String]) -> Result<Self, BundledError> {
        let bad = |what: &str, value: &str| BundledError::Csv(format!("{what}: {value:?}"));
        if fields.len() != 14 {
            return Err(BundledError::Csv(format!(
                "catalog row has {} fields, expected 14: {:?}",
                fields.len(),
                fields.first(),
            )));
        }
        let text = |i: usize| -> Option<String> { (!fields[i].is_empty()).then(|| fields[i].clone()) };
        let year = |i: usize, what: &str| -> Result<Option<u16>, BundledError> {
            if fields[i].is_empty() {
                Ok(None)
            } else {
                fields[i].parse::<u16>().map(Some).map_err(|_| bad(what, &fields[i]))
            }
        };
        let mlbam = if fields[1].is_empty() {
            None
        } else {
            Some(fields[1].parse::<u32>().map_err(|_| bad("key_mlbam", &fields[1]))?)
        };
        let mlb_first = year(10, "mlb_played_first")?;
        let mlb_last = year(11, "mlb_played_last")?;
        let pro_first = year(12, "pro_played_first")?;
        let pro_last = year(13, "pro_played_last")?;
        let fold_last = fold(&fields[6]);
        let fold_first = fold(&fields[7]);
        let fold_full = [&fold_first, " ", &fold_last].concat().split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(Self {
            key_uuid: fields[0].clone(),
            key_mlbam: mlbam,
            key_retro: text(2),
            key_bbref: text(3),
            key_bbref_minors: text(4),
            key_fangraphs: text(5),
            name_last: fields[6].clone(),
            name_first: text(7),
            name_given: text(8),
            birth_year: year(9, "birth_year")?,
            mlb_played_first: mlb_first,
            mlb_played_last: mlb_last,
            pro_played_first: pro_first,
            pro_played_last: pro_last,
            fold_last,
            fold_first,
            fold_full,
            weight: Self::span_weight((mlb_first, mlb_last), (pro_first, pro_last)),
        })
    }
}

/// The parsed register with role evidence. Load once (typically from the bundled sample root) and
/// invoke `search` per keystroke.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    pub rows: Vec<CatRow>,
    /// Role evidence by bbref id (`G_p > 0` ⇒ pitch bit, `G_batting > 0` ⇒ bat bit).
    roles: HashMap<String, u8>,
}

impl Catalog {
    /// Loads `catalog.csv` (BOM-tolerant) and — when present — `lahman/Appearances.csv` role
    /// evidence from the given directory (the bundled sample root, `sabr-data/sample`).
    pub fn open<P: Into<PathBuf>>(root: P) -> Result<Self, BundledError> {
        let root = root.into();
        let path = root.join("catalog.csv");
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| BundledError::MissingFile(format!("{}: {e}", path.display())))?;
        let mut roles = HashMap::new();
        if let Ok(appearances) = std::fs::read_to_string(root.join("lahman/Appearances.csv")) {
            parse_role_bits(&appearances, &mut roles);
        }
        let mut lines = raw.strip_prefix('\u{feff}').unwrap_or(&raw).lines();
        let header = lines.next().ok_or_else(|| BundledError::Csv("catalog.csv is empty".into()))?;
        if !header.starts_with("key_uuid,") {
            return Err(BundledError::Csv(format!("unexpected catalog header: {header:?}")));
        }
        let mut rows = Vec::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            rows.push(CatRow::parse(&csv_split(line))?);
        }
        Ok(Self { rows, roles })
    }

    /// Search keeping each hit's rank class (lower is better; exact name/path classes sort
    /// first), for callers that need to tell exact matches from substrings (the app's entity
    /// resolver). Accent-, case- and suffix-folding; capped at 50 results; `role` cuts the
    /// candidate set before ranking.
    pub fn search_ranked(&self, q: &str, role: Option<Role>) -> Vec<(&CatRow, u8)> {
        let mut tokens: Vec<String> = q.split_whitespace().map(fold).collect();
        // Suffix fold (spec: "folds accents and suffixes"): drop a trailing `jr`/`sr`/`ii`/`iii`
        // token so `TATIS JR`, `TATÍS JR` and `tatis` rank identically.
        if let Some(suffix) = tokens.last().map(String::as_str)
            && matches!(suffix, "jr" | "sr" | "ii" | "iii")
        {
            tokens.pop();
        }
        let q = tokens.join(" ");
        let mut hits: Vec<(&CatRow, u8)> = Vec::new();
        for row in &self.rows {
            if let Some(role) = role {
                let bits = row.key_bbref.as_deref().and_then(|id| self.roles.get(id).copied()).unwrap_or(0);
                let need = match role {
                    Role::Batter => BAT,
                    Role::Pitcher => PITCH,
                };
                if bits & need == 0 {
                    continue;
                }
            }
            let Some(rank) = rank_of(&q, &tokens, row) else { continue };
            hits.push((row, rank));
        }
        // Rank class, then the role-neutral weight (span length, longer first), then stable
        // identity ordering for determinism.
        hits.sort_by_key(|(r, rank)| {
            (*rank, std::cmp::Reverse(r.weight), r.fold_last.clone(), r.fold_first.clone(), r.key_uuid.clone())
        });
        hits.into_iter().take(50).collect()
    }

    /// Accent-, case- and suffix-folding search ranked per the module docs and capped at 50
    /// results. `role` cuts the candidate set before ranking.
    pub fn search(&self, q: &str, role: Option<Role>) -> Vec<&CatRow> {
        self.search_ranked(q, role).into_iter().map(|(row, _)| row).collect()
    }
    /// Role evidence for a row: `(bats, pitches)` from the bundled Lahman Appearances file.
    pub fn roles(&self, row: &CatRow) -> (bool, bool) {
        let bits = row.key_bbref.as_deref().and_then(|id| self.roles.get(id).copied()).unwrap_or(0);
        (bits & BAT != 0, bits & PITCH != 0)
    }

    /// Row with the given MLBAM id, if any.
    pub fn by_mlbam(&self, mlbam: u32) -> Option<&CatRow> {
        self.rows.iter().find(|r| r.key_mlbam == Some(mlbam))
    }

    /// Role-neutral picker weight (career span in seasons, longer first).
    pub fn weight(&self, row: &CatRow) -> u32 {
        row.weight
    }

    /// Row with the given register UUID, if any.
    pub fn by_key_uuid(&self, uuid: &str) -> Option<&CatRow> {
        self.rows.iter().find(|r| r.key_uuid == uuid)
    }

    /// MLBAM id of the row with the given bbref id.
    pub fn mlbam_of(&self, bbref: &str) -> Option<u32> {
        self.rows.iter().find(|r| r.key_bbref.as_deref() == Some(bbref)).and_then(|r| r.key_mlbam)
    }
}

/// Rank classes; lower is better.
fn rank_of(q: &str, tokens: &[String], row: &CatRow) -> Option<u8> {
    // (1) exact full-name token match: the folded query appears as a contiguous token run of the
    // folded full name (`first last`).
    if !q.is_empty() && q.len() <= row.fold_full.len() && row.fold_full.contains(q) {
        return Some(0);
    }
    // (2) exact last name.
    if row.fold_last == q {
        return Some(1);
    }
    // (3) last-name prefix.
    if row.fold_last.starts_with(q) && !q.is_empty() {
        return Some(2);
    }
    // (4) first/last substring match on any query token.
    for token in tokens {
        if !token.is_empty() && (row.fold_first.contains(token.as_str()) || row.fold_last.contains(token.as_str())) {
            return Some(3);
        }
    }
    None
}

/// Static accent fold: NFD-like decomposition to ASCII for the common Latin accented letters.
/// Anything without an entry passes through; case folds via lowercase. (No Unicode deps.)
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for c in c.to_lowercase() {
            match c {
                'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => out.push('a'),
                'é' | 'è' | 'ë' | 'ê' => out.push('e'),
                'í' | 'ì' | 'ï' | 'î' => out.push('i'),
                'ó' | 'ò' | 'ö' | 'ô' | 'õ' => out.push('o'),
                'ú' | 'ù' | 'ü' | 'û' => out.push('u'),
                'ñ' => out.push('n'),
                'ç' => out.push('c'),
                'ø' => out.push('o'),
                'ý' => out.push('y'),
                _ => out.push(c),
            }
        }
    }
    out
}

/// Fills a bbref→role-bits map from the given Lahman `Appearances.csv` text: `G_p > 0` pitches,
/// `G_batting > 0` bats.
fn parse_role_bits(raw: &str, roles: &mut HashMap<String, u8>) {
    for line in raw.strip_prefix('\u{feff}').unwrap_or(raw).lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 21 || fields[3].is_empty() {
            continue;
        }
        let n = |i: usize| fields[i].parse::<u32>().unwrap_or(0);
        let bits = roles.entry(fields[3].to_string()).or_insert(0);
        if n(8) > 0 {
            *bits |= PITCH;
        }
        if n(6) > 0 {
            *bits |= BAT;
        }
    }
}

/// Splits one CSV line into unquoted fields (sample data is comma-only; no embedded newlines).
fn csv_split(line: &str) -> Vec<String> {
    line.split(',')
        .map(|field| {
            let field = field.trim();
            if field.starts_with('"') && field.ends_with('"') && field.len() >= 2 {
                field[1..field.len() - 1].replace("\"\"", "\"")
            } else {
                field.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_root() -> PathBuf {
        // Tests must work from any cwd: SABR_SAMPLE_DIR overrides, else repo-root-relative.
        std::env::var("SABR_SAMPLE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sabr-data/sample")))
    }

    fn catalog() -> Catalog {
        Catalog::open(sample_root()).expect("open catalog")
    }

    fn edwin_diaz_mlbam(catalog: &Catalog) -> u32 {
        // Find him by name in the real register: first Edwin, last Díaz, mlb debut 2016.
        let row = catalog
            .rows
            .iter()
            .find(|r| {
                r.name_last == "Díaz" && r.name_first.as_deref() == Some("Edwin") && r.mlb_played_first == Some(2016)
            })
            .expect("Edwin Díaz row");
        row.key_mlbam.expect("mlbam")
    }

    #[test]
    fn rows_loaded_and_ids_populated() {
        let catalog = catalog();
        assert_eq!(catalog.rows.len(), 135_208);
        let row = catalog.by_key_uuid("000007d9-a2b6-47fd-bc61-e582f190e74a").expect("first row");
        assert_eq!(row.name_last, "Garcia");
        assert_eq!(row.name_first.as_deref(), Some("Adolfito"));
        assert_eq!(row.birth_year, Some(1985));
        assert_eq!(row.mlb_played_first, None);
        assert_eq!((row.pro_played_first, row.pro_played_last), (Some(2006), Some(2008)));
    }

    #[test]
    fn diaz_with_pitch_role_filter_ranks_edwin_top3() {
        let catalog = catalog();
        let mlbam = edwin_diaz_mlbam(&catalog);
        let hits = catalog.search("DIAZ", Some(Role::Pitcher));
        assert!(!hits.is_empty());
        assert!(hits.len() <= 50, "cap applies");
        let rank = hits.iter().position(|r| r.key_mlbam == Some(mlbam)).expect("Edwin Díaz ranked");
        assert!(rank < 3, "Edwin Díaz should rank within the first 3, got {rank}");
    }

    #[test]
    fn tatis_jr_and_tatis_agree_on_one_id() {
        let catalog = catalog();
        let a = catalog.search("TATIS JR", None);
        let b = catalog.search("TATÍS JR", None);
        let c = catalog.search("tatis", None);
        assert!(!a.is_empty(), "tatis should match");
        assert_eq!(a, b, "accent fold");
        assert_eq!(a, c, "suffix fold");
        assert_eq!(a[0], c[0], "one ranked id across all three queries");
        assert!(!a.is_empty());
    }

    #[test]
    fn acuna_folds_accent() {
        let catalog = catalog();
        let a = catalog.search("ACUNA", None);
        let b = catalog.search("ACUÑA", None);
        assert_eq!(a, b);
        assert_eq!(a.first().and_then(|r| r.key_mlbam), Some(660_670), "Ronald Acuña top by span");
    }

    #[test]
    fn garbage_query_is_empty() {
        let catalog = catalog();
        assert!(catalog.search("zzqx", None).is_empty());
        assert!(catalog.search("", None).is_empty());
    }

    #[test]
    fn search_caps_at_50() {
        let catalog = catalog();
        assert_eq!(catalog.search("hernandez", None).len(), 50);
    }

    #[test]
    fn lookups_by_id() {
        let catalog = catalog();
        let row = catalog.by_mlbam(660_670).expect("Ronald Acuña by mlbam");
        assert_eq!(row.name_last, "Acuña");
        assert_eq!(catalog.mlbam_of("diazed04"), Some(621_242));
        assert!(catalog.by_mlbam(999_999_999).is_none());
    }

    #[test]
    fn fold_map_covers_specified_letters() {
        assert_eq!(fold("Díaz"), "diaz");
        assert_eq!(fold("Tatís"), "tatis");
        assert_eq!(fold("ACUÑA"), "acuna");
        assert_eq!(fold("Verdelü"), "verdelu");
        assert_eq!(fold("José"), "jose");
    }
}
