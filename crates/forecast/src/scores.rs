//! Scores and the calibration gate (DR-3.12, DR-3.12a).
//!
//! * **Rate score**: PA-weighted MAE of wOBA per season, re-centred — each system's projections
//!   are shifted so its PA-weighted league mean of projected rates equals the mean actual league
//!   rate of the population. All players in the population are included; players without a
//!   projection are scored at the **floor** (default Whiffs: the PA-weighted league mean wOBA of
//!   the last training season, restricted to players with ≥ 100 PA).
//! * **Playing time**: PA MAE, missing players at 100 PA (Whiffs convention).
//! * **Coverage**: PICP80 per (season, bucket) with `(picp, n)`, plus MIS (mean interval score:
//!   the pinball loss averaged over the 7 fitted levels and over the population).
//! * **Gate**: 0.80 inside the Wilson 95% interval of observed coverage in every scored cell of
//!   the last three seasons, after merges.

use crate::intervals::{Bucket, IntervalModel, LEVELS, WeightBasis};
use std::collections::BTreeMap;

/// Actual wOBA of a batting line under the given season weights.
pub fn actual_woba(line: &crate::corpus::BattingLine, w: &crate::constants::Weights) -> f64 {
    crate::marcel::woba_of(&crate::marcel::rates_of(line), w)
}

/// Per-(season, bucket) coverage cell.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PicpCell {
    pub picp: f64,
    pub n: usize,
}

/// One season's score row for one model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeasonScore {
    pub season: u16,
    /// Protocol cut label ("01-01" preseason, "05-01".."09-01" in-season).
    pub cut: String,
    /// PA-weighted MAE of re-centred wOBA.
    pub mae: f64,
    /// PA MAE (missing at 100 PA).
    pub pa_mae: f64,
    /// Coverage cells keyed by bucket (only scored — non-empty — cells).
    pub picp80: BTreeMap<Bucket, PicpCell>,
    /// Mean interval score across players with intervals.
    pub mis: Option<f64>,
    /// Wilson-gate pass for this season (all scored cells of this season pass).
    pub gate_pass: bool,
}

/// Full score report for one fit.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScoreReport {
    pub seasons: Vec<SeasonScore>,
    /// Pooled PA-weighted MAE across all scored seasons.
    pub pooled_mae: f64,
    /// Pooled PA MAE.
    pub pooled_pa_mae: f64,
    /// CI gate: Wilson check over the last three **preseason** (01-01) scored folds. In-season
    /// folds score separately (user story 25: "in-season cuts scored separately from
    /// preseason") because the ROS regime has its own sample-size profile; mixing the two
    /// makes the gate a property of the pooled timeline, not of the model (2026-10-22
    /// finding: 2025/Low over-covered once in-season folds entered the pooled tail).
    pub gate: GateResult,
    /// Informational Wilson check over the last three in-season scored folds (`None` when
    /// fewer than three are scored — the gate is not enforced on it).
    pub in_season_gate: Option<GateResult>,
    /// Pooled PA-weighted MAE of the preseason folds only.
    pub preseason_mae: Option<f64>,
    /// Pooled PA-weighted MAE of the in-season folds (`None` when none are scored).
    pub in_season_mae: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GateResult {
    pub pass: bool,
    /// Cells checked, with (season, bucket, picp, n, wilson_low, wilson_high, pass).
    pub cells: Vec<GateCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GateCell {
    pub season: u16,
    pub bucket: Bucket,
    pub picp: f64,
    pub n: usize,
    pub wilson_low: f64,
    pub wilson_high: f64,
    pub pass: bool,
}

/// Wilson 95% interval of an observed proportion.
pub fn wilson95(k: usize, n: usize) -> (f64, f64) {
    let nf = n as f64;
    if nf <= 0.0 {
        return (0.0, 1.0);
    }
    let p = k as f64 / nf;
    let z = 1.959_963_984_540_054_f64; // 95%
    let z2 = z * z;
    let centre = (p + z2 / (2.0 * nf)) / (1.0 + z2 / nf);
    let half = (z / (1.0 + z2 / nf)) * (p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)).sqrt();
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

/// Inputs for scoring one season's fold.
pub struct SeasonFold<'a> {
    pub season: u16,
    /// Protocol cut label: "01-01" preseason, "05-01".."09-01" in-season.
    pub cut: String,
    /// (player_id, projected wOBA, projected PA, bucket) for every projected player.
    pub projections: Vec<(String, f64, f64, Bucket)>,
    /// (player_id, actual wOBA, actual PA) for every player with a line in the season.
    pub actuals: &'a BTreeMap<String, (f64, f64)>,
    /// Floor wOBA (Whiffs): league mean of the last training season with ≥ 100 PA.
    pub floor_woba: f64,
    /// Weight a missing player is scored at (100 PA batting; 100 IP pitching — see
    /// [`crate::harness`]).
    pub floor_pa: f64,
    /// Fitted interval model (standardised error quantiles per bucket). `None` during warm-up →
    /// no coverage or MIS scored.
    pub intervals: Option<&'a IntervalModel>,
    /// Which weight standardises the interval residuals on both the fit and the de-standardisation
    /// side ([`crate::intervals::WeightBasis`]). Batting: actual PA; pitching: projected IP;
    /// in-season cuts: [`crate::intervals::WeightBasis::Future`] with
    /// `actual_pa − pa_to_date` (below).
    pub weight_basis: WeightBasis,
    /// Per-player PA already accrued at an in-season cut (`None` on a preseason fold). Required
    /// exactly when [`SeasonFold::weight_basis`] is [`crate::intervals::WeightBasis::Future`] —
    /// a Future basis with `None` fails loudly.
    pub pa_to_date: Option<&'a BTreeMap<String, f64>>,
}

