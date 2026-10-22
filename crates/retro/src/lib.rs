//! # sabr-retro — Retrosheet event parsing, cut lines and reconciliation (spec §4.5)
//!
//! Parses Chadwick-mirror event files (.EVA/.EVN) into per-game batting and
//! pitching lines, computes cumulative cut lines at as-of dates, and
//! reconciles parsed season totals against Lahman official totals. Unknown
//! play tokens fail loudly with the offending token named; nothing is
//! silently dropped.

pub mod cut;
pub mod event;
pub mod play;
pub mod reconcile;

pub use cut::{CutLine, cut_lines, season_cuts, write_cut_lines_csv, write_cut_lines_file};
pub use event::{BatLine, ErSource, EventFile, Game, PitLine, parse_file};
pub use play::{AdvFlag, Advance, Kind, ParseError, Parsed, parse_event};
pub use reconcile::{Discrepancy, Report, Side, StatRate, reconcile, reconcile_teams};

use std::path::Path;

/// Build the retro bundle for a season set from a directory of .EV[AN] files:
/// writes `cut-lines.csv` under `out_dir` and returns the reconciliation
/// reports (batting and pitching) for the caller to print. Downloads nothing:
/// `ev_dir` must already contain the event files, `lahman_dir` the Lahman
/// `Batting.csv`/`Pitching.csv`.
pub fn build_bundle(
    out_dir: &Path,
    ev_dir: &Path,
    lahman_dir: &Path,
    seasons: &[i16],
) -> anyhow::Result<(std::path::PathBuf, Report, Report)> {
    // Collect event files for the requested seasons: YYYY<TEAM>.EV[AN].
    let mut paths = Vec::new();
    let wanted: Vec<i16> = seasons.to_vec();
    for entry in std::fs::read_dir(ev_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let bytes = name.as_bytes();
        // Team codes are 3 or 4 letters (e.g. ARI, BSN), so accept both name lengths.
        let ok = (name.len() == 11 || (name.len() == 12 && bytes[7].is_ascii_uppercase()))
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[4..7].iter().all(u8::is_ascii_uppercase)
            && (name.ends_with(".EVA") || name.ends_with(".EVN"));
        if !ok {
            continue;
        }
        let year: i16 = name[..4].parse()?;
        if wanted.contains(&year) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    anyhow::ensure!(!paths.is_empty(), "no event files for seasons {seasons:?} in {}", ev_dir.display());

    let mut games = Vec::new();
    for path in &paths {
        let file = parse_file(path).map_err(|e| anyhow::anyhow!("{path:?}: {e}"))?;
        games.extend(file.games);
    }
    games.sort_by_key(|g| g.date);

    let cuts = season_cuts(seasons);
    let lines = cut_lines(&games, &cuts);
    std::fs::create_dir_all(out_dir)?;
    let csv_path = out_dir.join("cut-lines.csv");
    write_cut_lines_file(&csv_path, &lines)?;

    let batting = reconcile(&games, lahman_dir, Side::Batting)?;
    let pitching = reconcile(&games, lahman_dir, Side::Pitching)?;
    Ok((csv_path, batting, pitching))
}
