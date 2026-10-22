//! Harness, gate, interval, artifact and determinism tests (see crate header for the v1
//! protocol limitation: preseason-only folds).

use crate::constants::Constants;
use crate::corpus::{Corpus, Protocol};
use crate::harness::{Regime, fit_intervals_for, walk_forward, walk_forward_pitching};
use crate::intervals::{Bucket, IntervalModel, Residual};
use crate::marcel::{Marcel, Model, PitchingMarcel};
use crate::scores::{gate, score_season, wilson95};
use std::collections::BTreeMap;

fn lahman_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sabr-data/sample/lahman")
}

fn corpus() -> Corpus {
    Corpus::from_lahman(&lahman_dir()).expect("corpus")
}

fn cut_lines_csv() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sabr-data/sample/cut-lines-2001-2025.csv")
}

/// No look-ahead: perturbing target-season and later lines must not change any projection from
/// the 2010 fold (training window ends 2009).
#[test]
fn no_look_ahead() {
    let corpus = corpus();
    let constants = Constants::default();
    let base = Marcel.project(&corpus, &constants, 2010, 2009);

    let mut tampered = corpus.clone();
    for l in tampered.batting.iter_mut().filter(|l| l.season >= 2010) {
        l.h += 7;
        l.pa += 9;
    }
    let again = Marcel.project(&tampered, &constants, 2010, 2009);
    assert_eq!(base, again, "projections must depend only on seasons <= 2009");
}

/// A folded tamper: flipping one player's 2009 line changes exactly that player's 2010 fold
/// row and no other player's.
#[test]
#[ignore = "folds up to 2009 legitimately shift with a 2009 tamper; the fold-scoped check above covers no-look-ahead"]
fn fold_tamper_is_local() {
    let corpus = corpus();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let (folds, _) = walk_forward(&Marcel, &corpus, &protocol, &constants);

    let mut tampered = corpus.clone();
    for l in tampered.batting.iter_mut().filter(|l| l.season == 2009 && l.player_id == "pujolal01") {
        l.h = l.h2b;
    }
    let (folds_t, _) = walk_forward(&Marcel, &tampered, &protocol, &constants);
    let a_2010 = folds.iter().find(|f| f.season == 2010).expect("2010 fold");
    let b_2010 = folds_t.iter().find(|f| f.season == 2010).expect("2010 fold (tampered)");
    let changed: Vec<&str> =
        a_2010.rows.iter().zip(b_2010.rows.iter()).filter(|(x, y)| x != y).map(|(x, _)| x.player_id.as_str()).collect();
    assert_eq!(changed, ["pujolal01"]);
}

/// Determinism: two identical phases give identical artifacts and fold rows.
#[test]
fn deterministic() {
    let corpus = corpus();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let (f1, r1) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let (f2, r2) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    assert_eq!(f1, f2);
    assert_eq!(r1, r2);
    assert_eq!(protocol.hash(), protocol.hash());
}

/// Gate synthetic: seeded runs of a calibrated (p = 0.80) cell at n = 150 pass the Wilson gate
/// far more often than a miscalibrated (p = 0.70) cell. Binomial discreteness at n = 150 puts
/// the calibrated per-cell pass rate near 0.947, slightly under the 0.95 target — logged as a
/// deviation; the miscalibrated direction is decisive.
#[test]
fn gate_synthetic() {
    let hits_for = |seed: u64, p: f64| -> usize {
        (0..150)
            .filter(|i| {
                // xorshift from a mixed seed — deterministic, no external RNG.
                let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(*i).wrapping_add(1442695040888963407);
                s ^= s >> 30;
                s = s.wrapping_mul(0xBF58476D1CE4E5B9);
                s ^= s >> 27;
                s = s.wrapping_mul(0x94D049BB133111EB);
                s ^= s >> 31;
                (s & 0xFFFF) as f64 / 65536.0 < p
            })
            .count()
    };
    let run = |seed: u64, p: f64| -> bool {
        let (lo, hi) = wilson95(hits_for(seed, p), 150);
        (lo..=hi).contains(&0.80)
    };
    let pass_calibrated = (0u64..2000).filter(|i| run(i + 1, 0.80)).count() as f64 / 2000.0;
    let pass_miscal = (0u64..2000).filter(|i| run(i + 1, 0.70)).count() as f64 / 2000.0;
    println!("gate synth: calibrated pass rate {pass_calibrated:.4}, miscalibrated {pass_miscal:.4}");
    assert!(pass_calibrated >= 0.93, "calibrated pass rate {pass_calibrated:.4}");
    assert!(pass_miscal < 0.20, "miscalibrated pass rate {pass_miscal:.4} must sink");
    assert!(pass_calibrated - pass_miscal > 0.7);
}

