//! In-season walk-forward folds (DR-3.31) over the bundled real cut lines
//! (`sabr-data/sample/cut-lines-2001-2025.csv`).

use sabr_forecast::constants::Constants;
use sabr_forecast::corpus::{Corpus, Protocol};
use sabr_forecast::harness::walk_forward;
use sabr_forecast::marcel::{Marcel, Model};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn sample_dir() -> PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sabr-data/sample/lahman")
}

fn cuts_csv() -> PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sabr-data/sample/cut-lines-2001-2025.csv")
}

fn corpus_with_cuts() -> Corpus {
    Corpus::from_lahman_with_cuts(&sample_dir(), Some(&cuts_csv())).expect("corpus with cut lines")
}

/// The protocol's in-season cut labels (05-01..09-01), month/day parsed from the label.
fn in_season_cuts() -> Vec<(String, u8, u8)> {
    Protocol::v1()
        .cuts
        .into_iter()
        .filter(|(label, _)| label != "01-01")
        .map(|(label, _)| {
            let md: Vec<u8> = label.split('-').filter_map(|p| p.parse::<u8>().ok()).collect();
            (label, md[0], md[1])
        })
        .collect()
}

/// The harness's partial selection, rebuilt from the spec: the latest cut-line row per player
/// strictly before the cut (as_of = the day before the cut), for one season.
fn partials_for(
    corpus: &Corpus,
    season: u16,
    cut_m: u8,
    cut_d: u8,
) -> BTreeMap<String, sabr_forecast::corpus::BattingLine> {
    let mut latest: BTreeMap<&str, &sabr_forecast::corpus::CutLine> = BTreeMap::new();
    for cut in corpus.cuts.iter().filter(|c| c.season == season && (c.month, c.day) < (cut_m, cut_d)) {
        match latest.get(cut.player_id.as_str()) {
            Some(prev) if (prev.month, prev.day) >= (cut.month, cut.day) => {}
            _ => {
                latest.insert(&cut.player_id, cut);
            }
        }
    }
    let mut out = BTreeMap::new();
    for cut in latest.values() {
        let (by, bm) = corpus.birth.get(&cut.player_id).copied().unwrap_or((0, 0));
        out.insert(
            cut.player_id.clone(),
            sabr_forecast::corpus::BattingLine {
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
    out
}

/// Loader: the real sample parses to 133,183 lines = header + 133,182 cut-line rows.
#[test]
fn loader_parses_real_sample() {
    let corpus = corpus_with_cuts();
    assert_eq!(corpus.cuts.len(), 133_182, "real cut-lines sample fully parsed");
    // as_of month/day parsed from the date string: every 2024-04-30 row lands (4, 30).
    assert!(corpus.cuts.iter().filter(|c| c.season == 2024 && c.month == 4 && c.day == 30).count() > 900);
}

/// A player with rows before and after a cut only contributes the rows before the cut: the
/// 05-01 fold's partials for 2024 are the 04-30 rows, and the harness's scored fold matches a
/// projection built from exactly those rows.
#[test]
fn partials_are_cut_scoped() {
    let corpus = corpus_with_cuts();
    let constants = Constants::default();
    let protocol = Protocol { seasons: vec![2024], cuts: Protocol::v1().cuts };

    let (folds, _) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let fold = folds.iter().find(|f| f.cut == "05-01").expect("2024 05-01 fold");

    // Hand-built partials: 04-30 rows only. abramcj01's 04-30 PA (119) differs from its 05-31 PA.
    let partials = partials_for(&corpus, 2024, 5, 1);
    let abrac = partials.get("abramcj01").expect("abramcj01 cut row");
    assert_eq!(abrac.pa, 119, "05-01 fold uses the 04-30 row, not the 05-31 row");
    let expected = Marcel.project_in_season(&corpus, &constants, 2024, 2023, &partials);

    for row in &fold.rows {
        if let Some(f) = expected.get(&row.player_id) {
            assert_eq!(row.projected_woba, f.woba, "woba {}", row.player_id);
            // FoldRow PA is the projected season total = PA to date + remaining.
            assert_eq!(row.projected_pa, f.pa_to_date + f.pa, "PA total {}", row.player_id);
        }
    }
}

/// One cut fold: 2024/May-1 has ~1,000 partials; a player with a partial projects
/// pa = remaining (full_pa − pa_to_date, floored at 0); the fold row's PA MAE input is the
/// season total; and the projected rate lands within 0.08 of the player's actual wOBA
/// (hand-checked sanity on a real player — abramcj01; the brief's ohtansh01 has no cut-line
/// rows in the bundled sample, see the report note).
#[test]
fn one_cut_fold_2024_may_1() {
    let corpus = corpus_with_cuts();
    let constants = Constants::default();
    let protocol = Protocol { seasons: vec![2024], cuts: Protocol::v1().cuts };
    let (folds, _) = walk_forward(&Marcel, &corpus, &protocol, &constants);

    let fold = folds.iter().find(|f| f.cut == "05-01").expect("05-01 fold");
    // ~1,200 partials → the fold scores a comparably sized population.
    assert!(fold.rows.len() > 1_000, "{} rows", fold.rows.len());

    let partials = partials_for(&corpus, 2024, 5, 1);
    let expected = Marcel.project_in_season(&corpus, &constants, 2024, 2023, &partials);
    let abrac = partials.get("abramcj01").expect("abramcj01 partial");
    let forecast = expected.get("abramcj01").expect("abramcj01 forecast");
    assert!(forecast.pa_to_date > 0.0);
    // full_pa = 0.5·PA₋₁ + 0.1·PA₋₂ + 200 from the corpus history; projected PA = remaining.
    let pa_hist = |season: u16| {
        corpus
            .batting
            .iter()
            .filter(|l| l.player_id == "abramcj01" && l.season == season)
            .map(|l| l.pa as f64)
            .sum::<f64>()
    };
    let full_pa = 0.5 * pa_hist(2023) + 0.1 * pa_hist(2022) + 200.0;
    assert_eq!(forecast.pa, (full_pa - abrac.pa as f64).max(0.0));

    // The fold row compares against the season ACTUALS; the rate error stays in sanity range.
    let row = fold.rows.iter().find(|r| r.player_id == "abramcj01").expect("abramcj01 fold row");
    assert!(row.err.abs() <= 0.08, "abramcj01 |actual − projected| wOBA = {:.4}", row.err.abs());
}

/// No look-ahead: perturbing every batting line of the target season must leave all in-season
/// projections byte-identical (they read no target-season Lahman lines).
#[test]
fn no_lookahead_in_season() {
    let corpus = corpus_with_cuts();
    let constants = Constants::default();
    let mut tampered = corpus.clone();
    for l in tampered.batting.iter_mut().filter(|l| l.season == 2024) {
        l.h += 7;
        l.pa += 9;
        l.hr += 1;
    }
    for (label, cut_m, cut_d) in in_season_cuts() {
        let partials = partials_for(&corpus, 2024, cut_m, cut_d);
        let base = Marcel.project_in_season(&corpus, &constants, 2024, 2023, &partials);
        let again = Marcel.project_in_season(&tampered, &constants, 2024, 2023, &partials);
        assert_eq!(base, again, "in-season projections for cut {label} must not read target-season lines");
    }
}

/// Determinism: two runs over the same corpus are byte-equal (folds and report).
#[test]
fn determinism() {
    let corpus = corpus_with_cuts();
    let constants = Constants::default();
    let protocol = Protocol { seasons: vec![2022, 2023, 2024], cuts: Protocol::v1().cuts };
    let (folds_a, report_a) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let (folds_b, report_b) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    assert_eq!(folds_a, folds_b);
    assert_eq!(report_a, report_b);
}

/// Budget: the FULL walk-forward protocol (25 seasons × 6 cuts, Marcel batting) over the real
/// corpus with cut lines must stay under 60 s wall in release. Also checks the fold count
/// (25 × 6 = 150) and reports the pooled preseason vs in-season MAE split. Per DR-3.12a the
/// gate may fail on the real data — reported as a finding, not tuned here.
#[test]
fn full_protocol_budget_and_fold_count() {
    let corpus = corpus_with_cuts();
    let constants = Constants::default();
    let protocol = Protocol::v1();
    let t0 = std::time::Instant::now();
    let (folds, report) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let wall = t0.elapsed();

    // 25 seasons × 6 cuts = 150, less 2020's 05-01/06-01/07-01 cuts: the COVID-shortened season
    // has no April–June cut lines at all, so those three folds carry no to-date data and are
    // skipped (as-of correct — nobody had played yet).
    assert_eq!(folds.len(), 147, "25 seasons × 6 cuts, less the 3 skipped 2020 cuts");
    assert_eq!(report.seasons.len(), 147);
    println!("full protocol wall: {:.2}s", wall.as_secs_f64());
    assert!(wall < std::time::Duration::from_secs(60), "wall {:?}", wall);

    // Pooled MAE split: preseason (01-01) vs in-season cuts.
    let split = |label: &str| -> (f64, f64) {
        let mut num = 0.0;
        let mut den = 0.0;
        for f in folds.iter().filter(|f| f.cut == label) {
            for r in &f.rows {
                num += r.actual_pa as f64 * r.err.abs();
                den += r.actual_pa as f64;
            }
        }
        (if den > 0.0 { num / den } else { 0.0 }, den)
    };
    let (pre_mae, pre_w) = split("01-01");
    let mut in_num = 0.0;
    let mut in_den = 0.0;
    for f in folds.iter().filter(|f| f.cut != "01-01") {
        for r in &f.rows {
            in_num += r.actual_pa as f64 * r.err.abs();
            in_den += r.actual_pa as f64;
        }
    }
    println!(
        "pooled preseason MAE {:.5} (w {:.0}); pooled in-season MAE {:.5} (w {:.0}); pooled all {:.5}; PA MAE {:.1}; gate {}",
        pre_mae,
        pre_w,
        if in_den > 0.0 { in_num / in_den } else { 0.0 },
        in_den,
        report.pooled_mae,
        report.pooled_pa_mae,
        report.gate.pass
    );
    // Findings per DR-3.12a: the gate may fail on the real data — reported, not tuned.
    for c in report.gate.cells.iter().filter(|c| !c.pass) {
        println!(
            "gate FAIL: {} {:?} picp={:.3} n={} wilson=({:.3},{:.3})",
            c.season, c.bucket, c.picp, c.n, c.wilson_low, c.wilson_high
        );
    }
}
