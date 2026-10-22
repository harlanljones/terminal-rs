//! Event-file parser: one .EVA/.EVN file → per-game batting/pitching lines.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::play::{AdvFlag, Advance, Kind, ParseError, Parsed, parse_event};

/// Per-player batting line (per game, or accumulated).
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatLine {
    pub pa: u32,
    pub ab: u32,
    pub h: u32,
    pub d: u32,
    pub t: u32,
    pub hr: u32,
    pub bb: u32,
    pub ibb: u32,
    pub hbp: u32,
    pub sf: u32,
    pub sh: u32,
    pub so: u32,
    pub r: u32,
    pub rbi: u32,
    pub sb: u32,
    pub cs: u32,
}

impl BatLine {
    pub fn add(&mut self, o: &BatLine) {
        self.pa += o.pa;
        self.ab += o.ab;
        self.h += o.h;
        self.d += o.d;
        self.t += o.t;
        self.hr += o.hr;
        self.bb += o.bb;
        self.ibb += o.ibb;
        self.hbp += o.hbp;
        self.sf += o.sf;
        self.sh += o.sh;
        self.so += o.so;
        self.r += o.r;
        self.rbi += o.rbi;
        self.sb += o.sb;
        self.cs += o.cs;
    }
}

/// Per-pitcher line (per game, or accumulated).
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct PitLine {
    pub bf: u32,
    pub outs: u32,
    pub h: u32,
    pub hr: u32,
    pub bb: u32,
    pub ibb: u32,
    pub hbp: u32,
    pub so: u32,
    pub r: u32,
    pub er: u32,
}

impl PitLine {
    pub fn add(&mut self, o: &PitLine) {
        self.bf += o.bf;
        self.outs += o.outs;
        self.h += o.h;
        self.hr += o.hr;
        self.bb += o.bb;
        self.ibb += o.ibb;
        self.hbp += o.hbp;
        self.so += o.so;
        self.r += o.r;
        self.er += o.er;
    }
}

/// Which path supplied earned runs for a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ErSource {
    /// Explicit `data,er` records were present and used.
    #[default]
    DataRecords,
    /// Derived from advancement unearned markers.
    Parsed,
}

/// One parsed game.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub date: Option<Date>,
    /// Team codes (away, home).
    pub teams: [String; 2],
    pub batting: BTreeMap<String, BatLine>,
    pub pitching: BTreeMap<String, PitLine>,
    /// Team codes each player appeared for (batting or pitching).
    pub player_teams: BTreeMap<String, BTreeSet<String>>,
    pub er_source: ErSource,
}

/// A parsed event file (one .EVA/.EVN).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFile {
    pub games: Vec<Game>,
    /// Path the file was parsed from.
    pub source: String,
}

pub fn parse_file(path: &Path) -> Result<EventFile, ParseError> {
    let text = std::fs::read_to_string(path).map_err(|_| ParseError::UnknownToken {
        token: String::new(),
        raw: format!("unreadable file {}", path.display()),
    })?;
    let source = path.display().to_string();
    let mut p = FileParser::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        p.line(line).map_err(|e| annotate(e, lineno, line, &source))?;
    }
    p.flush();
    Ok(EventFile { games: p.games, source })
}

fn annotate(e: ParseError, lineno: usize, line: &str, source: &str) -> ParseError {
    let ctx = format!(" ({source}:{lineno}: `{line}`)");
    match e {
        ParseError::UnknownToken { token, raw } => ParseError::UnknownToken { token: format!("{token}{ctx}"), raw },
        ParseError::BadAdvance { token, raw } => ParseError::BadAdvance { token: format!("{token}{ctx}"), raw },
        ParseError::BadParen { token, raw } => ParseError::BadParen { token: format!("{token}{ctx}"), raw },
    }
}

struct FileParser {
    games: Vec<Game>,
    id: String,
    date: Option<Date>,
    teams: [String; 2],
    batting: BTreeMap<String, BatLine>,
    pitching: BTreeMap<String, PitLine>,
    player_teams: BTreeMap<String, BTreeSet<String>>,
    pitchers: [Option<String>; 2],
    bases: [Option<String>; 3],
    outs: u32,
    er: BTreeMap<String, u32>,
    data_er: bool,
}

impl FileParser {
    fn new() -> Self {
        FileParser {
            games: Vec::new(),
            id: String::new(),
            date: None,
            teams: [String::new(), String::new()],
            batting: BTreeMap::new(),
            pitching: BTreeMap::new(),
            player_teams: BTreeMap::new(),
            pitchers: [None, None],
            bases: [None, None, None],
            outs: 0,
            er: BTreeMap::new(),
            data_er: false,
        }
    }

