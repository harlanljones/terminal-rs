//! Savant Statcast search CSV normaliser (spec 03 §4.6; findings B2, A-F2).
//!
//! A Savant export is a per player per season CSV (both roles) whose header names drift
//! between exports. We therefore never parse by column position: every header cell is
//! matched by NAME against a pinned allowlist, the UTF-8 BOM (if present) is stripped from
//! the header's first cell, and a header whose required allowlist names are absent is
//! rejected with a [`SavantError::Schema`] naming the diff plus rename hints for near-miss
//! names. Extra or unknown columns (miss_distance-class backfills, inserted columns,
//! deprecated names) are accepted by name and simply never read.
//!
//! Sequencing: `game_pk`+`at_bat_number`+`pitch_number` is unique and stable across
//! refetches, but CSV row order is NOT pitch order (A-F2), so sequence numbers
//! (`Event::seq`, dense 1..n) are assigned by sorting on (game_date, game_pk, at_bat_number,
//! pitch_number) — never by file position. Duplicate triples are a misconfiguration and are
//! rejected with an error naming how many were seen.
//!
//! Unit-flag rules (spec §4.6, keep-it-simple):
//! - every consumed row is a pitch event and flags `PITCH`;
//! - the first pitch row of each new `(game_pk, at_bat_number)` closes the previous PA: a bat
//!   log flags `PA | AB` on it, a pit log flags `BF` (a batter faced);
//! - `BIP` is flagged when `bb_type` is non-empty (a batted ball was hit), on both roles.
//!
//! The `type`/`events` columns were rejected as a BIP source: their classification strings
//! churn between Savant releases, while `bb_type` directly answers "ball in play".
//!
//! Deprecated columns (spin_dir, break_*, tfs_*, umpire) are accepted in headers but are
//! never mapped to fields or metrics; `spin_rate_deprecated` is a header alias of
//! `release_spin_rate` (the live name wins when both are present); the vz0/az-class
//! kinematics columns are accepted as unknown extras, stored raw in the table, and unused.

use std::collections::{HashMap, HashSet};

use sabr_model::Date;
use sabr_model::event::{Event, EventLog, SourceRev, UnitFlags, field};
use time::Month;

// ---------------------------------------------------------------------------
// Allowlist
// ---------------------------------------------------------------------------

/// Numeric header names in the pinned allowlist (without aliases). All are required.
pub const NUMERIC: &[&str] = &[
    "plate_x",
    "plate_z",
    "zone",
    "balls",
    "strikes",
    "game_year",
    "on_3b",
    "on_2b",
    "on_1b",
    "outs_when_up",
    "inning",
    "hc_x",
    "hc_y",
    "sz_top",
    "sz_bot",
    "hit_distance_sc",
    "launch_speed",
    "launch_angle",
    "effective_speed",
    "release_speed",
    "release_spin_rate",
    "release_extension",
    "game_pk",
    "fielder_2",
    "fielder_3",
    "fielder_4",
    "fielder_5",
    "fielder_6",
    "fielder_7",
    "fielder_8",
    "fielder_9",
    "woba_value",
    "woba_denom",
    "estimated_woba_using_speedangle",
    "estimated_ba_using_speedangle",
    "at_bat_number",
    "pitch_number",
    "bat_speed",
    "swing_length",
    "miss_distance",
    "arm_angle",
    "attack_angle",
    "attack_direction",
    "swing_path_tilt",
    "intercept_ball_minus_batter_pos_x_inches",
    "intercept_ball_minus_batter_pos_y_inches",
];

/// Text header names in the pinned allowlist (values kept raw for flags/identity.
/// `at_bat_number`/`pitch_number`/`batter`/`pitcher` are numeric but live here too because
/// their parse target is an id, not a metric field).
pub const TEXTUAL: &[&str] = &[
    "pitch_type",
    "game_date",
    "player_name",
    "batter",
    "pitcher",
    "events",
    "description",
    "stand",
    "p_throws",
    "type",
    "bb_type",
    "inning_topbot",
];

