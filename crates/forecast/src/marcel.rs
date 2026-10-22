//! The Marcel batting projector (DR-3.19, DR-3.34, DR-3.36) and the naive baseline.
//!
//! ### Marcel form implemented (spec §4.4 "Marcel (batting)")
//!
//! * Seasons weighted 5/4/3 (most recent first). Components (uBB = BB − IBB, HBP, 1B, 2B, 3B, HR)
//!   are summed with those weights over the wOBA denominator `AB + BB − IBB + SF + HBP`.
//! * Regression: 1200 PA of the league rate **of the same weighted seasons** added to the
//!   weighted numerator/denominator.
//! * Age adjustment, multiplicative on the regressed rate: `(29 − age) × 0.006` below 29,
//!   `−(age − 29) × 0.003` at 29 and above. Seasonal age uses the July cut (see [`crate::corpus`]).
//! * Reliability = own-data share of the regressed denominator: `wD / (wD + 1200)`.
//! * Projected PA = `0.5·PA₋₁ + 0.1·PA₋₂ + 200` (DR-3.36: the PA formula *is* the model, scored
//!   on PA MAE with missing players at 100 PA).
//! * Rest-of-season (in-season form): the current partial season enters as a fourth term carrying
//!   the weight 5 — the same weight as the most recent full season, with the partial season's
//!   small denominator self-scaling through the shared regression — and remaining PA = full
//!   projected PA − PA to date, floored at 0. This is a documented deviation from the brief's
//!   "weight 5 scaled by pa fraction" phrasing. Generalised over a partials map in
//!   [`project_marcel_in_season`] (DR-3.31): only listed players carry the weight-5 partial term
//!   and a per-player PA-to-date; the open season enters the league rates only through those
//!   partial lines. The single-partial ROS form delegates to it with a one-entry map — note this
//!   removes the pre-generalisation behaviour of leaking the corpus's target-season lines into
//!   the league rates.
//! * Projected wOBA evaluates the component rates at the **projection-season** weights.
//!
//! No park adjustment inside Marcel. A player with no eligible history is not projected (the
//! harness scores them at the floor); a player whose regressed denominator is 0 is skipped.

use crate::constants::{Constants, Weights};
use crate::corpus::{BattingLine, Corpus, PitchingLine};
use crate::intervals::Bucket;
use std::collections::BTreeMap;

/// wOBA denominator: AB + BB − IBB + SF + HBP.
pub fn denom(l: &BattingLine) -> f64 {
    (l.ab + l.bb.saturating_sub(l.ibb) + l.sf + l.hbp) as f64
}

/// Component counts (uBB, HBP, 1B, 2B, 3B, HR) as wOBA-numerator pieces.
pub fn components(l: &BattingLine) -> [f64; 6] {
    let singles = l.h.saturating_sub(l.h2b + l.h3b + l.hr);
    [(l.bb.saturating_sub(l.ibb)) as f64, l.hbp as f64, singles as f64, l.h2b as f64, l.h3b as f64, l.hr as f64]
}

pub fn woba_of(rates: &[f64; 6], w: &Weights) -> f64 {
    rates[0] * w.bb
        + rates[1] * w.hbp
        + rates[2] * w.single
        + rates[3] * w.double
        + rates[4] * w.triple
        + rates[5] * w.home_run
}

/// The projector interface (spec §4.4 "Projector interface"). A model has an id (name + semantic
/// version) and produces per-player forecasts over a corpus view restricted to information dated
/// on or before the as-of (enforced by the harness perturbation test).
pub trait Model {
    fn id(&self) -> String;

    /// Forecast every player for `target` using only lines from seasons `<= as_of_season`
    /// (preseason: `as_of_season = target - 1`). Keyed by player_id, sorted.
    fn project(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        target: u16,
        as_of_season: u16,
    ) -> BTreeMap<String, Forecast>;

    /// In-season form (DR-3.31): as `project`, but `partials` carries each player's open-season
    /// to-date line as of the cut (players with no row are projected preseason-shaped). Only
    /// models that consume the partials need to override; the default ignores them.
    fn project_in_season(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        target: u16,
        as_of_season: u16,
        partials: &BTreeMap<String, crate::corpus::BattingLine>,
    ) -> BTreeMap<String, Forecast> {
        let _ = partials;
        self.project(corpus, constants, target, as_of_season)
    }
}