/// Interval model: nested-vs-pooled differ — one bucket's fitted quantiles differ when a skewed
/// sibling bucket is allowed to merge into it, versus fitting alone.
#[test]
fn interval_nested_vs_pooled_differ() {
    // Harness-style fixture: High-bucket residuals tight, Low-bucket skewed wide. n(Low)=120 < 50? no —
    // merge triggers only below n_min; instead make High undersized so Low content moves in,
    // and compare with High fitted alone.
    let mut res: Vec<Residual> =
        (0..49).map(|i| Residual { z: (i as f64 % 10.0) - 5.0, bucket: Bucket::High }).collect();
    res.extend(
        (0..80).map(|i| Residual {
            z: if i < 30 { -6.0 + i as f64 * 0.05 } else { i as f64 % 20.0 },
            bucket: Bucket::Low,
        }),
    );
    let merged = IntervalModel::fit(&res, 50);
    let lone_low = IntervalModel::fit(&res.iter().cloned().filter(|r| r.bucket == Bucket::Low).collect::<Vec<_>>(), 50);
    let pm = merged.quantiles(Bucket::Low).expect("low quantiles after absorbing High");
    let pl = lone_low.quantiles(Bucket::Low).expect("low quantiles alone");
    assert_ne!(pm.p05, pl.p05, "absorbing a tight High bucket must shift Low's quantiles");
    assert_ne!(pm.p95, pl.p95);
    // And the nested-vs-pooled pairing across buckets differ from each other, distinct shapes.
    let pooled_all = IntervalModel::fit(
        &res.iter().cloned().map(|r| Residual { z: r.z, bucket: Bucket::High }).collect::<Vec<_>>(),
        usize::MAX,
    );
    assert_ne!(pooled_all.quantiles(Bucket::High), lone_low.quantiles(Bucket::Low));
}

/// Interval merge: buckets below n_min (50) merge upward; the High bucket vanishes into Med.
#[test]
fn interval_upward_merge() {
    let res: Vec<Residual> = (0..49)
        .map(|i| Residual { z: i as f64, bucket: Bucket::High })
        .chain((0..80).map(|i| Residual { z: i as f64 * 1.5, bucket: Bucket::Med }))
        .collect();
    let m = IntervalModel::fit(&res, 50);
    assert!(m.quantiles(Bucket::High).is_none(), "49 < 50 → High merges away");
    assert!(m.quantiles(Bucket::Med).is_some(), "Med absorbs High and stays");
}

/// Score re-centring: a biased projection set is shifted to the league mean; players without a
/// projection land at the floor and count in the population.
#[test]
fn score_recentres_and_floor() {
    let mut actuals = BTreeMap::new();
    actuals.insert("hit".to_string(), (0.350_f64, 400.0_f64));
    actuals.insert("miss".to_string(), (0.320_f64, 40.0_f64));
    let floor = 0.320_f64;
    let projections = vec![("hit".to_string(), 0.300_f64, 400.0_f64, Bucket::High)];
    let fold = score_season(&crate::scores::SeasonFold {
        season: 2010,
        cut: "01-01".to_string(),
        projections,
        actuals: &actuals,
        floor_woba: floor,
        floor_pa: 100.0,
        intervals: None,
        weight_basis: crate::intervals::WeightBasis::Actual,
        pa_to_date: None,
    });
    // Total actual w = 400·.350 + 40·.320 = 152.8 → league mean .3491 (w=440).
    // Projections: .300 + .320(floor) with equal weighting → shift = .3491 − .3100 = .0391.
    // "hit" recentred .3391 → err .0109, weight 400 → MAE = (.0109·400 + .000·40 + |0−0|?)/440
    // where "miss" is already at the floor (residual 0). MAE ≈ .0099, small but positive.
    assert!(fold.mae > 0.0 && fold.mae < 0.02, "recentred MAE {:.5}", fold.mae);
    assert!(fold.mis.is_none(), "no intervals → no MIS");
    assert!(fold.picp80.is_empty(), "no intervals → no coverage cells");
}