    fn flush(&mut self) {
        if self.id.is_empty() {
            return;
        }
        let mut pitching = std::mem::take(&mut self.pitching);
        let er_source = if self.data_er {
            for (pid, er) in std::mem::take(&mut self.er) {
                pitching.entry(pid).or_default().er = er;
            }
            ErSource::DataRecords
        } else {
            for (pid, line) in pitching.iter_mut() {
                // Fallback: earned = runs minus runs flagged unearned.
                let unearned = self.er.get(pid).copied().unwrap_or(0);
                line.er = line.r.saturating_sub(unearned);
            }
            ErSource::Parsed
        };
        self.games.push(Game {
            id: std::mem::take(&mut self.id),
            date: self.date.take(),
            teams: std::mem::take(&mut self.teams),
            batting: std::mem::take(&mut self.batting),
            pitching,
            player_teams: std::mem::take(&mut self.player_teams),
            er_source,
        });
        self.bases = [None, None, None];
        self.outs = 0;
        self.pitchers = [None, None];
        self.er.clear();
        self.data_er = false;
    }

    fn line(&mut self, line: &str) -> Result<(), ParseError> {
        let mut f = line.splitn(2, ',');
        let tag = f.next().unwrap_or_default();
        match tag {
            "id" => {
                self.flush();
                self.id = line.strip_prefix("id,").unwrap_or_default().to_string();
            }
            "info" => {
                let Some(rest) = line.strip_prefix("info,") else { return Ok(()) };
                let mut it = rest.splitn(2, ',');
                let key = it.next().unwrap_or_default();
                let val = it.next().unwrap_or_default();
                match key {
                    "date" => {
                        self.date = Some(Date::strptime("%Y/%m/%d", val).map_err(|_| ParseError::UnknownToken {
                            token: format!("date={val}"),
                            raw: line.to_string(),
                        })?);
                    }
                    "visteam" => self.teams[0] = val.to_string(),
                    "hometeam" => self.teams[1] = val.to_string(),
                    _ => {}
                }
            }
            "start" | "sub" => {
                let f: Vec<&str> = line.split(',').collect();
                // start,playerID,name,home,order,position
                if f.len() >= 6 {
                    let pid = f[1].to_string();
                    let home: usize = f[3].parse().unwrap_or(0);
                    let pos: u8 = f[5].parse().unwrap_or(0);
                    if pos == 1 {
                        self.pitchers[home.min(1)] = Some(pid);
                    }
                }
            }
            "play" => self.play_line(line)?,
            "data" => {
                let Some(rest) = line.strip_prefix("data,er,") else { return Ok(()) };
                let mut it = rest.split(',');
                while let (Some(pid), Some(er)) = (it.next(), it.next()) {
                    if let Ok(er) = er.parse::<u32>() {
                        self.er.insert(pid.to_string(), er);
                        self.data_er = true;
                    }
                }
            }
            "radj" => {
                // radj,playerID,base: extra-inning placed runner; not a PA.
                let f: Vec<&str> = line.split(',').collect();
                if f.len() >= 3
                    && let Ok(base) = f[2].parse::<u8>()
                    && (1..=3).contains(&base)
                {
                    self.bases[(base - 1) as usize] = Some(f[1].to_string());
                }
            }
            _ => {} // com, badj, ladj, stop, ... ignored
        }
        Ok(())
    }

