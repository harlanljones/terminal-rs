//! Season-line corpus loaded from the bundled Lahman sample (spec §4.4 "Corpus").
//!
//! Loader choice (documented per assignment): the corpus reads `sabr-data/sample/lahman/*.csv`
//! directly with sabr-data's BOM-stripping approach rather than going through the bundled
//! loaders, because the forecast corpus needs raw season sums (AB, H, 2B, …) and birth
//! dates keyed by `playerID`, not the bundled loader's metric-shaped rows. The dependency on
//! `sabr-data` remains for `Attribution` provenance and the store pattern.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// One batting season line (Lahman Batting, stint-summed per player-season).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattingLine {
    pub player_id: String,
    pub season: u16,
    pub team: String,
    pub lg: String,
    /// Seasonal age under the July cut: `season - birthYear - (birthMonth > 6 ? 1 : 0)`.
    pub age: u16,
    pub pa: u32,
    pub ab: u32,
    pub h: u32,
    pub h2b: u32,
    pub h3b: u32,
    pub hr: u32,
    pub bb: u32,
    pub ibb: u32,
    pub hbp: u32,
    pub sf: u32,
    pub sh: u32,
    pub so: u32,
}

/// One to-date partial batting line, as of the day before an in-season cut (DR-3.37). The `as_of`
/// date is parsed into `month`/`day`; the season-to-date counters (PA = AB + BB + HBP + SF + SH)
/// come from the sample's cut-lines file. Pitching-only columns (bf/outs/r/er) are ignored on this
/// batting path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutLine {
    pub player_id: String,
    pub season: u16,
    /// Month (1–12) of the `as_of` date — the day before the cut.
    pub month: u8,
    /// Day of month of the `as_of` date.
    pub day: u8,
    pub pa: u32,
    pub ab: u32,
    pub h: u32,
    pub h2b: u32,
    pub h3b: u32,
    pub hr: u32,
    pub bb: u32,
    pub ibb: u32,
    pub hbp: u32,
    pub sf: u32,
    pub sh: u32,
    pub so: u32,
}

/// One pitching season line (Lahman Pitching, stint-summed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitchingLine {
    pub player_id: String,
    pub season: u16,
    pub team: String,
    pub lg: String,
    pub age: u16,
    /// Games and games started (Lahman `G`/`GS`); the starter/reliever role rule keys off them.
    pub g: u32,
    pub gs: u32,
    /// Recorded outs (IP*3 in Lahman terms; `IPouts`).
    pub ip_outs: u32,
    pub bf: u32,
    pub h: u32,
    pub er: u32,
    pub hr: u32,
    pub bb: u32,
    /// Intentional walks (Lahman `IBB`; needed by the BaseRuns-style ERA composite).
    pub ibb: u32,
    /// Hit batters (Lahman `HBP`; needed by the BaseRuns-style ERA composite).
    pub hbp: u32,
    pub so: u32,
}

/// The corpus: batting and pitching lines 1998–2025 (whatever the sample holds) plus birth data.
/// Rows are sorted by (player_id, season) for deterministic iteration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Corpus {
    pub batting: Vec<BattingLine>,
    pub pitching: Vec<PitchingLine>,
    /// playerID → (birthYear, birthMonth).
    pub birth: BTreeMap<String, (u16, u8)>,
    /// In-season cut lines (empty unless loaded with [`Corpus::from_lahman_with_cuts`]).
    pub cuts: Vec<CutLine>,
    /// SHA-256 of each input file consumed, hex-encoded, in fixed order (Batting, People, Pitching).
    pub input_sha256: [String; 3],
}

impl Corpus {
    /// Load from a Lahman directory containing `Batting.csv`, `People.csv`, `Pitching.csv`.
    /// No in-season cut lines (`cuts` stays empty).
    pub fn from_lahman(dir: &Path) -> std::io::Result<Corpus> {
        Corpus::from_lahman_with_cuts(dir, None)
    }

