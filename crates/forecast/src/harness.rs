//! The walk-forward harness (spec §4.4 "Walk-forward protocol").
//!
//! Per (season, cut): train on the three completed seasons before the as-of, project, score
//! against actuals. The January 1 cut folds preseason (as-of season = target − 1); the May–September
//! month-start cuts fold in season, with each player's to-date line from the bundled cut-lines
//! sample (as-of the day before the cut) entering the projection through
//! [`crate::marcel::Model::project_in_season`]. Residuals from each fold feed the interval model of
//! later folds of the same cut (strictly-earlier; per-cut residual history, [`WeightBasis::Future`]
//! for the in-season folds). Seasons 2001–2003 are interval warm-up: they are scored on MAE/PA MAE
//! only, and coverage (PICP80/MIS/gate) begins once residual history exists (folds from 2004 on,
//! with buckets below `n_min` merged upward). A corpus without cut lines reproduces the
//! preseason-only behaviour.

use crate::corpus::{Corpus, Protocol};
use crate::intervals::{Bucket, IntervalModel, Residual, WeightBasis};
use crate::marcel::Model;
use crate::scores::{self, ScoreReport, SeasonScore};
use std::collections::BTreeMap;

/// Rolling residual-history window (in scored seasons) feeding the interval model, batting. The
/// bucket-conditional centred error quantiles drift season to season; the window tracks the drift
/// while keeping `n_min` comfortably populated.
const HISTORY_SEASONS_BATTING: usize = 4;
/// Rolling window for the pitching folds (see [`HISTORY_SEASONS_BATTING`]).
const HISTORY_SEASONS_PITCHING: usize = 6;

/// One scored fold row per player (kept for tests and the perturbation check).
#[derive(Debug, Clone, PartialEq)]
pub struct FoldRow {
    pub season: u16,
    pub cut: String,
    pub player_id: String,
    pub projected_woba: f64,
    pub projected_pa: f64,
    pub bucket: Bucket,
    pub actual_woba: f64,
    pub actual_pa: u32,
    /// Error = actual − projected (before re-centring).
    pub err: f64,
}

/// One fold's result.
#[derive(Debug, Clone, PartialEq)]
pub struct Fold {
    pub season: u16,
    pub cut: String,
    pub rows: Vec<FoldRow>,
    pub score: SeasonScore,
    /// The fold's own rate-conditional standardised residuals — exactly the rows pushed into
    /// the cut's rolling history. Kept so [`fit_intervals_for`] can replay a regime's history
    /// without re-scoring.
    pub residuals: Vec<Residual>,
}