/// One forecast: metric projections as points plus reliability and playing time.
#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    /// Projected wOBA (rate), after age adjustment, before any harness re-centring.
    pub woba: f64,
    /// Projected PA for the target season (remaining PA for in-season cuts).
    pub pa: f64,
    /// Own-data share of the regressed denominator.
    pub reliability: f64,
    /// PA already accrued in an open season (0 preseason).
    pub pa_to_date: f64,
    /// Reliability bucket at the fixed thresholds (High ≥ 0.7, Med ≥ 0.45, Low ≥ 0.15, else None).
    pub bucket: Bucket,
}

/// Marcel: the v1 batting model (DR-3.19). Pitching is a separate ticket; the interface is
/// role-generic so `project_pitching` can land without re-shaping callers.
#[derive(Debug, Clone, Copy)]
pub struct Marcel;

/// Naive baseline (spec §4.4 "Naive baseline"): prior season's rate and PA; missing players get
/// the floor (harness side). Reliability is the prior season's own share of `(D, 1200)` so the
/// interval bucketing has something to key on.
#[derive(Debug, Clone, Copy)]
pub struct PriorSeason;

impl Model for Marcel {
    fn id(&self) -> String {
        "marcel.batting/1.0.0".to_string()
    }

    fn project(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        target: u16,
        as_of_season: u16,
    ) -> BTreeMap<String, Forecast> {
        project_marcel(corpus, constants, target, as_of_season, None)
    }

    fn project_in_season(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        target: u16,
        as_of_season: u16,
        partials: &BTreeMap<String, BattingLine>,
    ) -> BTreeMap<String, Forecast> {
        project_marcel_in_season(corpus, constants, target, as_of_season, partials)
    }
}

impl Model for PriorSeason {
    fn id(&self) -> String {
        "naive.prior-season/1.0.0".to_string()
    }

    fn project(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        _target: u16,
        as_of_season: u16,
    ) -> BTreeMap<String, Forecast> {
        let mut out = BTreeMap::new();
        for line in &corpus.batting {
            if line.season != as_of_season {
                continue;
            }
            let d = denom(line);
            if d <= 0.0 {
                continue;
            }
            let rates = rates_of(line);
            let w = constants.for_season(line.season);
            let woba = woba_of(&rates, &w);
            let reliability = d / (d + 1200.0);
            out.insert(
                line.player_id.clone(),
                Forecast { woba, pa: line.pa as f64, reliability, pa_to_date: 0.0, bucket: Bucket::of(reliability) },
            );
        }
        out
    }
}

/// Component rates of one line (no regression, no aging).
pub fn rates_of(l: &BattingLine) -> [f64; 6] {
    let d = denom(l);
    if d <= 0.0 {
        return [0.0; 6];
    }
    let c = components(l);
    [c[0] / d, c[1] / d, c[2] / d, c[3] / d, c[4] / d, c[5] / d]
}

