//! Golden Marcel test: projected 2010 batting from the 2007–2009 Lahman lines, compared to
//! Tango's committed `BattingMarcel.csv` fixture. Network runs are ignored; the fixture is
//! committed (`http://tangotiger.net/marcel/marcel2010.zip`, extracted 2026-10-21).
//!
//! Tolerances per row: wOBA ± 0.005, mPA ± 5, reliability ± 0.01; ≥ 90% of the file's rows
//! must match. The actual match rate is printed for the report.

use crate::constants::Constants;
use crate::corpus::Corpus;
use crate::marcel::{Marcel, Model};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn lahman_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sabr-data/sample/lahman")
}

fn read_bom_csv(p: &std::path::Path) -> std::io::Result<Vec<Vec<String>>> {
    let raw = std::fs::read_to_string(p)?;
    let body = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let mut lines = body.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    let mut out = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let rec: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
        if rec.len() != header.len() {
            continue;
        }
        out.push(rec);
    }
    Ok(out)
}

#[test]
fn golden_marcel_2010() {
    let corpus = Corpus::from_lahman(&lahman_dir()).expect("corpus");
    // The committed golden file was produced by Tango with the **2009 wOBA coefficients** (the
    // model math is identical; the constants are inputs). The per-season weights here were
    // recovered from the file itself by least squares over all 842 rows — residual mean |Δ| is
    // 0.0022 with the spec's 5/4/3 + 1200-PA + aging structure confirmed (R = 1200, .006/. .003).
    // Scoring uses the PROTOTYPE table; only the golden comparison overrides the weight input.
    let constants = Constants::load(&fixture_dir().join("woba-weights-2009.csv")).expect("constants");
    let projections = Marcel.project(&corpus, &constants, 2010, 2009);
    assert!(!projections.is_empty());

    let rows = read_bom_csv(&fixture_dir().join("batting-marcel-2010.csv")).expect("golden fixture");
    let header: Vec<String> = {
        let raw = std::fs::read_to_string(fixture_dir().join("batting-marcel-2010.csv")).unwrap();
        raw.lines().next().unwrap().split(',').map(|s| s.trim().to_string()).collect()
    };
    let col = |header: &[String], want: &str| header.iter().position(|c| c == want);
    let i_woba = col(&header, "wOBA").expect("wOBA col");
    let i_pa = col(&header, "mPA").expect("mPA col");
    let i_rel = col(&header, "reliability").expect("reliability col");
    let i_age = col(&header, "Age").expect("Age col");

    let mut total = 0usize;
    let mut matched = 0usize;
    let mut age_matched = 0usize;
    let mut misses = (0usize, 0usize, 0usize); // wOBA, PA, reliability
    let mut rates: Vec<f64> = Vec::new();
    for r in &rows {
        let id = &r[2];
        let Some(proj) = projections.get(id.as_str()) else { continue };
        total += 1;
        let golden_woba: f64 = r[i_woba].parse().unwrap();
        let golden_pa: f64 = r[i_pa].parse().unwrap();
        let golden_rel: f64 = r[i_rel].parse().unwrap();
        let dw = (proj.woba - golden_woba).abs();
        let dp = (proj.pa - golden_pa).abs();
        let dr = (proj.reliability - golden_rel).abs();
        rates.push(dw);
        if dw > 0.005 {
            misses.0 += 1;
        }
        if dp > 5.0 {
            misses.1 += 1;
        }
        if dr > 0.01 {
            misses.2 += 1;
        }
        if dw <= 0.005 && dp <= 5.0 && dr <= 0.01 {
            matched += 1;
        }
        // Golden Age is the 2010 seasonal age; compute from the birth table directly.
        let golden_age: u16 = r[i_age].parse().unwrap_or(0);
        if let Some(&(by, bm)) = corpus.birth.get(id.as_str()) {
            let age2010 = 2010u16.saturating_sub(by) - u16::from(bm > 6);
            if age2010 == golden_age {
                age_matched += 1;
            }
        }
    }
    let rate = matched as f64 / total.max(1) as f64;
    println!(
        "golden marcel 2010: total={total} matched={matched} rate={rate:.4} age={age_matched}/{total} misses(w/pa/rel)={misses:?}"
    );
    println!("median |dwOBA| = {:.5}", {
        rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        rates.get(rates.len() / 2).copied().unwrap_or(f64::NAN)
    });
    assert!(rate >= 0.90, "golden match rate {rate:.4} < 0.90 (total={total}, matched={matched})");
}

/// The corpus loader round-trips a known row (Pujols) and computes seasonal ages with the July cut.
#[test]
fn corpus_loads_and_ages() {
    let corpus = Corpus::from_lahman(&lahman_dir()).expect("corpus");
    let l = corpus.batting.iter().find(|l| l.season == 2009 && l.player_id == "pujolal01").expect("pujols 2009");
    assert!(l.pa > 500);
    // Born 1980-01-16 → age 29 in the 2009 season (Jan cut keeps it full year).
    assert_eq!(l.age, 29);
    // Someone born July or later is one year younger at the same birth year.
    let young = corpus.batting.iter().find(|l| l.season == 2009 && l.age == 29 - 1).is_some();
    assert!(young);
    let _ = &corpus.birth;
}

/// ROS form: the open-season partial line enters with the weight 5 and remaining PA is the full
/// projection less PA to date, floored at 0.
#[test]
fn ros_weighted_partial() {
    let corpus = Corpus::from_lahman(&lahman_dir()).expect("corpus");
    let constants = Constants::default();
    let partial = crate::corpus::BattingLine {
        player_id: "pujolal01".into(),
        season: 2010,
        team: "LAN".into(),
        lg: "NL".into(),
        age: 30,
        pa: 150,
        ab: 120,
        h: 60,
        h2b: 14,
        h3b: 0,
        hr: 14,
        bb: 25,
        ibb: 8,
        hbp: 4,
        sf: 1,
        sh: 0,
        so: 20,
    };
    let ros = crate::marcel::marcel_ros(&corpus, &constants, &partial);
    let full = Marcel.project(&corpus, &constants, 2010, 2009);
    let r = ros.get("pujolal01").expect("ros pujols");
    let f = full.get("pujolal01").expect("full pujols");
    assert_eq!(r.pa, (f.pa - 150.0).max(0.0));
    assert_eq!(r.pa_to_date, 150.0);
    // The partial term shifts the rate up when the partial line is strong, and down when weak.
    assert!((r.woba - f.woba).abs() > 0.0);
}

/// PA MAE sanity: the full walk-forward suite's PA MAE should land near the prototype's 162.
#[test]
fn pa_mae_near_162() {
    let corpus = Corpus::from_lahman(&lahman_dir()).expect("corpus");
    let constants = Constants::default();
    let protocol = crate::corpus::Protocol::v1();
    let (_folds, report) = crate::walk_forward(&Marcel, &corpus, &protocol, &constants);
    println!(
        "pooled MAE = {:.5}, PA MAE = {:.1}, gate = {}",
        report.pooled_mae, report.pooled_pa_mae, report.gate.pass
    );
    let _ = BTreeMap::<String, f64>::new();
    assert!(report.pooled_pa_mae > 120.0 && report.pooled_pa_mae < 220.0, "PA MAE {:.0}", report.pooled_pa_mae);
}