/// One aligned scored row.
pub(crate) struct Row {
    pub id: String,
    pub proj_woba: f64,
    pub proj_pa: f64,
    pub bucket: Bucket,
    pub actual_woba: f64,
    pub actual_pa: f64,
    /// Recentred projection, post-shift.
    pub centred: f64,
    /// True when the actual was floor-substituted (projected player with no season line) — a
    /// phantom row: a convention value, not an observed rate.
    pub phantom: bool,
}

/// Align the scored population (every player with a projection or an appearance in the season;
/// missing projections and missing actuals at the floor) and apply the re-centring shift so the
/// PA-weighted mean of projections equals the mean actual league rate. Shared by
/// [`score_season`] and the harness's residual collection so both see exactly the same rows.
pub(crate) fn align_rows(
    projections: &[(String, f64, f64, Bucket)],
    actuals: &BTreeMap<String, (f64, f64)>,
    floor_woba: f64,
    floor_pa: f64,
) -> Vec<Row> {
    let mut ids: Vec<String> = projections.iter().map(|(id, ..)| id.clone()).collect();
    for id in actuals.keys() {
        ids.push(id.clone());
    }
    ids.sort();
    ids.dedup();

    let mut rows: Vec<Row> = Vec::with_capacity(ids.len());
    for id in &ids {
        let (proj_woba, proj_pa, bucket) = match projections.iter().find(|(pid, ..)| pid == id) {
            Some((_, w, pa, b)) => (*w, *pa, *b),
            None => (floor_woba, floor_pa, Bucket::None),
        };
        let (actual_woba, actual_pa, phantom) = match actuals.get(id) {
            Some((aw, pa)) => (*aw, *pa, false),
            None => (floor_woba, floor_pa, true),
        };
        rows.push(Row { id: id.clone(), proj_woba, proj_pa, bucket, actual_woba, actual_pa, centred: 0.0, phantom });
    }

    // Re-centre so the PA-weighted mean of projections equals the mean actual league rate.
    let mut w_sum = 0.0;
    let mut act_sum = 0.0;
    let mut proj_sum = 0.0;
    for r in &rows {
        w_sum += r.actual_pa;
        act_sum += r.actual_pa * r.actual_woba;
        proj_sum += r.actual_pa * r.proj_woba;
    }
    let shift = if w_sum > 0.0 { act_sum / w_sum - proj_sum / w_sum } else { 0.0 };
    for r in &mut rows {
        r.centred = r.proj_woba + shift;
    }
    rows
}