/// Marcel core, generalised over a partials map (map version, DR-3.31). `partials` maps a player
/// to the open season's to-date line (season `target`); only listed players carry a weight-5
/// partial term and a per-player PA-to-date — everyone else is projected preseason-shaped.
/// The partial season's **league-rate contribution** comes only from the listed partial lines
/// (not the corpus's target-season lines, which are future information on a cut).
pub fn project_marcel_in_season(
    corpus: &Corpus,
    constants: &Constants,
    target: u16,
    as_of_season: u16,
    partials: &BTreeMap<String, BattingLine>,
) -> BTreeMap<String, Forecast> {
    // Window: the three completed seasons as_of-2, as_of-1, as_of with weights 5/4/3. Partials
    // join the weighted sums with the weight 5.
    let seasons: Vec<(u16, f64)> = [(as_of_season, 5.0), (as_of_season - 1, 4.0), (as_of_season - 2, 3.0)]
        .into_iter()
        .filter(|(s, _)| corpus.batting.iter().any(|l| l.season == *s))
        .collect();

    // League rates of the same weighted seasons (all players); the open season enters only
    // through the listed partial lines, at weight 5.
    let mut lg_num = [0.0f64; 6];
    let mut lg_den = 0.0f64;
    for line in &corpus.batting {
        let Some((_, w)) = seasons.iter().find(|(s, _)| *s == line.season) else { continue };
        let c = components(line);
        for j in 0..6 {
            lg_num[j] += w * c[j];
        }
        lg_den += w * denom(line);
    }
    for p in partials.values() {
        if p.season != target {
            continue;
        }
        let c = components(p);
        for j in 0..6 {
            lg_num[j] += 5.0 * c[j];
        }
        lg_den += 5.0 * denom(p);
    }
    let lg_rate: [f64; 6] = if lg_den > 0.0 {
        lg_num.map(|n| n / lg_den)
    } else {
        return BTreeMap::new();
    };

    // Weighted component sums per player over the window.
    let mut own: BTreeMap<String, ([f64; 6], f64)> = BTreeMap::new();
    for line in &corpus.batting {
        let Some((_, w)) = seasons.iter().find(|(s, _)| *s == line.season) else { continue };
        let c = components(line);
        let e = own.entry(line.player_id.clone()).or_insert(([0.0; 6], 0.0));
        for (sum, cc) in e.0.iter_mut().zip(c) {
            *sum += w * cc;
        }
        e.1 += w * denom(line);
    }
    for (id, p) in partials {
        if p.season != target {
            continue;
        }
        let c = components(p);
        let e = own.entry(id.clone()).or_insert(([0.0; 6], 0.0));
        for (sum, cc) in e.0.iter_mut().zip(c) {
            *sum += 5.0 * cc;
        }
        e.1 += 5.0 * denom(p);
    }

    // PA history for the playing-time formula.
    let mut pa_hist: BTreeMap<String, BTreeMap<u16, f64>> = BTreeMap::new();
    for line in &corpus.batting {
        if line.season <= as_of_season {
            pa_hist.entry(line.player_id.clone()).or_default().insert(line.season, line.pa as f64);
        }
    }

    let w = constants.for_season(target);
    let mut out = BTreeMap::new();
    for (id, (num, den)) in own {
        if den <= 0.0 {
            continue;
        }
        let rate: [f64; 6] = std::array::from_fn(|j| (num[j] + 1200.0 * lg_rate[j]) / (den + 1200.0));

        // Age adjustment: the **projection season's** age from the birth table (July cut). Tango's
        // golden files are matched with this convention; the line-storage age (per-season) is not
        // used here because a veteran retiring before the target season has no target-season line.
        let age_from_last_line = corpus
            .batting
            .iter()
            .filter(|l| l.player_id == *id && l.season <= target)
            .map(|l| l.age)
            .max()
            .unwrap_or(29);
        let age = match corpus.birth.get(&id) {
            Some(&(by, bm)) => target.saturating_sub(by).saturating_sub(u16::from(bm > 6)),
            None => age_from_last_line,
        };
        let factor = if age < 29 { (29.0 - f64::from(age)) * 0.006 } else { -(f64::from(age) - 29.0) * 0.003 };
        let rate = rate.map(|r| r * (1.0 + factor));

        let hist = pa_hist.entry(id.clone()).or_default();
        let pa1 = hist.get(&as_of_season).copied().unwrap_or(0.0);
        let pa2 = hist.get(&(as_of_season - 1)).copied().unwrap_or(0.0);
        let full_pa = 0.5 * pa1 + 0.1 * pa2 + 200.0;
        // PA to date is per player: listed partials carry their own to-date PA, others 0.
        let has_partial = partials.get(&id).is_some_and(|p| p.season == target);
        let pa_to_date = if has_partial { partials[&id].pa as f64 } else { 0.0 };
        let remaining = (full_pa - pa_to_date).max(0.0);

        let reliability = den / (den + 1200.0);
        out.insert(
            id,
            Forecast {
                woba: woba_of(&rate, &w),
                pa: if has_partial { remaining } else { full_pa },
                reliability,
                pa_to_date,
                bucket: Bucket::of(reliability),
            },
        );
    }
    out
}

/// Marcel core, single-partial form (CLI ROS path). Delegates to the map version
/// [`project_marcel_in_season`] with a one-entry map (documented choice: one projection body).
/// `partial` is the current open season's to-date line (its season is `target`); `None` for
/// preseason. Note the **league-rate change vs the pre-generalisation body:** the open season
/// enters the league rates only through the partial line, not through the corpus's target-season
/// lines (which on a cut are future information).
pub fn project_marcel(
    corpus: &Corpus,
    constants: &Constants,
    target: u16,
    as_of_season: u16,
    partial: Option<&BattingLine>,
) -> BTreeMap<String, Forecast> {
    let mut partials: BTreeMap<String, BattingLine> = BTreeMap::new();
    if let Some(p) = partial {
        partials.insert(p.player_id.clone(), p.clone());
    }
    project_marcel_in_season(corpus, constants, target, as_of_season, &partials)
}