    fn play_line(&mut self, line: &str) -> Result<(), ParseError> {
        let rest = line.strip_prefix("play,").unwrap_or_default();
        // play,inning,half,batter,count,pitches,event
        let mut f = rest.splitn(7, ',');
        let _inning = f.next().unwrap_or_default();
        let half: usize = f.next().unwrap_or_default().parse().unwrap_or(0);
        let batter = f.next().unwrap_or_default().to_string();
        let _count = f.next().unwrap_or_default();
        let _pitches = f.next().unwrap_or_default();
        let event = f.next().unwrap_or_default();
        if event == "NP" || event == "radj" || event.is_empty() {
            return Ok(());
        }
        let parsed = parse_event(event)?;
        let batting_team = half.min(1); // 0 top: visteam bats, 1 bottom: home team bats
        let fielding_team = 1 - batting_team;
        self.note_team(&batter, batting_team);
        let fielding_pitcher = self.pitchers[fielding_team].clone();
        if let Some(p) = &fielding_pitcher {
            self.note_team(p, fielding_team);
        }

        if !parsed.kind.is_pa() {
            let ev = self.augment_runner_event(&parsed);
            let mut unearned = 0;
            let outs = self.resolve(&ev, "", &fielding_pitcher, &mut unearned);
            self.commit_outs(outs, &fielding_pitcher, unearned);
            if self.outs >= 3 {
                self.outs = 0;
                self.bases = [None, None, None];
            }
            return Ok(());
        }

        // Plate appearance.
        let bl = self.batting.entry(batter.clone()).or_default();
        bl.pa += 1;
        match parsed.kind {
            Kind::Single => bl.h += 1,
            Kind::Double | Kind::GroundRuleDouble => {
                bl.h += 1;
                bl.d += 1;
            }
            Kind::Triple => {
                bl.h += 1;
                bl.t += 1;
            }
            Kind::Homer => {
                bl.h += 1;
                bl.hr += 1;
            }
            Kind::Walk => bl.bb += 1,
            Kind::IntWalk => {
                bl.bb += 1;
                bl.ibb += 1;
            }
            Kind::Hbp => bl.hbp += 1,
            Kind::Strikeout => bl.so += 1,
            Kind::SacFly => bl.sf += 1,
            Kind::SacHit => bl.sh += 1,
            _ => {}
        }
        let is_ab = !matches!(
            parsed.kind,
            Kind::Walk | Kind::IntWalk | Kind::Hbp | Kind::SacFly | Kind::SacHit | Kind::Interference
        );
        if is_ab {
            bl.ab += 1;
        }
        if let Some(pl) = &fielding_pitcher {
            let pl = self.pitching.entry(pl.clone()).or_default();
            pl.bf += 1;
            match parsed.kind {
                Kind::Single | Kind::Double | Kind::GroundRuleDouble | Kind::Triple => pl.h += 1,
                Kind::Homer => {
                    pl.h += 1;
                    pl.hr += 1;
                }
                Kind::Walk => pl.bb += 1,
                Kind::IntWalk => {
                    pl.bb += 1;
                    pl.ibb += 1;
                }
                Kind::Hbp => pl.hbp += 1,
                Kind::Strikeout => pl.so += 1,
                _ => {}
            }
        }

        // Apply the play: batter movement + runner movements + implicit forces.
        let mut unearned = 0;
        let outs = self.resolve(&parsed, &batter, &fielding_pitcher, &mut unearned);
        self.commit_outs(outs, &fielding_pitcher, unearned);
        // Compound suffix events (K+SB2, W+WP, ...) and listed extras
        // (double steal) act as runner events within the same play.
        let suffixes = parsed.suffix.iter().copied().chain(parsed.extras.iter().copied());
        for s in suffixes {
            let ev = self.augment_runner_event(&Parsed {
                kind: s,
                suffix: None,
                extras: Vec::new(),
                batter: None,
                dest: 0,
                advances: Vec::new(),
                base_error: false,
                dp: false,
                sf: false,
                sh: false,
                iphr: false,
            });
            let mut extra_unearned = 0;
            let extra_outs = self.resolve(&ev, "", &fielding_pitcher, &mut extra_unearned);
            if self.outs + extra_outs >= 3 {
                self.outs = 0;
                self.bases = [None, None, None];
            } else {
                self.commit_outs(extra_outs, &fielding_pitcher, extra_unearned);
            }
        }
        if self.outs >= 3 {
            self.outs = 0;
            self.bases = [None, None, None];
        }
        Ok(())
    }

    fn commit_outs(&mut self, outs: u32, pitcher: &Option<String>, unearned_runs: u32) {
        self.outs += outs;
        if let Some(pid) = pitcher {
            self.pitching.entry(pid.clone()).or_default().outs += outs;
            if unearned_runs > 0 {
                *self.er.entry(pid.clone()).or_insert(0) += unearned_runs;
            }
        }
    }

    fn note_team(&mut self, pid: &str, side: usize) {
        if self.teams[side].is_empty() {
            return;
        }
        self.player_teams.entry(pid.to_string()).or_default().insert(self.teams[side].clone());
    }

    /// Fill in implicit movements for a non-PA runner event.
    fn augment_runner_event(&self, parsed: &Parsed) -> Parsed {
        let mut ev = parsed.clone();
        match parsed.kind {
            Kind::WildPitch | Kind::PassedBall | Kind::Balk | Kind::Indifference => {
                // Advance every runner one base unless explicit.
                if ev.advances.is_empty() {
                    for b in (1..=3).rev() {
                        if self.bases[b - 1].is_some() {
                            let mut flags = Vec::new();
                            if matches!(ev.kind, Kind::WildPitch | Kind::PassedBall | Kind::Balk) {
                                flags.push(AdvFlag::Wp);
                            }
                            ev.advances.push(Advance { from: b as u8, to: b as u8 + 1, out: false, flags });
                        }
                    }
                }
            }
            Kind::Steal(n) | Kind::CaughtStealing(n) | Kind::PickoffCs(n) if (2..=4).contains(&n) => {
                // SB2/CS2: runner steals/tries to steal base n from base n-1.
                if !ev.advances.iter().any(|a| a.from == n - 1) {
                    let out = !matches!(ev.kind, Kind::Steal(_)) && !ev.base_error;
                    ev.advances.push(Advance { from: n - 1, to: n, out, flags: Vec::new() });
                }
            }
            Kind::Pickoff(n)
                if (1..=3).contains(&n)
                // PO2: the runner ON base n is picked off (out or error).
                && !ev.advances.iter().any(|a| a.from == n) =>
            {
                let out = !ev.base_error;
                ev.advances.push(Advance { from: n, to: n, out, flags: Vec::new() });
            }
            _ => {}
        }
        ev
    }

