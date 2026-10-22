//! Cumulative cut lines (spec 4.5).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::event::Game;

/// Cumulative per-player line through the day before a cut date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CutLine {
    pub player_id: String,
    pub season: i16,
    /// The day before the cut; all games strictly before the cut are included.
    pub as_of: Date,
    pub pa: u32,
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
    /// Batters faced (pitchers only).
    pub bf: u32,
    /// Outs recorded while pitching.
    pub outs: u32,
    pub r: u32,
    pub er: u32,
}

/// Standard as-of cuts: May 1, Jun 1, Jul 1, Aug 1, Sep 1 of each season.
pub fn season_cuts(seasons: &[i16]) -> Vec<Date> {
    let mut cuts = Vec::new();
    for &y in seasons {
        for m in 5..=9 {
            cuts.push(Date::constant(y, m, 1));
        }
    }
    cuts.sort();
    cuts.dedup();
    cuts
}

/// Cumulative per-player lines through the day before each cut. Games are
/// included when their date is known and strictly before the cut.
pub fn cut_lines(games: &[Game], cuts: &[Date]) -> Vec<CutLine> {
    let mut lines = Vec::new();
    for &cut in cuts {
        let as_of = cut - jiff::ToSpan::days(1);
        let mut bat: BTreeMap<&str, crate::event::BatLine> = BTreeMap::new();
        let mut pit: BTreeMap<&str, crate::event::PitLine> = BTreeMap::new();
        for g in games {
            let Some(d) = g.date else { continue };
            if d >= cut {
                continue;
            }
            for (pid, l) in &g.batting {
                bat.entry(pid).or_default().add(l);
            }
            for (pid, l) in &g.pitching {
                pit.entry(pid).or_default().add(l);
            }
        }
        let players: BTreeSet<_> = bat.keys().copied().chain(pit.keys().copied()).collect();
        let empty_bat = crate::event::BatLine::default();
        let empty_pit = crate::event::PitLine::default();
        for pid in players {
            let d = bat.get(pid).unwrap_or(&empty_bat);
            let pitl = pit.get(pid).unwrap_or(&empty_pit);
            lines.push(CutLine {
                player_id: pid.to_string(),
                season: as_of.year(),
                as_of,
                pa: d.pa,
                ab: d.ab,
                h: d.h,
                doubles: d.d,
                triples: d.t,
                hr: d.hr,
                bb: d.bb,
                ibb: d.ibb,
                hbp: d.hbp,
                sf: d.sf,
                sh: d.sh,
                so: d.so,
                bf: pitl.bf,
                outs: pitl.outs,
                r: d.r,
                er: pitl.er,
            });
        }
    }
    lines.sort_by(|a, b| a.as_of.cmp(&b.as_of).then_with(|| a.player_id.cmp(&b.player_id)));
    lines
}

/// Write cut lines as CSV (header matches the bundled cut-lines schema).
pub fn write_cut_lines_csv(w: &mut dyn Write, lines: &[CutLine]) -> std::io::Result<()> {
    writeln!(w, "playerID,season,as_of,pa,ab,h,2b,3b,hr,bb,ibb,hbp,sf,sh,so,bf,outs,r,er")?;
    for l in lines {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            l.player_id,
            l.season,
            l.as_of,
            l.pa,
            l.ab,
            l.h,
            l.doubles,
            l.triples,
            l.hr,
            l.bb,
            l.ibb,
            l.hbp,
            l.sf,
            l.sh,
            l.so,
            l.bf,
            l.outs,
            l.r,
            l.er
        )?;
    }
    Ok(())
}

/// Convenience: write cut lines to a file path.
pub fn write_cut_lines_file(path: &Path, lines: &[CutLine]) -> std::io::Result<()> {
    let f = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(f);
    write_cut_lines_csv(&mut w, lines)
}

use std::collections::BTreeSet;