/// ROS entry point: in-season form with the open season's to-date line. The partial line replaces
/// nothing — it is added with the weight 5 on top of the 5/4/3 window (see module header).
pub fn marcel_ros(corpus: &Corpus, constants: &Constants, partial: &BattingLine) -> BTreeMap<String, Forecast> {
    let target = partial.season;
    project_marcel(corpus, constants, target, target - 1, Some(partial))
}

// =====================================================================
// Pitching Marcel (spec §4.4 "Pitching: seasons 3/2/1 on innings…"),
// re-derived per DR-3.19a from the committed `pitching-marcel-2010.csv` fixture.
//
// ### Marcel pitching form implemented (DR-3.19a)
//
// * Seasons weighted 3/2/1 (most recent first) on recorded outs (IPouts). Rate components
//   are per-out averages of SO, BB, HBP, HR, H, ER and IBB, regressed toward the **league
//   average of the pitcher's own league** (Tango's marcel splits AL/NL because of the DH;
//   measured: split raises the golden ERA match rate from 0.22 to 0.27).
// * Regression: +804 IPouts (= 134 IPouts × the 6 weight units ≈ 268 IP) of the league rate of
//   the same weighted seasons, identical for starters and relievers. **DEVIATION from the
//   spec's 60/25 constants:** DR-3.19a's 60/25 was derived for the *no-history floor*; the
//   golden fixture's `reliability` column is reproduced to ±0.01 for 100% of the 964 rows
//   only by the flat 268-IP constant, so that is what is implemented (derived from the
//   fixture columns, not fitted to ERA).
// * Age: `target − birthYear` (no July cut — the fixture's `Age` column matches that for
//   955/964 rows), applied to pitching rates in the *inverse* direction of batting: young
//   pitchers improve, so their allowed-run rates shrink: `1/(1 + 0.006·(29 − age))` below 29,
//   `1 + 0.003·(age − 29)` at 29 and above.
// * Projected outs = `75 + 105·(GS/G) + 0.5·outs₋₁ + 0.1·outs₋₂`, i.e. IP = `25 + 35·(GS/G)
//   + 0.5·IP₋₁ + 0.1·IP₋₂` — a GS/G-weighted interpolation between the reliever and starter
//   floors. **DEVIATION from the spec's flat +50:** the role-weighted floor reproduces the
//   fixture's `mIP` column for 91% of rows (±5 IP); the flat +50 for 58%.
// * Rebaseline: each component's projected counts are scaled so the IP-weighted league rate
//   of all projected pitchers equals the 3/2/1-weighted target league rate (Tango's marcel
//   step 6, "rebaseline against the assumed league average").
// * ERA = projected ER / projected IP × 9 from the rebaselined ER count. The BaseRuns composite
//   was implemented and measured (see the golden test header): BR over the projected components
//   does **not** improve the ERA match (Tango's `bsrER` column cannot be reproduced from the
//   fixture's own `mH/mBB/mIBB/mHBP/mSO/mHR/mIP` to better than ≈0.9 runs median, and ERA tracks
//   `mER`, not `bsrER`), so the direct ER projection is kept.
// * Rest-of-season: the open season's to-date line carries the flat weight 3 (the same weight as
//   the most recent full season) and remaining IP = full projected IP − IP to date, floored at 0.
// * No park adjustment.

/// Projected pitching metrics for one player.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchingForecast {
    /// Projected ERA (regressed, age-adjusted ER/9).
    pub era: f64,
    /// Projected IP for the target season (remaining IP for in-season cuts).
    pub ip: f64,
    /// Own-data share of the regressed IP denominator.
    pub reliability: f64,
    /// IP already accrued in an open season (0 preseason).
    pub ip_to_date: f64,
    /// Reliability bucket (same fixed thresholds as batting).
    pub bucket: Bucket,
}

/// The projector interface, pitching side. Kept separate from [`Model`] (batting-shaped
/// [`Forecast`]) so neither shape has to pretend to be the other.
pub trait PitchingModel {
    fn id(&self) -> String;