/// Run the walk-forward protocol for one model over the corpus. For every target season: one
/// preseason fold (cut `01-01`) and, when the corpus carries cut lines, one scored fold per
/// in-season cut (the protocol's May–September month starts). An empty `corpus.cuts` reproduces
/// the preseason-only behaviour.
pub fn walk_forward(
    model: &dyn Model,
    corpus: &Corpus,
    protocol: &Protocol,
    constants: &crate::constants::Constants,
) -> (Vec<Fold>, ScoreReport) {
    let mut folds = Vec::new();
    // Residual history per cut label, strictly-earlier within the label: residuals of fold
    // (season, cut) feed later folds of the same cut, truncated to the rolling window.
    let mut residual_history: BTreeMap<String, Vec<(u16, Vec<Residual>)>> = BTreeMap::new();
    const NO_TO_DATE: Option<&BTreeMap<String, f64>> = None;

    for &season in &protocol.seasons {
        // As-of season: January 1 of the target year, i.e. target − 1, for every cut.
        let as_of_season = season - 1;
        let training: Vec<&crate::corpus::BattingLine> =
            corpus.batting.iter().filter(|l| l.season <= as_of_season && l.season > as_of_season - 3).collect();
        if training.is_empty() {
            continue;
        }

        // Actuals: all players with a line in the season (shared by every fold of the season).
        let mut actuals: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for line in corpus.batting_season(season) {
            let w = constants.for_season(season);
            let entry = actuals.entry(line.player_id.clone()).or_insert((0.0, 0.0));
            entry.0 += scores::actual_woba(line, &w) * line.pa as f64;
            entry.1 += line.pa as f64;
        }
        for v in actuals.values_mut() {
            if v.1 > 0.0 {
                v.0 /= v.1;
            }
        }

        // Floor (Whiffs): PA-weighted league mean wOBA of the last training season with ≥ 100 PA.
        let mut floor_woba = None;
        for train_season in (1998..season).rev() {
            let mut num = 0.0;
            let mut den = 0.0;
            let w = constants.for_season(train_season);
            for line in corpus.batting_season(train_season) {
                if line.pa >= 100 {
                    num += line.pa as f64 * scores::actual_woba(line, &w);
                    den += line.pa as f64;
                }
            }
            if den > 0.0 {
                floor_woba = Some(num / den);
                break;
            }
        }
        let floor_woba = floor_woba.unwrap_or(0.320);

        // Preseason fold: as-of January 1 of the target year.
        let projections = model.project(corpus, constants, season, as_of_season);
        if !projections.is_empty() {
            // Interval model from strictly-earlier folds of this cut, truncated to the rolling
            // window. Warm-up: the cut's first three scored seasons carry no coverage/MIS
            // (residual history too young by definition).
            let recent: Vec<Residual> = residual_history
                .entry("01-01".to_string())
                .or_default()
                .iter()
                .rev()
                .take(HISTORY_SEASONS_BATTING)
                .flat_map(|(_, rs)| rs.iter().copied())
                .collect();
            // Warm-up: the model materialises only once the label has 3 scored SEASONS of
            // history (DR-3.21a) — the residual vector's length is not the right measure.
            let history_len = residual_history.get("01-01").map_or(0, Vec::len);
            let intervals = if history_len < 3 || recent.is_empty() {
                None
            } else {
                Some(IntervalModel::fit(&recent, crate::intervals::N_MIN))
            };

            let season_projections: Vec<(String, f64, f64, Bucket)> =
                projections.iter().map(|(id, f)| (id.clone(), f.woba, f.pa, f.bucket)).collect();
            let fold_in = scores::SeasonFold {
                season,
                cut: "01-01".to_string(),
                projections: season_projections.clone(),
                actuals: &actuals,
                floor_woba,
                floor_pa: 100.0,
                intervals: intervals.as_ref(),
                weight_basis: WeightBasis::Actual,
                pa_to_date: NO_TO_DATE,
            };
            let score = scores::score_season(&fold_in);

            // Rows, plus residual collection over the rate-conditional scored population (the same
            // aligned rows the season score covers: an observed rate, no floor-substituted
            // phantom), floor rows included under bucket None. Centred error, standardised by the
            // actual PA — the fold's [`WeightBasis`], the same side the season score
            // de-standardises with.
            let mut rows = Vec::with_capacity(season_projections.len());
            for (id, pw, pa, bucket) in &season_projections {
                let (aw, apa) = match actuals.get(id) {
                    Some(x) => *x,
                    None => (floor_woba, 100.0),
                };
                let err = aw - pw;
                rows.push(FoldRow {
                    season,
                    cut: "01-01".to_string(),
                    player_id: id.clone(),
                    projected_woba: *pw,
                    projected_pa: *pa,
                    bucket: *bucket,
                    actual_woba: aw,
                    actual_pa: apa as u32,
                    err,
                });
            }
            let aligned = scores::align_rows(&season_projections, &actuals, floor_woba, 100.0);
            let residuals: Vec<Residual> = aligned
                .iter()
                .filter(|r| r.actual_pa > 0.0 && !r.phantom)
                .map(|r| Residual { z: (r.actual_woba - r.centred) * r.actual_pa.max(1.0).sqrt(), bucket: r.bucket })
                .collect();
            folds.push(Fold { season, cut: "01-01".to_string(), rows, score, residuals: residuals.clone() });
            residual_history.entry("01-01".to_string()).or_default().push((season, residuals));
        }

        // In-season folds. No cut lines in the corpus → zero in-season folds (preseason-only).
        if corpus.cuts.is_empty() {
            continue;
        }
        for (label, _cut_ord) in &protocol.cuts {
            if *label == "01-01" {
                continue;
            }
            // Partials: the latest cut-line row per player strictly before the cut (the cut
            // lines' as_of is the day before the cut, so each cut's own rows are included).
            let cut_md: Vec<u8> = label.split('-').filter_map(|p| p.parse::<u8>().ok()).collect();
            if cut_md.len() != 2 {
                continue;
            }
            let (cut_m, cut_d) = (cut_md[0], cut_md[1]);
            let mut latest: BTreeMap<&str, &crate::corpus::CutLine> = BTreeMap::new();
            for cut in corpus.cuts.iter().filter(|c| c.season == season && (c.month, c.day) < (cut_m, cut_d)) {
                match latest.get(cut.player_id.as_str()) {
                    Some(prev) if (prev.month, prev.day) >= (cut.month, cut.day) => {}
                    _ => {
                        latest.insert(&cut.player_id, cut);
                    }
                }
            }
            if latest.is_empty() {
                // No to-date data for this (season, cut) — e.g. 2020's COVID-shortened season has
                // no April–June cut lines — so the fold would be preseason-shaped for everyone;
                // skip it.
                continue;
            }
            let mut partials: BTreeMap<String, crate::corpus::BattingLine> = BTreeMap::new();
            for cut in latest.values() {
                let (by, bm) = corpus.birth.get(&cut.player_id).copied().unwrap_or((0, 0));
                partials.insert(
                    cut.player_id.clone(),
                    crate::corpus::BattingLine {
                        player_id: cut.player_id.clone(),
                        season: cut.season,
                        team: String::new(),
                        lg: String::new(),
                        age: season.saturating_sub(by).saturating_sub(u16::from(bm > 6)),
                        pa: cut.pa,
                        ab: cut.ab,
                        h: cut.h,
                        h2b: cut.h2b,
                        h3b: cut.h3b,
                        hr: cut.hr,
                        bb: cut.bb,
                        ibb: cut.ibb,
                        hbp: cut.hbp,
                        sf: cut.sf,
                        sh: cut.sh,
                        so: cut.so,
                    },
                );
            }

            let projections = model.project_in_season(corpus, constants, season, as_of_season, &partials);
            if projections.is_empty() {
                continue;
            }

            // Season-total PA per projected player (= PA to date + remaining); PA MAE compares
            // totals, and the Future weight basis subtracts the to-date part off the actual.
            let to_date: BTreeMap<String, f64> = partials.iter().map(|(id, p)| (id.clone(), p.pa as f64)).collect();

            let recent: Vec<Residual> = residual_history
                .entry(label.clone())
                .or_default()
                .iter()
                .rev()
                .take(HISTORY_SEASONS_BATTING)
                .flat_map(|(_, rs)| rs.iter().copied())
                .collect();
            let history_len = residual_history.get(label).map_or(0, Vec::len);
            let intervals = if history_len < 3 || recent.is_empty() {
                None
            } else {
                Some(IntervalModel::fit(&recent, crate::intervals::N_MIN))
            };

            let season_projections: Vec<(String, f64, f64, Bucket)> =
                projections.iter().map(|(id, f)| (id.clone(), f.woba, f.pa_to_date + f.pa, f.bucket)).collect();
            let fold_in = scores::SeasonFold {
                season,
                cut: label.clone(),
                projections: season_projections.clone(),
                actuals: &actuals,
                floor_woba,
                floor_pa: 100.0,
                intervals: intervals.as_ref(),
                weight_basis: WeightBasis::Future,
                pa_to_date: Some(&to_date),
            };
            let score = scores::score_season(&fold_in);

            // Rows (projected PA = the season total) and per-cut residuals standardised by the
            // future sample √(actual_pa − pa_to_date), the fold's [`WeightBasis::Future`].
            let mut rows = Vec::with_capacity(season_projections.len());
            for (id, pw, pa, bucket) in &season_projections {
                let (aw, apa) = match actuals.get(id) {
                    Some(x) => *x,
                    None => (floor_woba, 100.0),
                };
                let err = aw - pw;
                rows.push(FoldRow {
                    season,
                    cut: label.clone(),
                    player_id: id.clone(),
                    projected_woba: *pw,
                    projected_pa: *pa,
                    bucket: *bucket,
                    actual_woba: aw,
                    actual_pa: apa as u32,
                    err,
                });
            }
            let aligned = scores::align_rows(&season_projections, &actuals, floor_woba, 100.0);
            let residuals: Vec<Residual> = aligned
                .iter()
                .filter(|r| r.actual_pa > 0.0 && !r.phantom)
                .map(|r| {
                    let future_pa = (r.actual_pa - to_date.get(&r.id).copied().unwrap_or(0.0)).max(1.0);
                    Residual { z: (r.actual_woba - r.centred) * future_pa.sqrt(), bucket: r.bucket }
                })
                .collect();
            folds.push(Fold { season, cut: label.clone(), rows, score, residuals: residuals.clone() });
            residual_history.entry(label.clone()).or_default().push((season, residuals));
        }
    }

    // Season scores + pooled + gate. The CI gate is the PRESEASON gate (user story 25: in-season
    // cuts score separately); the in-season Wilson check rides along as an informational line.
    let seasons: Vec<SeasonScore> = folds.iter().map(|f| f.score.clone()).collect();
    let preseason: Vec<SeasonScore> = seasons.iter().filter(|s| s.cut == "01-01").cloned().collect();
    let in_season: Vec<SeasonScore> = seasons.iter().filter(|s| s.cut != "01-01").cloned().collect();
    let mut pooled_num = 0.0;
    let mut pooled_den = 0.0;
    let mut pa_num = 0.0;
    let mut pa_n = 0.0;
    let mut pre_num = 0.0;
    let mut pre_den = 0.0;
    let mut in_num = 0.0;
    let mut in_den = 0.0;
    for f in &folds {
        let is_pre = f.cut == "01-01";
        for r in &f.rows {
            pooled_num += r.actual_pa as f64 * r.err.abs();
            pooled_den += r.actual_pa as f64;
            pa_num += (r.projected_pa - r.actual_pa as f64).abs();
            pa_n += 1.0;
            if is_pre {
                pre_num += r.actual_pa as f64 * r.err.abs();
                pre_den += r.actual_pa as f64;
            } else {
                in_num += r.actual_pa as f64 * r.err.abs();
                in_den += r.actual_pa as f64;
            }
        }
    }
    let pooled_mae = if pooled_den > 0.0 { pooled_num / pooled_den } else { 0.0 };
    let report = ScoreReport {
        pooled_mae,
        pooled_pa_mae: if pa_n > 0.0 { pa_num / pa_n } else { 0.0 },
        gate: scores::gate(&preseason),
        // Informational: enforce nothing on the in-season regime per user story 25.
        in_season_gate: if in_season.len() >= 3 { Some(scores::gate(&in_season)) } else { None },
        preseason_mae: if pre_den > 0.0 { Some(pre_num / pre_den) } else { None },
        in_season_mae: if in_den > 0.0 { Some(in_num / in_den) } else { None },
        seasons,
    };
    (folds, report)
}

