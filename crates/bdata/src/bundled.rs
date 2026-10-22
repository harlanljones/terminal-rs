//! Bundled sample set: manifest-verified static data shipped under `sabr-data/sample`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Bundled data directory, relative to the repository root.
pub const BUNDLED_DIR: &str = "sabr-data/sample";

/// Files covered by `manifest.sha`. Paths are relative to [`BUNDLED_DIR`].
pub const MANIFEST_FILES: &[&str] = &[
    "lahman/Batting.csv",
    "lahman/Pitching.csv",
    "lahman/Appearances.csv",
    "lahman/People.csv",
    "lahman/Salaries.csv",
    "catalog.csv",
    "ohtani-2025-bat.csv",
    "ohtani-2025-pit.csv",
    "cut-lines-2001-2025.csv",
    "comparator-scores.csv",
];

/// Bundle of raw season lines parsed straight from the bundled Lahman CSVs.
///
/// `SeasonLine` proper is defined by the crate's data-primitives module; until it
/// lands, this exposes the raw parsed shape via [`raw::SeasonLineRaw`] plus a
/// conversion helper.
#[derive(Debug)]
pub struct BundledSet {
    /// Absolute or cwd-relative root of the bundled set (the directory holding `manifest.sha`).
    pub root: PathBuf,
}

/// Raw Lahman batting row restricted to the fields the season-lines loader needs.
pub mod raw {
    /// One player-season-team batting line, parsed verbatim from `lahman/Batting.csv`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SeasonLineRaw {
        pub player_id: String,
        pub season: u16,
        pub team: String,
        pub lg: String,
        pub ab: u32,
        pub h: u32,
        pub doubles: u32,
        pub triples: u32,
        pub hr: u32,
        pub bb: u32,
        pub ibb: u32,
        pub hbp: u32,
        pub sf: u32,
        pub sh: u32,
        pub so: u32,
    }
}

/// Errors raised while opening or reading a bundled set.
#[derive(Debug)]
pub enum BundledError {
    /// `manifest.sha` unreadable or unparseable.
    Manifest(String),
    /// A listed file is missing from disk.
    MissingFile(String),
    /// A listed file's sha256 does not match the manifest entry.
    HashMismatch { path: String, expected: String, actual: String },
    /// A CSV row could not be parsed.
    Csv(String),
    /// Underlying I/O failure.
    Io(std::io::Error),
}