    /// Forecast every pitcher for `target` using only lines from seasons `<= as_of_season`
    /// (preseason: `as_of_season = target - 1`). Keyed by player_id, sorted.
    fn project_pitching(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        target: u16,
        as_of_season: u16,
    ) -> BTreeMap<String, PitchingForecast>;
}

/// Pitching Marcel (spec §4.4, DR-3.19 pitcher constants).
#[derive(Debug, Clone, Copy)]
pub struct PitchingMarcel;

impl PitchingModel for PitchingMarcel {
    fn id(&self) -> String {
        "marcel.pitching/1.0.0".to_string()
    }

    fn project_pitching(
        &self,
        corpus: &Corpus,
        constants: &Constants,
        target: u16,
        as_of_season: u16,
    ) -> BTreeMap<String, PitchingForecast> {
        project_pitching_marcel(corpus, constants, target, as_of_season, None)
    }
}

const N_PC: usize = 7; // SO, BB, HBP, HR, H, ER, IBB cumulative counts (per-out league average)
/// Regression: 134 IPouts × the 6 weight units = 804 IPouts = 268 IP. Reproduces the golden
/// fixture's `reliability` column for 100% of rows (derived, DR-3.19a).
const P_REG_OUTS: f64 = 804.0;

/// Age adjustment for pitching rates: the *inverse* of the batting direction (allowed-run rates
/// improve while young, worsen while old). Age is `target − birthYear` (no July cut).
fn pitching_age_factor(age: u16) -> f64 {
    if age < 29 { 1.0 / (1.0 + 0.006 * (29.0 - f64::from(age))) } else { 1.0 + 0.003 * (f64::from(age) - 29.0) }
}

fn p_den(l: &PitchingLine) -> f64 {
    l.ip_outs as f64 / 3.0
}

fn p_components(l: &PitchingLine) -> [f64; N_PC] {
    [l.so as f64, l.bb as f64, l.hbp as f64, l.hr as f64, l.h as f64, l.er as f64, l.ibb as f64]
}

/// Starter rule: a season is a starter if `2·GS ≥ G` (GS ≥ G/2).
pub fn is_starter(l: &PitchingLine) -> bool {
    2 * l.gs >= l.g
}