/// Which cut's residual history the artifact interval model is fit from (DR-3.40 wiring).
///
/// Rationale (documented choice): a **next-season** projection is made with no in-season
/// information at all, so it inherits the January-1 regime's history — the same beam the
/// harness's own 01-01 folds standardise against. A **rest-of-season** projection made as of
/// a mid-season date has that date's to-date line in the projection, so it inherits the
/// closest in-season cut's own history (the newest cut at or before the as-of day); before
/// the first in-season cut, or when the protocol has no in-season cuts, 01-01 is the
/// substitute. Fitting includes every scored season of the chosen label inside the rolling
/// window: the walk-forward is complete, so all its residuals are observed and none of them
/// can look ahead at the season being projected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    /// January-1 preseason history (next-season projections).
    NextSeason,
    /// Rest-of-season as of a calendar month/day: the newest in-season cut at or before it.
    RestOfSeason { month: u8, day: u8 },
}

/// Refit the interval model for one regime from a completed walk-forward's folds.
///
/// Replays the cut label's rolling residual history exactly as the harness built it (the
/// last `HISTORY_SEASONS_*` scored folds of that label), then fits per-bucket quantiles.
/// Returns `None` during interval warm-up: the label has fewer than 3 scored seasons (the
/// harness materialises no model before then, DR-3.21a) or no residuals at all.
pub fn fit_intervals_for(regime: Regime, folds: &[Fold], protocol: &Protocol) -> Option<IntervalModel> {
    let label = match regime {
        Regime::NextSeason => "01-01".to_string(),
        Regime::RestOfSeason { month, day } => {
            let today = format!("{month:02}-{day:02}");
            protocol
                .cuts
                .iter()
                .filter(|(l, _)| *l != "01-01" && l.as_str() <= today.as_str())
                .map(|(l, _)| l.clone())
                .max()
                .unwrap_or_else(|| "01-01".to_string())
        }
    };
    let scored = label_folds(&label, folds)?;
    let residuals: Vec<Residual> =
        scored.iter().rev().take(HISTORY_SEASONS_BATTING).flat_map(|f| f.residuals.iter().copied()).collect();
    if residuals.is_empty() { None } else { Some(IntervalModel::fit(&residuals, crate::intervals::N_MIN)) }
}