    /// Apply batter and runner movements; credit runs. Returns outs made on
    /// the play. `batter` empty for pure runner events.
    fn resolve(&mut self, parsed: &Parsed, batter: &str, pitcher: &Option<String>, runs_unearned: &mut u32) -> u32 {
        let mut adv_by_from: BTreeMap<u8, &Advance> = BTreeMap::new();
        for a in &parsed.advances {
            adv_by_from.insert(a.from, a);
        }
        // Batter outcome.
        let (batter_to, batter_out) = match adv_by_from.get(&0) {
            Some(a) => (a.to, a.out && !a.erased_by_error()),
            None => (parsed.dest, parsed.dest == 0),
        };
        // Runner movement plan: base -> (to, out, unearned-flagged).
        let mut moved: BTreeMap<u8, (u8, bool, bool)> = BTreeMap::new();
        for b in (1..=3).rev() {
            if self.bases[b - 1].is_none() {
                continue;
            }
            if let Some(a) = adv_by_from.get(&(b as u8)) {
                moved.insert(b as u8, (a.to, a.out && !a.erased_by_error(), a.unearned_flagged()));
            } else if batter_to >= 1 && b as u8 >= batter_to {
                // The batter's advance pushes this runner one base up.
                moved.insert(b as u8, (b as u8 + 1, false, false));
            }
        }
        // Steal/CS credit to the moving runner (origin base n-1 for dest n).
        match parsed.kind {
            Kind::Steal(n) if (2..=4).contains(&n) => {
                if let Some(Some(runner)) = self.bases.get((n - 2) as usize)
                    && !runner.is_empty()
                {
                    self.batting.entry(runner.clone()).or_default().sb += 1;
                }
            }
            Kind::CaughtStealing(n) if (2..=4).contains(&n) => {
                if let Some(Some(runner)) = self.bases.get((n - 2) as usize)
                    && !runner.is_empty()
                {
                    self.batting.entry(runner.clone()).or_default().cs += 1;
                }
            }
            _ => {}
        }

        // Execute movements.
        let mut runs = 0u32;
        let mut unearned_runs = 0u32;
        let mut _runner_outs = 0u32;
        let mut new_bases: [Option<String>; 3] = [None, None, None];
        for b in 1..=3 {
            let Some(runner) = self.bases[b - 1].clone() else { continue };
            let Some(&(to, out, unearned)) = moved.get(&(b as u8)) else {
                new_bases[b - 1] = Some(runner);
                continue;
            };
            if out {
                _runner_outs += 1;
                continue;
            }
            if to == 4 {
                runs += 1;
                if unearned {
                    unearned_runs += 1;
                }
                if !runner.is_empty() {
                    self.batting.entry(runner.clone()).or_default().r += 1;
                }
            } else {
                new_bases[(to - 1) as usize] = Some(runner);
            }
        }
        // Batter placement.
        if !batter.is_empty() {
            if batter_out {
                // retired: no base
            } else if batter_to == 4 {
                runs += 1;
                if parsed.unearned_batter() {
                    unearned_runs += 1;
                }
                self.batting.entry(batter.to_string()).or_default().r += 1;
            } else if (1..=3).contains(&batter_to) {
                new_bases[(batter_to - 1) as usize] = Some(batter.to_string());
            }
        }

        // Runs charged to the pitcher; unearned bookkeeping for the ER fallback.
        let play_unearned = matches!(parsed.kind, Kind::Error | Kind::WildPitch | Kind::PassedBall | Kind::Balk);
        if runs > 0 {
            if let Some(pl) = pitcher {
                self.pitching.entry(pl.clone()).or_default().r += runs;
            }
            if play_unearned {
                unearned_runs = runs;
            }
            if unearned_runs > runs {
                unearned_runs = runs;
            }
        }

        // RBI credit: runs on the play unless the play or the run is flagged.
        if runs > 0 && !batter.is_empty() && !parsed.no_rbi() {
            let mut rbi = runs;
            for a in &parsed.advances {
                if !a.out && a.to == 4 && a.unearned_flagged() {
                    rbi = rbi.saturating_sub(1);
                }
            }
            if rbi > 0 {
                self.batting.entry(batter.to_string()).or_default().rbi += rbi;
            }
        }

        self.bases = new_bases;
        *runs_unearned += unearned_runs;
        // The kind-semantics count (GDP/NDP/FO/X outs) is authoritative;
        // the tracked count double counts when the runner state has drifted.
        parsed.outs()
    }
}