/// Pitching Marcel core. `partial` is the open season's to-date line (season = `target`);
/// `None` for preseason.
pub fn project_pitching_marcel(
    corpus: &Corpus,
    _constants: &Constants,
    target: u16,
    as_of_season: u16,
    partial: Option<&PitchingLine>,
) -> BTreeMap<String, PitchingForecast> {
    let seasons: Vec<(u16, f64)> = [(as_of_season, 3.0), (as_of_season - 1, 2.0), (as_of_season - 2, 1.0)]
        .into_iter()
        .filter(|(s, _)| corpus.pitching.iter().any(|l| l.season == *s))
        .collect();

    // Per-league raw season totals of the window (per-out averages are taken per league and
    // season; the DH split matters, see the module header). Falls back to the combined average.
    let mut lg_sums: BTreeMap<(String, u16), ([f64; N_PC], f64)> = BTreeMap::new();
    let mut all_sums: BTreeMap<u16, ([f64; N_PC], f64)> = BTreeMap::new();
    for line in &corpus.pitching {
        if line.ip_outs == 0 || !seasons.iter().any(|(s, _)| *s == line.season) {
            continue;
        }
        let c = p_components(line);
        let e = lg_sums.entry((line.lg.clone(), line.season)).or_insert(([0.0; N_PC], 0.0));
        for (n, cc) in e.0.iter_mut().zip(c) {
            *n += cc;
        }
        e.1 += line.ip_outs as f64;
        let e = all_sums.entry(line.season).or_insert(([0.0; N_PC], 0.0));
        for (n, cc) in e.0.iter_mut().zip(c) {
            *n += cc;
        }
        e.1 += line.ip_outs as f64;
    }
    if all_sums.is_empty() {
        return BTreeMap::new();
    }
    // Rebaseline target: 3/2/1-weighted average of the seasonal per-out rates (per league).
    let mut lg_target: BTreeMap<String, [f64; N_PC]> = BTreeMap::new();
    for lg in lg_sums.keys().map(|(lg, _)| lg.clone()).collect::<Vec<_>>() {
        let mut tgt = [0.0f64; N_PC];
        let mut w_sum = 0.0;
        for (s, w) in &seasons {
            if let Some((tot, outs)) = lg_sums.get(&(lg.clone(), *s))
                && *outs > 0.0
            {
                for (t, v) in tgt.iter_mut().zip(tot) {
                    *t += w * v / outs;
                }
                w_sum += w;
            }
        }
        if w_sum > 0.0 {
            lg_target.insert(lg, tgt.map(|t| t / w_sum));
        }
    }

    // Weighted component sums per player over the window (the partial open season enters at 3).
    let mut own: BTreeMap<&str, ([f64; N_PC], f64, f64)> = BTreeMap::new();
    let mut player_lg: BTreeMap<&str, String> = BTreeMap::new();
    for line in &corpus.pitching {
        let Some((_, w)) = seasons.iter().find(|(s, _)| *s == line.season) else { continue };
        if line.ip_outs == 0 {
            continue;
        }
        let c = p_components(line);
        let e = own.entry(line.player_id.as_str()).or_insert(([0.0; N_PC], 0.0, 0.0));
        for (sum, cc) in e.0.iter_mut().zip(c) {
            *sum += w * cc;
        }
        e.1 += w * line.ip_outs as f64;
        let lg_entry = lg_sums.get(&(line.lg.clone(), line.season));
        if let Some((lg_tot, lg_outs)) = lg_entry
            && *lg_outs > 0.0
        {
            for t in lg_tot {
                e.2 += w * t / lg_outs * line.ip_outs as f64;
            }
        }
        let e = player_lg.entry(line.player_id.as_str()).or_default();
        *e = line.lg.clone();
    }
    if let Some(p) = partial {
        let c = p_components(p);
        let e = own.entry(p.player_id.as_str()).or_insert(([0.0; N_PC], 0.0, 0.0));
        for (sum, cc) in e.0.iter_mut().zip(c) {
            *sum += 3.0 * cc;
        }
        e.1 += 3.0 * p.ip_outs as f64;
        let e = player_lg.entry(p.player_id.as_str()).or_default();
        *e = p.lg.clone();
    }

    // IP history for the playing-time formula; the GS/G role fraction comes from the most
    // recent active season ≤ as-of.
    let mut outs_hist: BTreeMap<&str, BTreeMap<u16, f64>> = BTreeMap::new();
    for line in &corpus.pitching {
        if line.season <= as_of_season {
            outs_hist.entry(line.player_id.as_str()).or_default().insert(line.season, line.ip_outs as f64);
        }
    }
    let mut latest_role_season: BTreeMap<&str, u16> = BTreeMap::new();
    let mut role_frac: BTreeMap<&str, f64> = BTreeMap::new();
    for line in &corpus.pitching {
        if line.season > as_of_season || line.g == 0 {
            continue;
        }
        let entry = latest_role_season.entry(line.player_id.as_str()).or_insert(line.season);
        if line.season > *entry {
            *entry = line.season;
            role_frac.insert(line.player_id.as_str(), line.gs as f64 / line.g as f64);
        } else if line.season == *entry {
            role_frac.insert(line.player_id.as_str(), line.gs as f64 / line.g as f64);
        }
    }
    if let Some(p) = partial
        && p.g > 0
    {
        role_frac.insert(p.player_id.as_str(), p.gs as f64 / p.g as f64);
    }
    let partial_ip = partial.map(p_den).unwrap_or(0.0);

    struct Cand {
        id: String,
        lg: String,
        rate: [f64; N_PC],
        pt_outs: f64,
        w_outs: f64,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for (id, (num, den, _lg_num_unused)) in own {
        if den <= 0.0 {
            continue;
        }
        let lg = player_lg.get(id).cloned().unwrap_or_else(|| "NL".to_string());
        // Weighted league average for the regression (per-out league rates weighted by the
        // pitcher's own weighted outs — the season rates themselves are league-wide).
        let mut lg_rate = [0.0f64; N_PC];
        let mut w_sum = 0.0;
        for line in &corpus.pitching {
            let Some((_, w)) = seasons.iter().find(|(s, _)| *s == line.season) else { continue };
            if line.lg != lg || line.ip_outs == 0 || line.player_id.as_str() != id {
                continue;
            }
            let (tot, outs) = lg_sums.get(&(lg.clone(), line.season)).expect("league sums");
            if *outs <= 0.0 {
                continue;
            }
            w_sum += w * line.ip_outs as f64;
            for (n, t) in lg_rate.iter_mut().zip(tot) {
                *n += w * line.ip_outs as f64 * t / outs;
            }
        }
        if w_sum > 0.0 {
            lg_rate = lg_rate.map(|n| n / w_sum);
        } else {
            // Fallback: combined league average.
            let mut w_c = 0.0;
            for (s, w) in &seasons {
                if let Some((tot, outs)) = all_sums.get(s)
                    && *outs > 0.0
                {
                    w_c += w;
                    for (n, t) in lg_rate.iter_mut().zip(tot) {
                        *n += w * t / outs;
                    }
                }
            }
            lg_rate = lg_rate.map(|n| n / w_c);
        }
        let rate: [f64; N_PC] = std::array::from_fn(|j| (num[j] + lg_rate[j] * P_REG_OUTS) / (den + P_REG_OUTS));
        let age_from_last_line = corpus
            .pitching
            .iter()
            .filter(|l| l.player_id == *id && l.season <= target)
            .map(|l| l.age)
            .max()
            .unwrap_or(29);
        let age = match corpus.birth.get(id) {
            Some(&(by, _bm)) => target.saturating_sub(by),
            None => age_from_last_line,
        };
        let rate = rate.map(|r| r * pitching_age_factor(age));
        let hist = outs_hist.entry(id).or_default();
        let outs1 = hist.get(&as_of_season).copied().unwrap_or(0.0);
        let outs2 = hist.get(&(as_of_season - 1)).copied().unwrap_or(0.0);
        let frac = role_frac.get(id).copied().unwrap_or(1.0);
        let pt_outs = 75.0 + 105.0 * frac + 0.5 * outs1 + 0.1 * outs2;
        cands.push(Cand { id: id.to_string(), lg, rate, pt_outs, w_outs: den });
    }

    // Rebaseline multiplier per league (marcel step 6: force the projected league rate onto the
    // weighted target league rate).
    let mut agg: BTreeMap<String, ([f64; N_PC], f64)> = BTreeMap::new();
    for c in &cands {
        let e = agg.entry(c.lg.clone()).or_insert(([0.0; N_PC], 0.0));
        for (n, cc) in e.0.iter_mut().zip(c.rate.map(|r| r * c.pt_outs)) {
            *n += cc;
        }
        e.1 += c.pt_outs;
    }
    let mut out = BTreeMap::new();
    for c in cands {
        let (sum, tp) = agg.get(&c.lg).expect("league aggregated");
        let tgt = lg_target.get(&c.lg).cloned().unwrap_or_else(|| {
            // No league target (single-league corpus edge): combined weighted average.
            let mut t = [0.0f64; N_PC];
            let mut w_c = 0.0;
            for (s, w) in &seasons {
                if let Some((tot, outs)) = all_sums.get(s)
                    && *outs > 0.0
                {
                    w_c += w;
                    for (t2, v) in t.iter_mut().zip(tot) {
                        *t2 += w * v / outs;
                    }
                }
            }
            t.map(|t2| t2 / w_c)
        });
        let mult: [f64; N_PC] = std::array::from_fn(|j| {
            let agg_rate = if *tp > 0.0 { sum[j] / *tp } else { 0.0 };
            if agg_rate > 0.0 { tgt[j] / agg_rate } else { 1.0 }
        });
        let full_ip = c.pt_outs / 3.0;
        let remaining = (full_ip - partial_ip).max(0.0);
        let reliability = c.w_outs / (c.w_outs + P_REG_OUTS);
        let era = (c.rate[5] * mult[5] * 27.0).max(0.0);
        out.insert(
            c.id,
            PitchingForecast {
                era,
                ip: if partial.is_some() { remaining } else { full_ip },
                reliability,
                ip_to_date: partial_ip,
                bucket: Bucket::of(reliability),
            },
        );
    }
    out
}

/// ROS entry point, pitching: in-season form with the open season's to-date line (weight 3).
pub fn pitching_ros(
    corpus: &Corpus,
    constants: &Constants,
    partial: &PitchingLine,
) -> BTreeMap<String, PitchingForecast> {
    let target = partial.season;
    project_pitching_marcel(corpus, constants, target, target - 1, Some(partial))
}