/// Refit the interval model for the pitching artifact from a completed walk-forward
/// ([`PitchingFold`]s; same warm-up rule, wider rolling window).
pub fn fit_pitching_intervals_for(
    regime: Regime,
    folds: &[PitchingFold],
    _protocol: &Protocol,
) -> Option<IntervalModel> {
    let label = match regime {
        Regime::NextSeason => "01-01".to_string(),
        // Pitching scores the preseason regime only (T15/T11); a rest-of-season ERA model is
        // a later ticket, so the in-season pick falls through to 01-01.
        Regime::RestOfSeason { .. } => "01-01".to_string(),
    };
    let scored: Vec<&PitchingFold> = folds.iter().filter(|f| f.cut == label).collect();
    if scored.len() < 3 {
        return None;
    }
    let residuals: Vec<Residual> =
        scored.iter().rev().take(HISTORY_SEASONS_PITCHING).flat_map(|f| f.residuals.iter().copied()).collect();
    if residuals.is_empty() { None } else { Some(IntervalModel::fit(&residuals, crate::intervals::N_MIN)) }
}

/// The folds of one cut label in walk order (ascending season, as the harness emits them).
/// Folds are scored at most once per label per season, so stable on the harness's emission order.
fn label_folds<'a>(label: &str, folds: &'a [Fold]) -> Option<Vec<&'a Fold>> {
    let mut fs: Vec<&Fold> = folds.iter().filter(|f| f.cut == label).collect();
    if fs.len() < 3 {
        return None;
    }
    fs.sort_by_key(|f| f.season);
    Some(fs)
}