/// All required allowlist names (NUMERIC ∪ TEXTUAL).
pub fn required_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = NUMERIC.to_vec();
    v.extend(TEXTUAL.iter().copied());
    v
}

/// Deprecated header names: tolerated, read, never exposed as metrics.
pub const DEPRECATED: &[&str] = &[
    "spin_dir",
    "break_angle_deprecated",
    "break_length_deprecated",
    "tfs_deprecated",
    "tfs_zulu_deprecated",
    "umpire",
];

/// Alias: header alias -> canonical required name.
fn alias(name: &str) -> Option<&'static str> {
    match name {
        "spin_rate_deprecated" => Some("release_spin_rate"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Header-schema mismatch. `missing` names the required allowlist columns that are absent;
/// `renamed_hint` lists payload headers that look like renames of a missing column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub missing: Vec<String>,
    pub renamed_hint: Vec<String>,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "savant schema mismatch: missing columns {:?}; possible renamed headers {:?}",
            self.missing, self.renamed_hint
        )
    }
}

impl std::error::Error for SchemaError {}

/// Savant normaliser errors: schema drift, bad cells, duplicate sequence triples.
#[derive(Debug, thiserror::Error)]
pub enum SavantError {
    #[error("{0}")]
    Schema(#[from] SchemaError),
    /// Misconfiguration: a non-empty numeric/id/date cell that cannot be parsed.
    #[error("savant column '{column}' (data row {row}): '{cell}' is not a valid {kind}")]
    BadCell { column: String, cell: String, row: usize, kind: &'static str },
    /// A-F2 tagging must be unique; reject payloads repeating the triple, naming the count.
    #[error("savant payload has {count} duplicate (game_pk, at_bat_number, pitch_number) triples")]
    DuplicateTriples { count: usize },
}

// ---------------------------------------------------------------------------
// Table (parsed raw CSV, names -> cells)
// ---------------------------------------------------------------------------

/// Parsed Savant CSV: header names plus raw (unquoted) row cells, looked up by name.
#[derive(Debug, Clone)]
pub struct SavantTable {
    pub names: Vec<String>,
    pub rows: Vec<Vec<String>>,
    index: HashMap<String, usize>,
}

impl SavantTable {
    /// Header index of a name, or `None` if the payload lacks it (unknown extras never error here).
    pub fn col(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// Raw cell text (`""` when the column or the cell is empty).
    pub fn cell(&self, row: usize, name: &str) -> &str {
        match (self.col(name), self.rows.get(row)) {
            (Some(c), Some(r)) if c < r.len() => r[c].as_str(),
            _ => "",
        }
    }

    /// Parsed number: empty cell -> `None`; non-empty unparseable -> misconfiguration error.
    pub fn num(&self, row: usize, name: &str) -> Result<Option<f64>, SavantError> {
        match self.cell(row, name).trim() {
            "" => Ok(None),
            s => s.parse::<f64>().map(Some).map_err(|_| self.bad_cell(row, name, s, "number")),
        }
    }

    /// Parsed u32 id (mlbam ids, game pk, at-bat/pitch counters). Empty is an error: these
    /// columns carry the sequence/identity contract.
    pub fn id(&self, row: usize, name: &str) -> Result<u32, SavantError> {
        match self.cell(row, name).trim() {
            "" => Err(SavantError::BadCell { column: name.into(), cell: String::new(), row: row + 2, kind: "id" }),
            s => s.parse::<u32>().map_err(|_| self.bad_cell(row, name, s, "id")),
        }
    }

    /// `YYYY-MM-DD` civil date (game_date).
    pub fn date(&self, row: usize, name: &str) -> Result<Date, SavantError> {
        let s = self.cell(row, name).trim();
        let bad =
            |cell: &str| SavantError::BadCell { column: name.into(), cell: cell.into(), row: row + 2, kind: "date" };
        let ymd: Vec<&str> = s.splitn(3, '-').collect();
        if ymd.len() != 3 {
            return Err(bad(s));
        }
        let y: i32 = ymd[0].parse().map_err(|_| bad(s))?;
        let m: u8 = ymd[1].parse().map_err(|_| bad(s))?;
        let d: u8 = ymd[2].parse().map_err(|_| bad(s))?;
        time::Date::from_calendar_date(y, Month::try_from(m).map_err(|_| bad(s))?, d).map_err(|_| bad(s))
    }

    /// Season = `game_year` of the first data row (a player-season export carries one year).
    pub fn season(&self, tracked: u32, id_col: &str) -> Result<u16, SavantError> {
        (0..self.rows.len())
            .find(|&i| self.cell(i, id_col).trim() == tracked.to_string())
            .map(|i| self.id(i, "game_year").map(|y| y as u16))
            .unwrap_or(Err(SavantError::BadCell {
                column: id_col.into(),
                cell: tracked.to_string(),
                row: 1,
                kind: "tracked player",
            }))
    }

    fn bad_cell(&self, row: usize, name: &str, cell: &str, kind: &'static str) -> SavantError {
        SavantError::BadCell { column: name.into(), cell: cell.into(), row: row + 2, kind }
    }
}

/// Strips a UTF-8 BOM from the start of the payload.
fn strip_bom(raw: &str) -> &str {
    raw.strip_prefix('\u{feff}').unwrap_or(raw)
}

/// RFC-4180-ish row parser: quoted fields, `""` escapes, embedded commas and newlines stay
/// inside quoted cells; stripped BOM; final unterminated record is kept.
fn csv_records(raw: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut chars = strip_bom(raw).chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cell.push(c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                ',' => row.push(std::mem::take(&mut cell)),
                '\n' => {
                    row.push(std::mem::take(&mut cell));
                    out.push(std::mem::take(&mut row));
                }
                '\r' => {}
                _ => cell.push(c),
            }
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        out.push(row);
    }
    out
}

/// Parses a Savant payload into a [`SavantTable`] and validates the header against the
/// pinned allowlist BY NAME, never by position. The BOM is stripped from the header's first
/// cell. Unknown extras (miss_distance-class backfills, inserted columns) parse by name too.
pub fn parse_table(raw: &str) -> Result<SavantTable, SavantError> {
    let mut recs = csv_records(raw);
    recs.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    let mut it = recs.into_iter();
    let names: Vec<String> = it.next().ok_or_else(|| {
        SavantError::Schema(SchemaError {
            missing: required_names().into_iter().map(Into::into).collect(),
            renamed_hint: vec![],
        })
    })?;
    let names: Vec<String> = names.into_iter().map(|n| n.trim().to_owned()).collect();
    let index: HashMap<String, usize> = names.iter().cloned().enumerate().map(|(i, n)| (n, i)).collect();

    // Missing required names -> schema error naming the diff, with rename hints.
    let missing: Vec<&'static str> = required_names().into_iter().filter(|n| !index.contains_key(*n)).collect();
    if !missing.is_empty() {
        let tolerated: HashSet<&str> = required_names().into_iter().chain(DEPRECATED.iter().copied()).collect();
        let mut renamed_hint: Vec<String> = Vec::new();
        for absent in &missing {
            for present in &names {
                if tolerated.contains(present.as_str()) {
                    continue; // a known/tolerated column is not a rename candidate
                }
                if similar(absent, present) && !renamed_hint.iter().any(|h| h == present) {
                    renamed_hint.push(present.clone());
                }
            }
        }
        return Err(SchemaError { missing: missing.into_iter().map(Into::into).collect(), renamed_hint }.into());
    }

    let rows: Vec<Vec<String>> = it.collect();
    Ok(SavantTable { names, rows, index })
}

/// Near-miss test for rename hints: edit distance <= 2.
fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if (a.len() as i64 - b.len() as i64).abs() > 2 {
        return false;
    }
    levenshtein(a, b) <= 2
}

fn levenshtein(a: &[u8], b: &[u8]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = if ca == cb { prev[j] } else { 1 + prev[j].min(prev[j + 1]).min(cur[j]) };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

// ---------------------------------------------------------------------------
// Event log construction
// ---------------------------------------------------------------------------

/// Which role the log tracks: identity column and the first-pitch unit flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavantMode {
    /// Batter log: identity column `batter`, first-pitch flag `PA | AB`.
    Bat,
    /// Pitcher log: identity column `pitcher`, first-pitch flag `BF` (batter faced).
    Pitch,
}

/// Numeric fields exposed on [`Event::fields`], in fixed internal order; the model's pinned
/// field indices are attached where they exist. Columns parsed but never exposed (plate_x,
/// fielder_2..9, hc_x/hc_y, ...) live in [`NUMERIC`] without a pin.
const MODEL_PINS: &[(u16, u16)] = &[
    (0, field::LAUNCH_SPEED),
    (1, field::LAUNCH_ANGLE),
    (2, field::WOBA_VALUE),
    (3, field::ESTIMATED_WOBA),
    (4, field::BAT_SPEED),
    (5, field::SWING_LENGTH),
    (6, field::ATTACK_ANGLE),
    (7, field::ATTACK_DIRECTION),
    (8, field::SWING_PATH_TILT),
    (9, field::RELEASE_SPEED),
    (10, field::SPIN_RATE),
    (11, field::EXTENSION),
];

/// Header name backing each internal id in [`MODEL_PINS`] (`spin_rate` alias-resolved later).
const MODEL_PIN_NAMES: &[&str] = &[
    "launch_speed",
    "launch_angle",
    "woba_value",
    "estimated_woba_using_speedangle",
    "bat_speed",
    "swing_length",
    "attack_angle",
    "attack_direction",
    "swing_path_tilt",
    "release_speed",
    "release_spin_rate", // alias: spin_rate_deprecated only when this is absent
    "release_extension",
];

/// Builds an [`EventLog`] from a parsed table. `tracked` is the mlbam id of the player whose
/// log this is; rows of the opposite role (only possible in a mixed export) are skipped.
/// Flags follow the rules in the module docs; sequence numbers are dense 1..n sorted
/// pitch-order keys; duplicate `(game_pk, at_bat_number, pitch_number)` triples reject the
/// whole payload.
pub fn build_log(table: &SavantTable, mode: SavantMode, tracked: u32, rev: SourceRev) -> Result<EventLog, SavantError> {
    let id_col = match mode {
        SavantMode::Bat => "batter",
        SavantMode::Pitch => "pitcher",
    };
    let entity_key = match mode {
        SavantMode::Bat => format!("p:{tracked}:bat"),
        SavantMode::Pitch => format!("p:{tracked}:pit"),
    };

    // Collect this player's pitch rows with their sort keys; A-F2 says CSV row order is not
    // pitch order, so we sort on (game_date, game_pk, at_bat_number, pitch_number) and never
    // consult file position.
    let mut keyed: Vec<(Date, u32, u32, u32, usize)> = Vec::new();
    for i in 0..table.rows.len() {
        if table.id(i, id_col)? != tracked {
            continue;
        }
        keyed.push((
            table.date(i, "game_date")?,
            table.id(i, "game_pk")?,
            table.id(i, "at_bat_number")?,
            table.id(i, "pitch_number")?,
            i,
        ));
    }
    keyed.sort();

    // Duplicate triples: exact (game_pk, at_bat_number, pitch_number) repeats are a
    // misconfiguration. Reject the payload naming the count.
    let mut seen: HashSet<(u32, u32, u32)> = HashSet::new();
    let mut dup_triples = 0usize;
    for (_, game_pk, ab, pitch_num, _) in &keyed {
        if !seen.insert((*game_pk, *ab, *pitch_num)) {
            dup_triples += 1;
        }
    }
    if dup_triples > 0 {
        return Err(SavantError::DuplicateTriples { count: dup_triples });
    }

    let mut events: Vec<Event> = Vec::with_capacity(keyed.len());
    let mut prev: Option<(u32, u32)> = None; // (game_pk, at_bat_number)
    for (date, game_pk, at_bat, _pitch_num, i) in &keyed {
        let first_of_pa = prev.is_none_or(|(pk, ab)| pk != *game_pk || ab != *at_bat);
        prev = Some((*game_pk, *at_bat));

        let mut flags = UnitFlags(UnitFlags::PITCH);
        if first_of_pa {
            flags.0 |= match mode {
                SavantMode::Bat => UnitFlags::PA | UnitFlags::AB,
                SavantMode::Pitch => UnitFlags::BF,
            };
        }
        if !table.cell(*i, "bb_type").trim().is_empty() {
            flags.0 |= UnitFlags::BIP;
        }

        // Pinned numeric fields: undefined = absent (empty cell), never zero, never NaN.
        let mut fields: Vec<(u16, Option<f64>)> = Vec::new();
        for (internal, pin) in MODEL_PINS {
            let name = MODEL_PIN_NAMES[*internal as usize];
            let v = if name == "release_spin_rate" {
                table.num(*i, "release_spin_rate")?.or(table.num(*i, alias("spin_rate_deprecated").unwrap())?)
            } else {
                table.num(*i, name)?
            };
            if let Some(v) = v
                && v.is_finite()
            {
                fields.push((*pin, Some(v)));
            }
        }
        events.push(Event { seq: events.len() as u64 + 1, game_pk: *game_pk, date: *date, unit_flags: flags, fields });
    }

    Ok(EventLog { entity_key, season: table.season(tracked, id_col)?, source_rev: rev, events })
}

/// Parses a Savant payload and builds the player's [`EventLog`] in one step.
pub fn parse_log(raw: &str, mode: SavantMode, tracked: u32, rev: SourceRev) -> Result<EventLog, SavantError> {
    build_log(&parse_table(raw)?, mode, tracked, rev)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sabr_model::UnitFlags;
    use sabr_model::event::field;

    fn dummy_rev() -> SourceRev {
        SourceRev { source: "savant", url: "test://fixture".into(), fetched: 0, sha256: [0; 32] }
    }

    fn fixture(name: &str) -> String {
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        std::fs::read_to_string(format!("{root}/../../sabr-data/sample/{name}")).expect("fixture readable")
    }

    /// Synthetic CSV over the full pinned allowlist (required names + deprecated), one row.
    fn synthetic_row_spec(overrides: &[(&str, &str)]) -> (String, String) {
        let mut names: Vec<&str> = required_names();
        names.extend(DEPRECATED.iter().copied());
        let mut cells: Vec<String> = vec![String::new(); names.len()];
        let defaults: &[(&str, &str)] = &[
            ("pitch_type", "FF"),
            ("game_date", "2025-09-28"),
            ("release_speed", "94.6"),
            ("player_name", "Ohtani, Shohei"),
            ("batter", "660271"),
            ("pitcher", "501962"),
            ("events", "strikeout"),
            ("description", "swinging_strike"),
            ("zone", "12"),
            ("stand", "R"),
            ("p_throws", "L"),
            ("type", "S"),
            ("bb_type", ""),
            ("balls", "0"),
            ("strikes", "2"),
            ("game_year", "2025"),
            ("inning", "3"),
            ("inning_topbot", "Top"),
            ("outs_when_up", "1"),
            ("game_pk", "777288"),
            ("at_bat_number", "50"),
            ("pitch_number", "6"),
            ("release_spin_rate", "2381"),
        ];
        let set = |map: &mut Vec<String>, name: &str, val: &str| {
            let i =
                required_names().iter().chain(DEPRECATED.iter()).position(|n| n.eq_ignore_ascii_case(name)).unwrap();
            map[i] = val.into();
        };
        for (n, v) in defaults.iter().chain(overrides) {
            set(&mut cells, n, v);
        }
        let header: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        let row = names
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let c = &cells[i]; // RFC-4180: quote cells containing commas or quotes
                let esc = c.replace('"', "\"\"");
                if c.contains(',') || c.contains('"') { format!("\"{esc}\"") } else { c.clone() }
            })
            .collect::<Vec<String>>();
        (header.join(","), row.join(","))
    }

    fn synthetic_csv(rows: &[(&str, &str)]) -> String {
        let (header, row0) = synthetic_row_spec(rows);
        format!("{header}\n{row0}\n")
    }

    #[test]
    fn fixtures_parse_and_count() {
        let bat = fixture("ohtani-2025-bat.csv");
        let log = parse_log(&bat, SavantMode::Bat, 660271, dummy_rev()).expect("bat fixture parses");
        assert_eq!(log.events.len(), 2940, "bat events");
        assert_eq!(log.season, 2025);
        assert_eq!(log.entity_key, "p:660271:bat");
        assert!(log.events.windows(2).all(|w| w[0].seq + 1 == w[1].seq), "seq dense ascending");
        assert!(log.events.iter().all(|e| e.unit_flags.has(UnitFlags::PITCH)));
        let pa_events = log.events.iter().filter(|e| e.unit_flags.has(UnitFlags::PA)).count();
        assert_eq!(pa_events, 727, "bat PA count from the fixture");
        let bip = log.events.iter().filter(|e| e.unit_flags.has(UnitFlags::BIP)).count();
        assert_eq!(bip, 426, "bat BIP = non-empty bb_type rows");

        let pit = fixture("ohtani-2025-pit.csv");
        let log = parse_log(&pit, SavantMode::Pitch, 660271, dummy_rev()).expect("pit fixture parses");
        assert_eq!(log.events.len(), 753, "pit events");
        assert!(log.events.windows(2).all(|w| w[0].seq + 1 == w[1].seq), "seq dense ascending");
        let bf = log.events.iter().filter(|e| e.unit_flags.has(UnitFlags::BF)).count();
        assert_eq!(bf, 188, "pit batter-faced count from the fixture");
    }

    #[test]
    fn row_order_shuffle_yields_same_seq() {
        let bat = fixture("ohtani-2025-bat.csv");
        let expected = parse_log(&bat, SavantMode::Bat, 660271, dummy_rev()).expect("parses");
        // reverse every data line (never the header):
        let mut lines: Vec<String> = bat.lines().map(Into::into).collect();
        let header = lines.remove(0);
        lines.reverse();
        let shuffled = format!("{header}\n{}\n", lines.join("\n"));
        let actual = parse_log(&shuffled, SavantMode::Bat, 660271, dummy_rev()).expect("shuffled parses");
        assert_eq!(expected.events, actual.events, "seq assignment is file-position free");
    }

    #[test]
    fn unknown_header_rejection_names_the_column() {
        let csv = synthetic_csv(&[]);
        let csv = csv.replacen(",zone,", ",", 1);
        let err = parse_table(&csv).expect_err("missing zone rejects");
        match err {
            SavantError::Schema(SchemaError { missing, renamed_hint }) => {
                assert!(missing.contains(&"zone".to_string()), "{missing:?}");
                assert!(renamed_hint.is_empty());
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn renamed_required_column_fails_with_hint() {
        let csv = synthetic_csv(&[]);
        let csv = csv.replacen(",woba_value,", ",woba_valu,", 1);
        let err = parse_table(&csv).expect_err("renamed column rejects");
        match err {
            SavantError::Schema(SchemaError { missing, renamed_hint }) => {
                assert!(missing.contains(&"woba_value".to_string()));
                assert!(renamed_hint.contains(&"woba_valu".to_string()), "{renamed_hint:?}");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn inserted_column_still_parses_by_name() {
        let csv = synthetic_csv(&[("bb_type", "line_drive")]);
        let csv = csv.replacen(",player_name,", ",player_name,extra_tmp_col,", 1);
        let csv = csv.replacen("\"Ohtani, Shohei\",", "\"Ohtani, Shohei\",7,", 1);
        let table = parse_table(&csv).expect("inserted column tolerated");
        let log = build_log(&table, SavantMode::Bat, 660271, dummy_rev()).expect("log builds");
        assert_eq!(log.events.len(), 1);
        assert!(log.events[0].unit_flags.has(UnitFlags::BIP), "bb_type non-empty flags BIP");
    }

    #[test]
    fn empty_cell_is_none_not_zero() {
        let csv = synthetic_csv(&[("launch_speed", ""), ("launch_angle", "")]);
        let log = parse_log(&csv, SavantMode::Bat, 660271, dummy_rev()).expect("parses");
        let e = &log.events[0];
        assert!(e.fields.iter().all(|(i, _)| *i != field::LAUNCH_SPEED && *i != field::LAUNCH_ANGLE));
        assert_eq!(e.field(field::LAUNCH_SPEED), None);
        assert_eq!(e.field(field::RELEASE_SPEED), Some(94.6));
    }

    #[test]
    fn bom_stripped_from_header() {
        let base = synthetic_csv(&[]);
        let (header, rest) = base.split_once('\n').unwrap();
        let bommed = format!("\u{feff}{header}\n{rest}");
        let table = parse_table(&bommed).expect("BOM stripped");
        assert!(table.col("pitch_type").is_some(), "first header cell usable");
        assert_eq!(table.cell(0, "pitch_type"), "FF");
    }

    #[test]
    fn numeric_garbage_is_an_error() {
        let csv = synthetic_csv(&[("launch_speed", "abc")]);
        match parse_log(&csv, SavantMode::Bat, 660271, dummy_rev()) {
            Err(SavantError::BadCell { column, cell, .. }) => {
                assert_eq!(column, "launch_speed");
                assert_eq!(cell, "abc");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn duplicate_triples_rejected_naming_count() {
        let (header, row) = synthetic_row_spec(&[]);
        let csv = format!("{header}\n{row}\n{row}\n"); // same (game_pk, at_bat_number, pitch_number) twice
        match parse_log(&csv, SavantMode::Pitch, 501962, dummy_rev()) {
            Err(SavantError::DuplicateTriples { count }) => assert_eq!(count, 1),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn spin_rate_alias_live_name_wins() {
        // both present: the live name wins over `spin_rate_deprecated`.
        let (h, r) = synthetic_row_spec(&[]);
        let h = h.replacen(",release_spin_rate,", ",release_spin_rate,spin_rate_deprecated,", 1);
        let r = r.replacen(",2381,", ",2381,1,", 1);
        let csv = format!("{h}\n{r}");
        let table = parse_table(&csv).expect("parsed");
        let log = build_log(&table, SavantMode::Bat, 660271, dummy_rev()).expect("built");
        assert_eq!(log.events[0].field(field::SPIN_RATE), Some(2381.0), "live name wins over alias");
    }

    #[test]
    fn alias_only_header_is_still_a_required_column() {
        // `spin_rate_deprecated` alone does NOT satisfy the schema: `release_spin_rate` is
        // required and deprecated names are tolerated, never substituted.
        let (h, r) = synthetic_row_spec(&[]);
        let h = h.replacen(",release_spin_rate,", ",spin_rate_deprecated,", 1);
        let r = r.replacen(",2381,", ",1500,", 1);
        assert!(matches!(parse_table(&format!("{h}\n{r}")), Err(SavantError::Schema(_))));
    }

    #[test]
    fn flag_rules_documented_in_the_module_doc() {
        // syntactic guard: referenced rule fns exist and are used by build_log
        let csv = synthetic_csv(&[]);
        let log = parse_log(&csv, SavantMode::Bat, 660271, dummy_rev()).expect("built");
        let e = &log.events[0];
        assert!(e.unit_flags.has(UnitFlags::PITCH));
        assert!(e.unit_flags.has(UnitFlags::PA) && e.unit_flags.has(UnitFlags::AB));
        assert!(!e.unit_flags.has(UnitFlags::BIP), "empty bb_type is not a batted ball");
    }
}