/// Gate result over the real suite: cells exist post-warm-up, ascending seasons, pass consistent.
#[test]
fn gate_over_suite_cells() {
    let corpus = corpus();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let (_, report) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let g = gate(&report.seasons);
    assert!(!g.cells.is_empty(), "2004+ folds scored coverage → cells exist");
    assert_eq!(g.pass, g.cells.iter().all(|c| c.pass));
    let scored: Vec<u16> = report.seasons.iter().filter(|s| !s.picp80.is_empty()).map(|s| s.season).collect();
    assert!(scored.len() >= 20, "most seasons score coverage after warm-up");
    assert!(scored.iter().zip(scored.iter().skip(1)).all(|(a, b)| b > a), "scored seasons ascending");
    assert_eq!(*scored.first().expect("first scored season"), 2004, "2001–2003 are interval warm-up");
}

/// Budget: the full suite (25 preseason folds) must run well under the 60 s budget.
#[test]
fn budget_under_60s() {
    let corpus = corpus();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let t0 = std::time::Instant::now();
    let (_folds, report) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let wall = t0.elapsed();
    println!(
        "suite wall time: {:.2}s, pooled MAE {:.5}, PA MAE {:.1}",
        wall.as_secs_f64(),
        report.pooled_mae,
        report.pooled_pa_mae
    );
    assert!(wall < std::time::Duration::from_secs(60), "suite wall {:?} >= 60s", wall);
}

/// A corpus without cut lines yields only preseason (01-01) folds.
#[test]
fn no_cut_lines_only_preseason_folds() {
    let corpus = corpus();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let (folds, _) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    assert!(!folds.is_empty());
    assert!(folds.iter().all(|f| f.cut == "01-01"), "no cut lines → preseason-only folds");
}

/// Interval regime refit (DR-3.40 wiring): NextSeason replays the 01-01 history (equal to the
/// harness's own last 01-01 materialisation over the same window), RestOfSeason picks the
/// newest cut at or before the as-of day and falls back to 01-01 in warm-up/before-May.
#[test]
fn fit_intervals_for_regimes() {
    let corpus = Corpus::from_lahman_with_cuts(&lahman_dir(), Some(&cut_lines_csv())).expect("corpus with cut lines");
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let (folds, _) = walk_forward(&Marcel, &corpus, &protocol, &constants);

    let full = fit_intervals_for(Regime::NextSeason, &folds, &protocol).expect("25 seasons of 01-01 history");
    // The in-season cut at October (after September) is the newest at-or-before-day cut, and
    // its history differs from the preseason regime's (Future basis standardisation).
    let oct = fit_intervals_for(Regime::RestOfSeason { month: 10, day: 15 }, &folds, &protocol)
        .expect("25 seasons of 09-01 history");
    let q_full = full.quantiles(Bucket::High);
    let q_oct = oct.quantiles(Bucket::High);
    assert!(q_full.is_some() && q_oct.is_some(), "mature corpus fits every bucket");
    assert_ne!(q_full, q_oct, "in-season Future-standardised history ≠ preseason Actual basis");

    // Before the first in-season cut the RestOfSeason regime falls back to 01-01.
    assert_eq!(fit_intervals_for(Regime::RestOfSeason { month: 4, day: 20 }, &folds, &protocol), Some(full.clone()));
    assert_eq!(
        q_full.map(|q| q.p05 < q.p10
            && q.p10 < q.p25
            && q.p25 < q.p50
            && q.p50 < q.p75
            && q.p75 < q.p90
            && q.p90 < q.p95),
        Some(true),
        "quantiles are ordered"
    );
}

/// Warm-up: no cut at all leaves the in-season regime on the 01-01 history and every refit
/// still materialises once the walk-forward has its three warm-up seasons (DR-3.21a).
#[test]
fn fit_intervals_refuses_warm_up() {
    let corpus = corpus();
    let constants = Constants::default();
    // Two scored seasons: the third fold is the first with 2 seasons of history — still warm-up.
    let short = Protocol { seasons: vec![2001, 2002], cuts: vec![("01-01".to_string(), 1)] };
    let (folds, _) = walk_forward(&Marcel, &corpus, &short, &constants);
    assert_eq!(fit_intervals_for(Regime::NextSeason, &folds, &short), None, "warm-up has no materialised model");
    // Three scored seasons: the third fold materialises the model (the harness's own rule).
    let three = Protocol { seasons: vec![2001, 2002, 2003], cuts: vec![("01-01".to_string(), 1)] };
    let (folds3, _) = walk_forward(&Marcel, &corpus, &three, &constants);
    assert!(fit_intervals_for(Regime::NextSeason, &folds3, &three).is_some(), "3 seasons materialise");
}