// =====================================================================
// Pitching walk-forward (spec §4.4). Scoring metric is **ERA** — the corpus carries earned runs
// for every pitching line, so the ER variant of the spec's "ERA where the corpus has them" is the
// scored metric (an eprintln names the metric at run time). The rate and playing-time scoring,
// the re-centring and the interval/gate machinery are the same shape as the batting side, with
// innings in place of PA: the generalised [`crate::scores::SeasonFold`] carries fractional
// weights, so no pitching-specific scorer exists.

/// One scored pitching fold row per player.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchingFoldRow {
    pub season: u16,
    pub cut: String,
    pub player_id: String,
    /// Projected ERA (the [`crate::scores::SeasonFold`] field names read "wOBA/PA"; on this path
    /// they carry ERA/IP).
    pub projected_woba: f64,
    pub projected_pa: f64,
    pub bucket: Bucket,
    pub actual_woba: f64,
    pub actual_pa: f64,
    /// Error = actual − projected (before re-centring).
    pub err: f64,
}

/// One pitching fold's result.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchingFold {
    pub season: u16,
    pub cut: String,
    pub rows: Vec<PitchingFoldRow>,
    pub score: SeasonScore,
    /// The fold's own standardised residuals (same shape as `Fold::residuals`).
    pub residuals: Vec<Residual>,
}