    /// [`Corpus::from_lahman`] plus in-season cut lines from a cut-lines CSV
    /// (`playerID,season,as_of,pa,ab,h,2b,3b,hr,bb,ibb,hbp,sf,sh,so,bf,outs,r,er`; BOM-stripped,
    /// empty lines skipped, `bf/outs/r/er` ignored on the batting path). Every cut-line player
    /// must exist in `People.csv`, else the loader errors naming the unmatched count.
    pub fn from_lahman_with_cuts(dir: &Path, cuts_csv: Option<&Path>) -> std::io::Result<Corpus> {
        let batting_raw = std::fs::read(dir.join("Batting.csv"))?;
        let people_raw = std::fs::read(dir.join("People.csv"))?;
        let pitching_raw = std::fs::read(dir.join("Pitching.csv"))?;
        let hex = |b: &[u8]| -> String {
            let d = Sha256::digest(b);
            d.iter().map(|x| format!("{x:02x}")).collect()
        };
        let input_sha256 = [hex(&batting_raw), hex(&people_raw), hex(&pitching_raw)];

        let mut birth = BTreeMap::new();
        // Retrosheet id → Lahman playerID (People's trailing `retroID` column). The bundled cut
        // lines are keyed by retro id (they come from the retro parser), so they join to the
        // corpus through this map.
        let mut retro_to_id: BTreeMap<String, String> = BTreeMap::new();
        let mut rows: BTreeMap<(String, u16), BattingLine> = BTreeMap::new();
        let mut prows: BTreeMap<(String, u16), PitchingLine> = BTreeMap::new();

        for rec in csv_records(&people_raw)? {
            let y = parse_u16(&rec[2], "birthYear");
            let m = parse_u8(&rec[3], "birthMonth");
            birth.insert(rec[1].clone(), (y, m));
            let retro = rec.get(24).map(|s| s.trim()).unwrap_or("");
            if !retro.is_empty() {
                retro_to_id.insert(retro.to_string(), rec[1].clone());
            }
        }

        for rec in csv_records(&batting_raw)? {
            let get = |i: usize| -> u32 { parse_u32(&rec[i], "batting") };
            let (id, season) = (rec[0].clone(), parse_u16(&rec[1], "yearID"));
            let (by, bm) = birth.get(&id).copied().unwrap_or((0, 0));
            let line = BattingLine {
                player_id: id.clone(),
                season,
                team: rec[3].clone(),
                lg: rec[4].clone(),
                age: season.saturating_sub(by).saturating_sub(u16::from(bm > 6)),
                ab: get(6),
                h: get(8),
                h2b: get(9),
                h3b: get(10),
                hr: get(11),
                bb: get(15),
                so: get(16),
                ibb: get(17),
                hbp: get(18),
                sh: get(19),
                sf: get(20),
                // Lahman has no PA column; PA = AB + BB + HBP + SF + SH.
                pa: 0,
            };
            let mut line = line;
            line.pa = line.ab + line.bb + line.hbp + line.sf + line.sh;
            match rows.entry((id, season)) {
                std::collections::btree_map::Entry::Occupied(mut e) => merge_batting(e.get_mut(), &line),
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(line);
                }
            }
        }