/// Score one season's fold.
pub fn score_season(fold: &SeasonFold<'_>) -> SeasonScore {
    let rows = align_rows(&fold.projections, fold.actuals, fold.floor_woba, fold.floor_pa);

    // PA-weighted MAE and PA MAE.
    let mut mae_num = 0.0;
    let mut pa_mae_num = 0.0;
    let mut w_total = 0.0;
    let mut mis_sum = 0.0;
    let mut mis_n = 0usize;
    let mut agg: BTreeMap<Bucket, (usize, usize)> = BTreeMap::new();
    for r in &rows {
        mae_num += r.actual_pa * (r.centred - r.actual_woba).abs();
        pa_mae_num += (r.proj_pa - r.actual_pa).abs();
        w_total += r.actual_pa;

        if let Some(q) = fold.intervals.and_then(|m| m.quantiles(r.bucket)) {
            // Rate-conditional coverage: rows without an observed rate (actual weight 0) and
            // phantom rows (floor-substituted actual) are convention rows, not rate draws.
            if r.actual_pa <= 0.0 || r.phantom {
                continue;
            }
            let w = match fold.weight_basis {
                WeightBasis::Projected => r.proj_pa,
                WeightBasis::Actual => r.actual_pa,
                WeightBasis::Future => {
                    // Future sample = actual PA minus PA to date. Fails loudly on a preseason
                    // fold (no pa_to_date map) — there is no future sample to standardise with.
                    let to_date = fold
                        .pa_to_date
                        .unwrap_or_else(|| panic!("WeightBasis::Future used on a preseason fold (no pa_to_date map)"));
                    r.actual_pa - to_date.get(&r.id).copied().unwrap_or(0.0)
                }
            };
            let q = IntervalModel::destandardise(q, w);
            // De-standardised error quantiles e_i; quantile forecast_i = centred + e_i.
            let es = [q.p05, q.p10, q.p25, q.p50, q.p75, q.p90, q.p95];
            let forecast = |e: f64| r.centred + e;

            // MIS: pinball loss averaged over the 7 levels.
            let mut mis = 0.0;
            for (level, e) in LEVELS.iter().zip(es.iter()) {
                let f = forecast(*e);
                mis +=
                    if r.actual_woba >= f { level * (r.actual_woba - f) } else { (1.0 - level) * (f - r.actual_woba) };
            }
            mis_sum += mis / es.len() as f64;
            mis_n += 1;

            // PICP80: the 80% band is (p10, p90).
            let lo = forecast(q.p10);
            let hi = forecast(q.p90);
            let hit = r.actual_woba >= lo - 1e-12 && r.actual_woba <= hi + 1e-12;
            let entry = agg.entry(r.bucket).or_insert((0, 0));
            entry.1 += 1;
            if hit {
                entry.0 += 1;
            }
        }
    }

    let mae = if w_total > 0.0 { mae_num / w_total } else { 0.0 };
    let pa_mae = if !rows.is_empty() { pa_mae_num / rows.len() as f64 } else { 0.0 };
    let picp80: BTreeMap<Bucket, PicpCell> = agg
        .into_iter()
        .filter(|(_, (_, n))| *n > 0)
        .map(|(b, (k, n))| (b, PicpCell { picp: k as f64 / n as f64, n }))
        .collect();

    let gate_pass = picp80.iter().all(|(b, c)| {
        let _ = b;
        let k = (c.picp * c.n as f64).round() as usize;
        let (lo, hi) = wilson95(k, c.n);
        (lo..=hi).contains(&0.80)
    });

    SeasonScore {
        season: fold.season,
        cut: fold.cut.clone(),
        mae,
        pa_mae,
        picp80,
        mis: if mis_n > 0 { Some(mis_sum / mis_n as f64) } else { None },
        gate_pass,
    }
}

/// Gate over the last three scored seasons: every scored cell must contain 0.80 in its Wilson
/// 95% interval.
pub fn gate(scores: &[SeasonScore]) -> GateResult {
    let scored: Vec<&SeasonScore> = scores.iter().filter(|s| !s.picp80.is_empty()).collect();
    let last: Vec<&SeasonScore> = scored.iter().rev().take(3).copied().collect();
    let mut cells = Vec::new();
    for s in &last {
        for (b, c) in &s.picp80 {
            let k = (c.picp * c.n as f64).round() as usize;
            let (lo, hi) = wilson95(k, c.n);
            cells.push(GateCell {
                season: s.season,
                bucket: *b,
                picp: c.picp,
                n: c.n,
                wilson_low: lo,
                wilson_high: hi,
                pass: (lo..=hi).contains(&0.80),
            });
        }
    }
    GateResult { pass: cells.iter().all(|c| c.pass), cells }
}