/// Run the walk-forward protocol for a pitching model over the corpus, scoring ERA and IP.
pub fn walk_forward_pitching(
    model: &dyn crate::marcel::PitchingModel,
    corpus: &Corpus,
    protocol: &Protocol,
) -> (Vec<PitchingFold>, ScoreReport) {
    eprintln!("walk_forward_pitching: scoring ERA (the corpus carries earned runs for every line)");
    let constants = crate::constants::Constants::default();
    let mut folds = Vec::new();
    let mut residual_history: Vec<(u16, Vec<Residual>)> = Vec::new();

    for &season in &protocol.seasons {
        let as_of_season = season - 1;
        let training: Vec<&crate::corpus::PitchingLine> =
            corpus.pitching.iter().filter(|l| l.season <= as_of_season && l.season > as_of_season - 3).collect();
        if training.is_empty() {
            continue;
        }
        let projections = model.project_pitching(corpus, &constants, season, as_of_season);
        if projections.is_empty() {
            continue;
        }

        // Actuals: ERA per stint-merged line, innings as the weight.
        let mut actuals: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for line in corpus.pitching.iter().filter(|l| l.season == season) {
            if line.ip_outs == 0 {
                continue;
            }
            let ip = line.ip_outs as f64 / 3.0;
            let entry = actuals.entry(line.player_id.clone()).or_insert((0.0, 0.0));
            entry.0 += 9.0 * line.er as f64 / ip * ip;
            entry.1 += ip;
        }
        for v in actuals.values_mut() {
            if v.1 > 0.0 {
                v.0 /= v.1;
            }
        }

        // Floor: IP-weighted league ERA of the last training season restricted to ≥ 50 IP arms.
        let mut floor_era = None;
        for train_season in (1998..season).rev() {
            let mut num = 0.0;
            let mut den = 0.0;
            for line in corpus.pitching.iter().filter(|l| l.season == train_season && l.ip_outs >= 150) {
                let ip = line.ip_outs as f64 / 3.0;
                num += 9.0 * line.er as f64 / ip * ip;
                den += ip;
            }
            if den > 0.0 {
                floor_era = Some(num / den);
                break;
            }
        }
        let floor_era = floor_era.unwrap_or(4.30);

        let recent: Vec<Residual> = residual_history
            .iter()
            .rev()
            .take(HISTORY_SEASONS_PITCHING)
            .flat_map(|(_, rs)| rs.iter().copied())
            .collect();
        let intervals = if folds.len() < 3 || recent.is_empty() {
            None
        } else {
            Some(IntervalModel::fit(&recent, crate::intervals::N_MIN))
        };

        let season_projections: Vec<(String, f64, f64, Bucket)> =
            projections.iter().map(|(id, f)| (id.clone(), f.era, f.ip, f.bucket)).collect();
        let fold_in = scores::SeasonFold {
            season,
            cut: "01-01".to_string(),
            projections: season_projections.clone(),
            actuals: &actuals,
            floor_woba: floor_era,
            floor_pa: 100.0,
            intervals: intervals.as_ref(),
            weight_basis: WeightBasis::Projected,
            pa_to_date: None,
        };
        let score = scores::score_season(&fold_in);

        let mut rows = Vec::with_capacity(season_projections.len());
        for (id, pe, pip, bucket) in &season_projections {
            let (ae, aip) = match actuals.get(id) {
                Some(x) => *x,
                None => (floor_era, 100.0),
            };
            let err = ae - pe;
            rows.push(PitchingFoldRow {
                season,
                cut: "01-01".to_string(),
                player_id: id.clone(),
                projected_woba: *pe,
                projected_pa: *pip,
                bucket: *bucket,
                actual_woba: ae,
                actual_pa: aip,
                err,
            });
        }
        let aligned = scores::align_rows(&season_projections, &actuals, floor_era, 100.0);
        let residuals: Vec<Residual> = aligned
            .iter()
            .filter(|r| r.actual_pa > 0.0 && !r.phantom)
            .map(|r| Residual { z: (r.actual_woba - r.centred) * r.proj_pa.max(1.0).sqrt(), bucket: r.bucket })
            .collect();
        folds.push(PitchingFold { season, cut: "01-01".to_string(), rows, score, residuals: residuals.clone() });
        residual_history.push((season, residuals));
    }

    let seasons: Vec<SeasonScore> = folds.iter().map(|f| f.score.clone()).collect();
    let mut pooled_num = 0.0;
    let mut pooled_den = 0.0;
    let mut ip_num = 0.0;
    let mut ip_n = 0.0;
    for f in &folds {
        for r in &f.rows {
            pooled_num += r.actual_pa * r.err.abs();
            pooled_den += r.actual_pa;
            ip_num += (r.projected_pa - r.actual_pa).abs();
            ip_n += 1.0;
        }
    }
    // All pitching folds are preseason (01-01) today.
    let report = ScoreReport {
        pooled_mae: if pooled_den > 0.0 { pooled_num / pooled_den } else { 0.0 },
        pooled_pa_mae: if ip_n > 0.0 { ip_num / ip_n } else { 0.0 },
        gate: scores::gate(&seasons),
        // The pitching path scores the preseason regime only (T15/T11); an in-season ERA fold
        // is a later ticket (peer of the batting in-season loop).
        in_season_gate: None,
        preseason_mae: if pooled_den > 0.0 { Some(pooled_num / pooled_den) } else { None },
        in_season_mae: None,
        seasons,
    };
    (folds, report)
}