        for rec in csv_records(&pitching_raw)? {
            let get = |i: usize| -> u32 { parse_u32(&rec[i], "pitching") };
            let (id, season) = (rec[0].clone(), parse_u16(&rec[1], "yearID"));
            let (by, bm) = birth.get(&id).copied().unwrap_or((0, 0));
            let line = PitchingLine {
                player_id: id.clone(),
                season,
                team: rec[3].clone(),
                lg: rec[4].clone(),
                age: season.saturating_sub(by).saturating_sub(u16::from(bm > 6)),
                ip_outs: get(12),
                g: get(7),
                gs: get(8),
                h: get(13),
                er: get(14),
                hr: get(15),
                bb: get(16),
                ibb: get(20),
                hbp: get(22),
                so: get(17),
                bf: get(24),
            };
            match prows.entry((id, season)) {
                std::collections::btree_map::Entry::Occupied(mut e) => merge_pitching(e.get_mut(), &line),
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(line);
                }
            }
        }

        // In-season cut lines (optional). Deterministic order: (player_id, season, as_of).
        let mut cuts: Vec<CutLine> = Vec::new();
        if let Some(path) = cuts_csv {
            let raw = std::fs::read(path)?;
            let mut unmatched = 0usize;
            for rec in csv_records(&raw)? {
                let get = |i: usize| -> u32 { parse_u32(&rec[i], "cut lines") };
                let (id, season) = (rec[0].clone(), parse_u16(&rec[1], "cut season"));
                let ymd: Vec<&str> = rec[2].split('-').collect();
                if ymd.len() != 3 {
                    return Err(invalid(&format!("cut line: bad as_of date {:?}", rec[2])));
                }
                // Cut lines are keyed by retro id (retro-parser output); resolve to the Lahman
                // playerID. A cut line naming an unknown player is an error.
                let Some(player_id) = retro_to_id.get(id.trim()) else {
                    unmatched += 1;
                    continue;
                };
                let player_id = player_id.clone();
                cuts.push(CutLine {
                    player_id,
                    season,
                    month: parse_u8(ymd[1], "cut month"),
                    day: parse_u8(ymd[2], "cut day"),
                    pa: get(3),
                    ab: get(4),
                    h: get(5),
                    h2b: get(6),
                    h3b: get(7),
                    hr: get(8),
                    bb: get(9),
                    ibb: get(10),
                    hbp: get(11),
                    sf: get(12),
                    sh: get(13),
                    so: get(14),
                });
            }
            if unmatched > 0 {
                return Err(invalid(&format!("cut lines: {unmatched} row(s) name a player absent from People.csv")));
            }
            cuts.sort_by(|a, b| {
                (&a.player_id, a.season, a.month, a.day).cmp(&(&b.player_id, b.season, b.month, b.day))
            });
        }

        Ok(Corpus {
            batting: rows.into_values().collect(),
            pitching: prows.into_values().collect(),
            birth,
            cuts,
            input_sha256,
        })
    }

    /// Batting lines for one season, in (player_id, season) order.
    pub fn batting_season(&self, season: u16) -> Vec<&BattingLine> {
        self.batting.iter().filter(|l| l.season == season).collect()
    }

    /// Players with a batting line (any PA) in the season — the "appearance" half of the
    /// walk-forward population.
    pub fn batters_in(&self, season: u16) -> Vec<String> {
        let mut ids: Vec<String> =
            self.batting.iter().filter(|l| l.season == season).map(|l| l.player_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

fn merge_batting(a: &mut BattingLine, b: &BattingLine) {
    // Stint merge: keep the first stint's team/lg (highest stint) and sum counters. Age identical.
    a.pa += b.pa;
    a.ab += b.ab;
    a.h += b.h;
    a.h2b += b.h2b;
    a.h3b += b.h3b;
    a.hr += b.hr;
    a.bb += b.bb;
    a.ibb += b.ibb;
    a.hbp += b.hbp;
    a.sf += b.sf;
    a.sh += b.sh;
    a.so += b.so;
}

fn merge_pitching(a: &mut PitchingLine, b: &PitchingLine) {
    a.ip_outs += b.ip_outs;
    a.g += b.g;
    a.gs += b.gs;
    a.bf += b.bf;
    a.h += b.h;
    a.er += b.er;
    a.hr += b.hr;
    a.bb += b.bb;
    a.ibb += b.ibb;
    a.hbp += b.hbp;
    a.so += b.so;
}

/// Walk-forward protocol: target seasons plus the cut dates (as-of, day-of-year style) per season.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protocol {
    /// Target seasons, ascending (2001..=2025 in v1).
    pub seasons: Vec<u16>,
    /// Cut lines per season as `(as_of_label, ordinal_day_in_year)`: the January 1 preseason cut
    /// plus the May–September month-start in-season cuts. The in-season cuts are scored when the
    /// corpus carries cut lines; otherwise they are skipped (preseason-only behaviour).
    pub cuts: Vec<(String, u16)>,
}

impl Protocol {
    /// The v1 protocol: 2001–2025, cuts Jan 1 + May 1..Sep 1 at month starts.
    pub fn v1() -> Protocol {
        let cuts = [("01-01", 1u16), ("05-01", 121), ("06-01", 152), ("07-01", 182), ("08-01", 213), ("09-01", 244)]
            .into_iter()
            .map(|(label, ord)| (label.to_string(), ord))
            .collect();
        Protocol { seasons: (2001..=2025).collect(), cuts }
    }

    /// Stable hash over the canonical serialisation (seasons; cut labels in order).
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        for s in &self.seasons {
            h.update(s.to_le_bytes());
        }
        for (label, ord) in &self.cuts {
            h.update(label.as_bytes());
            h.update(ord.to_le_bytes());
        }
        h.finalize().iter().map(|x| format!("{x:02x}")).collect()
    }
}

// --- minimal CSV reader (BOM-stripping, CRLF-tolerant, no quoted commas in Lahman samples) ---

fn csv_records(raw: &[u8]) -> std::io::Result<Vec<Vec<String>>> {
    let text = std::str::from_utf8(raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = body.lines();
    let header = lines.next().ok_or_else(|| invalid("empty csv"))?;
    let cols: Vec<&str> = header.split(',').collect();
    let mut out = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let rec: Vec<&str> = line.split(',').collect();
        if rec.len() != cols.len() {
            return Err(invalid(&format!("column count {} != {}", rec.len(), cols.len())));
        }
        out.push(rec.iter().map(|s| s.trim().to_string()).collect());
    }
    Ok(out)
}

fn invalid(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

fn parse_u32(s: &str, what: &str) -> u32 {
    s.trim().parse::<u32>().unwrap_or_else(|_| missing_u32(s, what))
}

fn missing_u32(s: &str, what: &str) -> u32 {
    if s.trim().is_empty() { 0 } else { panic!("corpus: bad u32 for {what}: {s:?}") }
}

fn parse_u16(s: &str, what: &str) -> u16 {
    s.trim().parse::<u16>().unwrap_or_else(|_| missing_u16(s, what))
}

fn missing_u16(s: &str, what: &str) -> u16 {
    if s.trim().is_empty() { 0 } else { panic!("corpus: bad u16 for {what}: {s:?}") }
}

fn parse_u8(s: &str, what: &str) -> u8 {
    s.trim().parse::<u8>().unwrap_or_else(|_| missing_u8(s, what))
}

fn missing_u8(s: &str, what: &str) -> u8 {
    if s.trim().is_empty() { 0 } else { panic!("corpus: bad u8 for {what}: {s:?}") }
}
