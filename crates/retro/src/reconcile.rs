//! Reconciliation of parsed lines against Lahman official totals (spec 4.5).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::event::{BatLine, Game, PitLine};

/// Exact-match rate for one stat across compared player-seasons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatRate {
    pub stat: String,
    pub compared: usize,
    pub exact: usize,
    pub rate: f64,
}

/// One mismatch worth surfacing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discrepancy {
    pub player_id: String,
    pub season: i16,
    pub stat: String,
    pub parsed: i64,
    pub lahman: i64,
}

/// Which side a player-season row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Batting,
    Pitching,
}

/// Reconciliation report for one side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub side: Side,
    pub player_seasons: usize,
    /// Exact-match rate per stat.
    pub stats: Vec<StatRate>,
    /// Worst mismatches per stat (up to 3 each).
    pub worst: Vec<Discrepancy>,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "reconciliation ({:?}): {} player-seasons", self.side, self.player_seasons)?;
        for s in &self.stats {
            writeln!(f, "  {:>5}: {}/{} exact ({:.2}%)", s.stat, s.exact, s.compared, 100.0 * s.rate)?;
        }
        for d in &self.worst {
            writeln!(f, "  worst {} {} {}: parsed {} vs lahman {}", d.stat, d.season, d.player_id, d.parsed, d.lahman)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Agg {
    bat: BatLine,
    pit: PitLine,
    pa: u32,
    teams: BTreeSet<String>,
}

fn parsed_totals(games: &[Game]) -> BTreeMap<(String, i16), Agg> {
    let mut m: BTreeMap<(String, i16), Agg> = BTreeMap::new();
    for g in games {
        let Some(d) = g.date else { continue };
        let y = d.year();
        for (pid, l) in &g.batting {
            let a = m.entry((pid.clone(), y)).or_default();
            a.bat.add(l);
            a.teams.extend(g.player_teams.get(pid).cloned().unwrap_or_default());
        }
        for (pid, l) in &g.pitching {
            let a = m.entry((pid.clone(), y)).or_default();
            a.pit.add(l);
            a.teams.extend(g.player_teams.get(pid).cloned().unwrap_or_default());
        }
    }
    for a in m.values_mut() {
        // Interference PAs are neither AB, BB, HBP, SH nor SF; Lahman has no
        // column for them, so the comparable PA total excludes them.
        a.pa = a.bat.ab + a.bat.bb + a.bat.hbp + a.bat.sh + a.bat.sf;
    }
    m
}

/// Read a Lahman CSV into ((playerID, yearID) -> col -> value).
fn read_lahman(path: &Path) -> std::io::Result<BTreeMap<(String, i16), BTreeMap<String, i64>>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<String> =
        lines.next().unwrap_or_default().trim_start_matches('\u{feff}').split(',').map(str::to_string).collect();
    let mut out: BTreeMap<(String, i16), BTreeMap<String, i64>> = BTreeMap::new();
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < header.len() {
            continue;
        }
        let get = |name: &str| header.iter().position(|h| h == name).map(|i| f[i]);
        let (Some(pid), Some(year)) = (get("playerID"), get("yearID")) else { continue };
        let Ok(y) = year.parse::<i16>() else { continue };
        let row = out.entry((pid.to_string(), y)).or_default();
        for (i, h) in header.iter().enumerate() {
            if matches!(h.as_str(), "playerID" | "yearID" | "stint" | "lgID" | "teamID" | "BAOpp" | "ERA") {
                continue;
            }
            if let Ok(v) = f[i].parse::<i64>() {
                *row.entry(h.clone()).or_insert(0) += v;
            }
        }
    }
    Ok(out)
}

const BAT_STATS: &[&str] =
    &["pa", "ab", "h", "2b", "3b", "hr", "rbi", "r", "bb", "so", "ibb", "hbp", "sh", "sf", "sb", "cs"];
const PIT_STATS: &[&str] = &["bf", "outs", "h", "hr", "bb", "ibb", "hbp", "so", "r", "er"];