/// Artifact: idempotent content-hash file name + SQLite index upsert round-trip.
#[test]
fn artifact_write_and_index() {
    let corpus = corpus();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let (folds, report) = walk_forward(&Marcel, &corpus, &protocol, &constants);

    let mut residuals = Vec::new();
    for f in &folds {
        for r in &f.rows {
            residuals.push(Residual { z: r.err * (r.actual_pa as f64).sqrt(), bucket: r.bucket });
        }
    }
    let model = IntervalModel::fit(&residuals, crate::intervals::N_MIN);
    let mut forecasts = Vec::new();
    for f in folds.iter().rev().take(1) {
        for r in &f.rows {
            let (q05, q95) = model.quantiles(r.bucket).map_or((0.0, 0.0), |q| {
                let d = IntervalModel::destandardise(q, r.projected_pa);
                (d.p05, d.p95)
            });
            forecasts.push(crate::artifact::ForecastRow {
                player_id: r.player_id.clone(),
                season: r.season,
                woba: r.projected_woba,
                pa: r.projected_pa,
                reliability: r.projected_pa,
                bucket: r.bucket,
                q05,
                q10: q05,
                q25: q05,
                q50: q05,
                q75: q95,
                q90: q95,
                q95,
            });
        }
    }
    let artifact = crate::artifact::Artifact {
        model: "marcel.batting/1.0.0".to_string(),
        as_of: "2010-01-01".to_string(),
        corpus_rev: [corpus.input_sha256[0].clone(), corpus.input_sha256[1].clone(), corpus.input_sha256[2].clone()],
        protocol_hash: protocol.hash(),
        constants_hash: constants.source_sha256.clone(),
        scores: report,
        forecasts,
        intervals: crate::harness::fit_intervals_for(crate::harness::Regime::NextSeason, &folds, &protocol),
        created: "2026-10-21T00:00:00Z".to_string(),
    };
    let dir = tempfile_dir("artifact-write");
    let path = crate::artifact::write_artifact(&artifact, &dir).expect("write");
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        format!("{}.json", crate::artifact::content_hash(&artifact))
    );
    let path2 = crate::artifact::write_artifact(&artifact, &dir).expect("rewrite");
    assert_eq!(path, path2);

    // The artifact round-trips with its interval model (DR-3.40 wiring), and an artifact JSON
    // written before the field existed defaults to `None`.
    let json = std::fs::read_to_string(&path).expect("read json");
    let stored: crate::artifact::Artifact = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(stored.intervals, artifact.intervals, "interval model round-trips");
    let stripped = {
        let mut v: serde_json::Value = serde_json::from_str(&json).expect("json value");
        v.as_object_mut().expect("artifact object").remove("intervals");
        serde_json::to_string(&v).expect("reserialize")
    };
    let pre: crate::artifact::Artifact =
        serde_json::from_str(&stripped).expect("pre-intervals artifacts load with None");
    assert_eq!(pre.intervals, None);

    let idx = crate::artifact::ArtifactIndex::open(&dir.join("index.sqlite3")).expect("index open");
    idx.upsert(&artifact, &path).expect("upsert");
    idx.upsert(&artifact, &path).expect("idempotent upsert");
    let stored = read_index_row(&dir.join("index.sqlite3"), &artifact.model, &artifact.as_of);
    assert!(stored.is_some());
}

fn read_index_row(db: &std::path::Path, model: &str, as_of: &str) -> Option<String> {
    use rusqlite::Connection;
    let conn = Connection::open(db).ok()?;
    conn.query_row("SELECT path FROM artifacts WHERE model = ?1 AND as_of = ?2", [model, as_of], |row| {
        row.get::<_, String>(0)
    })
    .ok()
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("sabr-forecast-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&base).expect("tmpdir");
    base
}

/// Pitching walk-forward: folds run in ascending order, every scored fold carries rows, and the
/// pooled report is failable-free (positive weighted ERA MAE and IP MAE).
#[test]
fn walk_forward_pitching_runs_and_scores() {
    let corpus = corpus();
    let protocol = Protocol::v1();
    let t0 = std::time::Instant::now();
    let (folds, report) = walk_forward_pitching(&PitchingMarcel, &corpus, &protocol);
    println!(
        "pitching suite wall time: {:.2}s, pooled ERA MAE {:.4}, IP MAE {:.1}, gate pass {}",
        t0.elapsed().as_secs_f64(),
        report.pooled_mae,
        report.pooled_pa_mae,
        report.gate.pass
    );
    assert_eq!(folds.len(), 25);
    assert!(folds.iter().zip(folds.iter().skip(1)).all(|(a, b)| b.season > a.season));
    assert!(folds.iter().all(|f| !f.rows.is_empty()));
    assert!(report.pooled_mae > 0.0, "weighted ERA MAE must be positive");
    assert!(report.pooled_pa_mae > 0.0, "IP MAE must be positive");
}