impl std::fmt::Display for BundledError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manifest(m) => write!(f, "manifest: {m}"),
            Self::MissingFile(p) => write!(f, "missing file: {p}"),
            Self::HashMismatch { path, expected, actual } => {
                write!(f, "sha256 mismatch for {path}: expected {expected}, got {actual}")
            }
            Self::Csv(m) => write!(f, "csv: {m}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for BundledError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for BundledError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Per-file verification status. `false` means the hash did not match (an error condition
/// surfaced at open time, per the misconfiguration-is-an-error rule).
pub type FileStatus = (String, bool);

/// Strips a UTF-8 BOM from the first line of a CSV payload, if present.
/// Every other line is returned untouched.
fn strip_bom_lines(raw: &str) -> impl Iterator<Item = &str> {
    let body = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    body.lines()
}

/// Parses one CSV field: unquotes surrounding `"` and restores `""` to `"` inside.
fn csv_field(field: &str) -> String {
    let field = field.trim();
    if field.starts_with('"') && field.ends_with('"') && field.len() >= 2 {
        field[1..field.len() - 1].replace("\"\"", "\"")
    } else {
        field.to_string()
    }
}

/// Splits a CSV line into unquoted fields (sample data is comma-only; no embedded newlines).
fn csv_split(line: &str) -> Vec<String> {
    line.split(',').map(csv_field).collect()
}

/// Parses `u32` or fails loudly — saberkit rule: misconfiguration is an error, never a silent zero.
fn parse_u32(value: &str, what: &str) -> Result<u32, BundledError> {
    value.trim().parse::<u32>().map_err(|_| BundledError::Csv(format!("{what}: {value:?}")))
}

/// Parses `u16` (season years).
fn parse_u16(value: &str, what: &str) -> Result<u16, BundledError> {
    value.trim().parse::<u16>().map_err(|_| BundledError::Csv(format!("{what}: {value:?}")))
}

impl BundledSet {
    /// Opens the bundled set at `root`, reading `manifest.sha` up front. Every listed file's
    /// sha256 is verified; a mismatch is an error.
    pub fn open<P: AsRef<Path>>(root: P) -> Result<Self, BundledError> {
        let root = root.as_ref().to_path_buf();
        let set = Self { root };
        let statuses = set.verify()?;
        for (path, ok) in statuses {
            if !ok {
                // verify() already returned Err on mismatch, so this is unreachable in practice,
                // but the guard keeps the invariant honest.
                return Err(BundledError::MissingFile(path));
            }
        }
        Ok(set)
    }

    /// Verifies every bundled file. Paths listed in `manifest.sha` are sha256-checked;
    /// a mismatch is an error, a match is `true`. Required files ([`MANIFEST_FILES`]) that the
    /// manifest does not cover are still verified for existence (missing file is an error, and
    /// their status reflects that check only — no manifest hash is available to verify).
    pub fn verify(&self) -> Result<Vec<FileStatus>, BundledError> {
        let manifest_path = self.root.join("manifest.sha");
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|e| BundledError::Manifest(format!("{}: {e}", manifest_path.display())))?;
        let mut expected: Vec<(&str, &str)> = Vec::new();
        for (lineno, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (hash, path) = match (parts.next(), parts.next()) {
                (Some(h), Some(p)) => (h, p),
                _ => return Err(BundledError::Manifest(format!("line {}: expected '<sha256>  <path>'", lineno + 1))),
            };
            if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(BundledError::Manifest(format!("line {}: bad sha256 {hash:?}", lineno + 1)));
            }
            expected.push((path, hash));
        }
        let mut statuses = Vec::with_capacity(expected.len());
        for (path, hash) in &expected {
            let full = self.root.join(path);
            let bytes = fs::read(&full).map_err(|_| BundledError::MissingFile(path.to_string()))?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let actual = format!("{:x}", hasher.finalize());
            if actual != *hash {
                return Err(BundledError::HashMismatch { path: path.to_string(), expected: hash.to_string(), actual });
            }
            statuses.push((path.to_string(), true));
        }
        // Required files the manifest does not list: existence-check them so a dropped file is
        // still an error even though no manifest hash covers it.
        let manifested: Vec<String> = expected.iter().map(|(p, _)| (*p).to_string()).collect();
        for path in MANIFEST_FILES {
            if manifested.iter().any(|m| m == path) {
                continue;
            }
            if !self.root.join(path).exists() {
                return Err(BundledError::MissingFile((*path).to_string()));
            }
            statuses.push(((*path).to_string(), true));
        }
        Ok(statuses)
    }

    /// Loads player-season batting lines from the bundled Lahman CSVs (Batting 1998-2025,
    /// cross-referenced with People for player ids and Salaries/Appearances where useful).
    /// Returns raw rows; conversion to the crate's `SeasonLine` is a thin map.
    pub fn load_season_lines_raw(&self) -> Result<Vec<raw::SeasonLineRaw>, BundledError> {
        // catalog.csv: keyed by bbref id — used to confirm which players carry MLB ids.
        let catalog_path = self.root.join("catalog.csv");
        let catalog_raw =
            fs::read_to_string(&catalog_path).map_err(|_| BundledError::MissingFile("catalog.csv".into()))?;
        let mut catalog_keys: HashMap<String, ()> = HashMap::new();
        for (i, line) in strip_bom_lines(&catalog_raw).enumerate() {
            if i == 0 {
                continue; // header
            }
            let fields = csv_split(line);
            // key_bbref is column 4 (0-based); empty in some rows is fine.
            if let Some(bbref) = fields.get(4).filter(|s| !s.is_empty()) {
                catalog_keys.insert(bbref.clone(), ());
            }
        }

        // People.csv: playerID is column 2 (BOM-prefixed header).
        let people_path = self.root.join("lahman/People.csv");
        let people_raw =
            fs::read_to_string(&people_path).map_err(|_| BundledError::MissingFile("lahman/People.csv".into()))?;
        let mut people_ids: HashMap<String, ()> = HashMap::new();
        for (i, line) in strip_bom_lines(&people_raw).enumerate() {
            if i == 0 {
                continue;
            }
            let fields = csv_split(line);
            if let Some(pid) = fields.get(1).filter(|s| !s.is_empty()) {
                people_ids.insert(pid.clone(), ());
            }
        }

        // Batting.csv: playerID,yearID,stint,teamID,lgID,G,AB,R,H,2B,3B,HR,RBI,SB,CS,BB,SO,IBB,HBP,SH,SF,GIDP
        let batting_path = self.root.join("lahman/Batting.csv");
        let batting_raw =
            fs::read_to_string(&batting_path).map_err(|_| BundledError::MissingFile("lahman/Batting.csv".into()))?;
        let mut lines = Vec::new();
        for (i, line) in strip_bom_lines(&batting_raw).enumerate() {
            if i == 0 {
                continue;
            }
            let f = csv_split(line);
            if f.len() < 21 {
                return Err(BundledError::Csv(format!("Batting.csv line {}: {} fields, want >= 21", i + 1, f.len())));
            }
            let player_id = f[0].clone();
            if !people_ids.contains_key(&player_id) {
                continue; // row references a player not in People.csv — skip silently per Lahman convention
            }
            let season = parse_u16(&f[1], "Batting yearID")?;
            if !(1998..=2025).contains(&season) {
                continue;
            }
            let raw_line = raw::SeasonLineRaw {
                player_id,
                season,
                team: f[3].clone(),
                lg: f[4].clone(),
                ab: parse_u32(&f[6], "Batting AB")?,
                h: parse_u32(&f[8], "Batting H")?,
                doubles: parse_u32(&f[9], "Batting 2B")?,
                triples: parse_u32(&f[10], "Batting 3B")?,
                hr: parse_u32(&f[11], "Batting HR")?,
                bb: parse_u32(&f[15], "Batting BB")?,
                so: parse_u32(&f[16], "Batting SO")?,
                ibb: parse_u32(&f[17], "Batting IBB")?,
                hbp: parse_u32(&f[18], "Batting HBP")?,
                sh: parse_u32(&f[19], "Batting SH")?,
                sf: parse_u32(&f[20], "Batting SF")?,
            };
            let _ = catalog_keys.contains_key(&raw_line.player_id); // catalog cross-ref available for future filters
            lines.push(raw_line);
        }
        Ok(lines)
    }
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

    #[test]
    fn verify_green_on_real_sample() {
        let set = BundledSet::open(sample_root()).expect("open bundled set");
        let statuses = set.verify().expect("verify");
        // Manifest covers 9 files; MANIFEST_FILES adds cut-lines + comparator-scores (existence-
        // only, unmanifested) but not Teams.csv (manifest-only), so the union is 11.
        assert_eq!(statuses.len(), 11, "union of manifest + required files");
        assert!(statuses.iter().all(|(_, ok)| *ok), "all files must pass verification");
        // Every required file must be reported.
        for req in MANIFEST_FILES {
            assert!(statuses.iter().any(|(p, _)| p == req), "missing status for {req}");
        }
    }

    #[test]
    fn verify_catches_corruption() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // Minimal fake bundle: tiny manifest with a deliberately wrong hash.
        std::fs::write(root.join("catalog.csv"), "key_uuid\nabc\n").expect("write csv");
        std::fs::write(
            root.join("manifest.sha"),
            "0000000000000000000000000000000000000000000000000000000000000000  catalog.csv\n",
        )
        .expect("write manifest");
        let err = BundledSet::open(root).expect_err("mismatch must error");
        assert!(matches!(err, BundledError::HashMismatch { .. }), "got: {err}");
    }

    #[test]
    fn season_lines_exceed_30000() {
        let set = BundledSet::open(sample_root()).expect("open bundled set");
        let lines = set.load_season_lines_raw().expect("load season lines");
        assert!(lines.len() > 30_000, "expected > 30000 rows, got {}", lines.len());
        assert!(lines.iter().all(|l| (1998..=2025).contains(&l.season)));
    }

    #[test]
    fn bom_stripped_from_header() {
        let with_bom = "\u{feff}a,b\n1,2\n";
        let mut it = strip_bom_lines(with_bom);
        assert_eq!(it.next(), Some("a,b"));
        assert_eq!(it.next(), Some("1,2"));
    }

    #[test]
    fn csv_field_unquotes() {
        assert_eq!(csv_field("\"a,b\"\"c\""), "a,b\"c");
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field(""), "");
    }
}