/// Lahman CSV column for a stat code.
fn lahman_col(stat: &str) -> String {
    match stat {
        "2b" => "2B".to_string(),
        "3b" => "3B".to_string(),
        "bf" => "BFP".to_string(),
        "outs" => "IPouts".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

/// Compare parsed season totals against Lahman per player-season. Only rows
/// present in the parsed corpus are compared (a partial corpus yields partial
/// lines and that is visible in the rates). `teams` restricts comparison to
/// players who appeared for one of the given team codes; empty compares all.
pub fn reconcile_teams(games: &[Game], lahman_dir: &Path, side: Side, teams: &[&str]) -> std::io::Result<Report> {
    let file = match side {
        Side::Batting => "Batting.csv",
        Side::Pitching => "Pitching.csv",
    };
    let mut parsed = parsed_totals(games);
    if !teams.is_empty() {
        parsed.retain(|_, a| teams.iter().any(|t| a.teams.iter().any(|x| x == t)));
    }
    let lahman = read_lahman(&lahman_dir.join(file))?;
    let people = read_people(&lahman_dir.join("People.csv"))?;
    Ok(build_report(&parsed, &lahman, &people, side))
}

/// Reconcile across all parsed player-seasons.
pub fn reconcile(games: &[Game], lahman_dir: &Path, side: Side) -> std::io::Result<Report> {
    reconcile_teams(games, lahman_dir, side, &[])
}

/// Retrosheet id → Lahman playerID, resolved from People.csv `retroID`.
fn read_people(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<String> =
        lines.next().unwrap_or_default().trim_start_matches('\u{feff}').split(',').map(str::to_string).collect();
    let Some(i_pid) = header.iter().position(|h| h == "playerID") else { return Ok(BTreeMap::new()) };
    let Some(i_retro) = header.iter().position(|h| h == "retroID") else { return Ok(BTreeMap::new()) };
    let mut out = BTreeMap::new();
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() <= i_pid || f.len() <= i_retro || f[i_retro].is_empty() {
            continue;
        }
        out.insert(f[i_retro].to_string(), f[i_pid].to_string());
    }
    Ok(out)
}

fn build_report(
    parsed: &BTreeMap<(String, i16), Agg>,
    lahman: &BTreeMap<(String, i16), BTreeMap<String, i64>>,
    people: &BTreeMap<String, String>,
    side: Side,
) -> Report {
    let stats: &[&str] = match side {
        Side::Batting => BAT_STATS,
        Side::Pitching => PIT_STATS,
    };
    let mut rows: BTreeMap<String, (usize, usize, Vec<Discrepancy>)> = BTreeMap::new();
    let mut n = 0usize;
    for ((pid, y), agg) in parsed {
        let key = people.get(pid).cloned().unwrap_or_else(|| pid.clone());
        let Some(l) = lahman.get(&(key, *y)) else { continue };
        let lm_pa = |l: &BTreeMap<String, i64>| {
            l.get("AB").unwrap_or(&0)
                + l.get("BB").unwrap_or(&0)
                + l.get("HBP").unwrap_or(&0)
                + l.get("SH").unwrap_or(&0)
                + l.get("SF").unwrap_or(&0)
        };
        n += 1;
        let vals: Vec<(String, i64, i64)> = stats
            .iter()
            .map(|&s| {
                let p = match (side, s) {
                    (Side::Batting, "pa") => agg.pa as i64,
                    (Side::Batting, "ab") => agg.bat.ab as i64,
                    (Side::Batting, "h") => agg.bat.h as i64,
                    (Side::Batting, "2b") => agg.bat.d as i64,
                    (Side::Batting, "3b") => agg.bat.t as i64,
                    (Side::Batting, "hr") => agg.bat.hr as i64,
                    (Side::Batting, "rbi") => agg.bat.rbi as i64,
                    (Side::Batting, "r") => agg.bat.r as i64,
                    (Side::Batting, "bb") => agg.bat.bb as i64,
                    (Side::Batting, "so") => agg.bat.so as i64,
                    (Side::Batting, "ibb") => agg.bat.ibb as i64,
                    (Side::Batting, "hbp") => agg.bat.hbp as i64,
                    (Side::Batting, "sh") => agg.bat.sh as i64,
                    (Side::Batting, "sf") => agg.bat.sf as i64,
                    (Side::Batting, "sb") => agg.bat.sb as i64,
                    (Side::Batting, "cs") => agg.bat.cs as i64,
                    (Side::Pitching, "bf") => agg.pit.bf as i64,
                    (Side::Pitching, "outs") => agg.pit.outs as i64,
                    (Side::Pitching, "h") => agg.pit.h as i64,
                    (Side::Pitching, "hr") => agg.pit.hr as i64,
                    (Side::Pitching, "bb") => agg.pit.bb as i64,
                    (Side::Pitching, "ibb") => agg.pit.ibb as i64,
                    (Side::Pitching, "hbp") => agg.pit.hbp as i64,
                    (Side::Pitching, "so") => agg.pit.so as i64,
                    (Side::Pitching, "r") => agg.pit.r as i64,
                    (Side::Pitching, "er") => agg.pit.er as i64,
                    _ => 0,
                };
                let lm = if matches!((side, s), (Side::Batting, "pa")) {
                    lm_pa(l)
                } else {
                    l.get(&lahman_col(s)).copied().unwrap_or(0)
                };
                (s.to_string(), p, lm)
            })
            .collect();
        for (s, p, lm) in vals {
            let e = rows.entry(s).or_default();
            e.0 += 1;
            if p == lm {
                e.1 += 1;
            } else {
                e.2.push(Discrepancy {
                    player_id: pid.clone(),
                    season: *y,
                    stat: String::new(),
                    parsed: p,
                    lahman: lm,
                });
            }
        }
    }
    let mut out_stats = Vec::new();
    let mut worst = Vec::new();
    for (s, (compared, exact, mut ds)) in rows {
        ds.sort_by_key(|d| (d.parsed - d.lahman).abs());
        ds.reverse();
        for mut d in ds.into_iter().take(3) {
            d.stat = s.clone();
            worst.push(d);
        }
        let rate = if compared == 0 { 1.0 } else { exact as f64 / compared as f64 };
        out_stats.push(StatRate { stat: s, compared, exact, rate });
    }
    Report { side, player_seasons: n, stats: out_stats, worst }
}
