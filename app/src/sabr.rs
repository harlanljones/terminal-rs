//! The `sabr` CLI subcommands on the terminal binary (spec §4.8; DR-3.4, DR-3.27, DR-3.32, DR-3.37).
//!
//! Dispatch happens before the GUI path: `terminal-rs sabr <SUB> [flags]`. Subcommands:
//!
//! * `fit --model marcel [--as-of LABEL]` — fits batting + pitching Marcel over the bundled
//!   corpus, writes the content-hashed artifact per role into the artifact store with a SQLite
//!   registry index, and prints the content hashes plus the scores/gate summary.
//! * `backtest [--model marcel]` — walk-forward suite for both roles; prints the per-season MAE
//!   table, the gate verdict and the wall time; exits non-zero when the gate fails (DR-3.7).
//! * `report <ENTITY> [--out FILE]` — Markdown one-pager (projection, interval, bucket,
//!   backtest score, as-of, model version, attributions) for the bundled player from the latest
//!   artifact; `--out` writes a file and a `.html` extension renders HTML instead (DR-3.32).
//! * `refresh [--scope all|tracked]` — refits tracked entities plus the bundled player from the
//!   bundled corpus (no network fetch in this wave — the Savant/Stats-API/Chadwick sources are
//!   Wave 8); scope `all` additionally logs `network sources not yet wired` (DR-3.27).
//! * `build-corpus --ev-dir DIR --out DIR` — runs `sabr_retro::build_bundle` (cut lines +
//!   reconciliation reports) over the given event files against the bundled Lahman set (DR-3.37).
//! * `constants` / `trim-register` — run the bundled set's manifest verification and print the
//!   hashes; both only *run* the already-landed generators (build_bundle / verify).
//!
//! Test/dev overrides: `SABR_SAMPLE_DIR` (bundled sample root), `SABR_ARTIFACT_DIR` (artifact
//! store) and `SABR_LAHMAN_DIR` (Lahman directory for `build-corpus` reconciliation).

use std::path::PathBuf;
use std::time::Instant;

use sabr_data::Attribution;
use sabr_forecast::artifact::{Artifact, ArtifactIndex, ForecastRow, content_hash, write_artifact};
use sabr_forecast::corpus::{Corpus, Protocol};
use sabr_forecast::harness::{walk_forward, walk_forward_pitching};
use sabr_forecast::intervals::{Bucket, IntervalModel, N_MIN, Residual};
use sabr_forecast::marcel::{Marcel, Model, PitchingMarcel, PitchingModel};
use sabr_forecast::scores::ScoreReport;

const BATTING_MODEL: &str = "marcel.batting/1.0.0";
const PITCHING_MODEL: &str = "marcel.pitching/1.0.0";

const USAGE: &str = "usage: terminal-rs sabr <SUBCOMMAND>

subcommands:
  fit --model marcel [--as-of LABEL]
  backtest [--model marcel]
  report <ENTITY> [--out FILE]
  refresh [--scope all|tracked]
  build-corpus --ev-dir DIR --out DIR
  constants
  trim-register
  smoke [--frames N]        headless UI pass: no window, no display server";

/// Root directories for the CLI: the bundled sample set, the Lahman set and the artifact store.
pub struct CliPaths {
    pub sample: PathBuf,
    pub artifacts: PathBuf,
    pub lahman: PathBuf,
}

impl CliPaths {
    /// Resolve from the environment; the bundled sample defaults to the working checkout.
    pub fn from_env() -> Self {
        Self::from_dirs(&sabr_data::xdg::XdgDirs::from_env())
    }

    /// The same resolution from explicit dirs (the worker's test seam).
    pub fn from_dirs(dirs: &sabr_data::xdg::XdgDirs) -> Self {
        let sample = std::env::var("SABR_SAMPLE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sabr-data/sample"));
        let artifacts = match std::env::var("SABR_ARTIFACT_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => dirs.data.join("sabr-rs").join("artifacts"),
        };
        let lahman = std::env::var("SABR_LAHMAN_DIR").map(PathBuf::from).unwrap_or_else(|_| sample.join("lahman"));
        CliPaths { sample, artifacts, lahman }
    }
}

fn load_corpus(paths: &CliPaths) -> anyhow::Result<Corpus> {
    // With the bundled in-season cut lines present (DR-3.37) the walk-forward runs the full
    // Q1+May..Sep protocol; without them it degrades to preseason-only (behaviour pre-T18).
    let cuts = paths.sample.join("cut-lines-2001-2025.csv");
    if cuts.is_file() {
        return Corpus::from_lahman_with_cuts(&paths.lahman, Some(&cuts))
            .map_err(|e| anyhow::anyhow!("loading Lahman corpus + cut lines from {}: {e}", paths.lahman.display()));
    }
    Corpus::from_lahman(&paths.lahman)
        .map_err(|e| anyhow::anyhow!("loading Lahman corpus from {}: {e}", paths.lahman.display()))
}

fn open_index(paths: &CliPaths) -> anyhow::Result<ArtifactIndex> {
    std::fs::create_dir_all(&paths.artifacts)?;
    let path = paths.artifacts.join("index.sqlite3");
    ArtifactIndex::open(&path).map_err(|e| anyhow::anyhow!("opening artifact index {}: {e}", path.display()))
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn check_model(rest: &[String], usage: &str) -> anyhow::Result<()> {
    let model = flag(rest, "--model").unwrap_or("marcel");
    if model != "marcel" {
        anyhow::bail!("unknown model {model:?}: only `marcel` is wired in this wave\n{usage}");
    }
    Ok(())
}

/// Entry: dispatches a `terminal-rs sabr ...` invocation, returning the process exit code.
pub fn run(argv: &[String]) -> i32 {
    let t0 = Instant::now();
    let Some((sub, rest)) = argv.split_first() else {
        eprintln!("{USAGE}");
        return 2;
    };
    let paths = CliPaths::from_env();
    let result: anyhow::Result<i32> = match sub.as_str() {
        "fit" => cmd_fit(&paths, rest, t0),
        "backtest" => cmd_backtest(&paths, rest, t0),
        "report" => cmd_report(&paths, rest),
        "refresh" => cmd_refresh(&paths, rest, t0),
        "build-corpus" => cmd_build_corpus(&paths, rest),
        "constants" => cmd_constants(&paths),
        "trim-register" => cmd_trim_register(&paths),
        "smoke" => cmd_smoke(rest),
        other => {
            eprintln!("unknown subcommand {other:?}\n{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("sabr error: {e}");
            1
        }
    }
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// De-standardised quantile band for one player, as flat (q05..q95) wOBA-space offsets.
fn band(model: &IntervalModel, bucket: Bucket, scale: f64) -> [f64; 7] {
    match model.quantiles(bucket) {
        Some(q) => {
            let d = IntervalModel::destandardise(q, scale);
            [d.p05, d.p10, d.p25, d.p50, d.p75, d.p90, d.p95]
        }
        None => [0.0; 7],
    }
}

fn score_summary(report: &ScoreReport, label: &str) -> Vec<String> {
    let role = if label.contains("ERA") { "pitching" } else { "batting" };
    let mut out = vec![format!("{label} pooled MAE={:.4} (gate {})", report.pooled_mae, gate_word(report.gate.pass))];
    for s in &report.seasons {
        out.push(format!(
            "  {role} season {}: mae={:.4} pa_mae={:.1} gate={}",
            s.season,
            s.mae,
            s.pa_mae,
            if s.gate_pass { "pass" } else { "FAIL" },
        ));
    }
    out
}

/// Fit both roles, write both artifacts + index rows; returns the printed lines.
pub fn fit_all(paths: &CliPaths, as_of: &str) -> anyhow::Result<Vec<String>> {
    let corpus = load_corpus(paths)?;
    let constants = sabr_forecast::constants::Constants::default();
    let protocol = Protocol::v1();
    let index = open_index(paths)?;
    let mut out = Vec::new();

    // Batting: fold residuals feed the interval model; forecasts are the final-season Marcel
    // projections (2026 from 2003..=2025 history), scored by the walk-forward suite.
    let (folds, report) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let residuals: Vec<Residual> = folds
        .iter()
        .flat_map(|f| f.rows.iter().map(|r| Residual { z: r.err * (r.actual_pa as f64).sqrt(), bucket: r.bucket }))
        .collect();
    let imodel = IntervalModel::fit(&residuals, N_MIN);
    let projections = Marcel.project(&corpus, &constants, 2026, 2025);
    let forecasts: Vec<ForecastRow> = projections
        .iter()
        .map(|(id, f)| {
            let q = band(&imodel, f.bucket, f.pa);
            ForecastRow {
                player_id: id.clone(),
                season: 2026,
                woba: f.woba,
                pa: f.pa,
                reliability: f.reliability,
                bucket: f.bucket,
                q05: q[0],
                q10: q[1],
                q25: q[2],
                q50: q[3],
                q75: q[4],
                q90: q[5],
                q95: q[6],
            }
        })
        .collect();
    let batting = Artifact {
        model: BATTING_MODEL.to_string(),
        as_of: as_of.to_string(),
        corpus_rev: corpus.input_sha256.clone(),
        protocol_hash: protocol.hash(),
        constants_hash: constants.source_sha256.clone(),
        scores: report,
        forecasts,
        intervals: Some(imodel),
        created: chrono_now(),
    };
    out.extend(store_artifact(&batting, paths, &index)?);
    out.extend(score_summary(&batting.scores, "batting wOBA"));

    // Pitching: same machinery through the pitching harness (ERA/IP scoring).
    let (pfolds, preport) = walk_forward_pitching(&PitchingMarcel, &corpus, &protocol);
    let residuals: Vec<Residual> = pfolds
        .iter()
        .flat_map(|f| f.rows.iter().map(|r| Residual { z: r.err * r.actual_pa, bucket: r.bucket }))
        .collect();
    let pmodel = IntervalModel::fit(&residuals, N_MIN);
    let pprojections = PitchingMarcel.project_pitching(&corpus, &constants, 2026, 2025);
    let pforecasts: Vec<ForecastRow> = pprojections
        .iter()
        .map(|(id, f)| {
            let q = band(&pmodel, f.bucket, f.ip);
            // On the pitching artifact the woba column carries ERA and the pa column IP
            // (the same convention as the pitching harness rows).
            ForecastRow {
                player_id: id.clone(),
                season: 2026,
                woba: f.era,
                pa: f.ip,
                reliability: f.reliability,
                bucket: f.bucket,
                q05: q[0],
                q10: q[1],
                q25: q[2],
                q50: q[3],
                q75: q[4],
                q90: q[5],
                q95: q[6],
            }
        })
        .collect();
    let pitching = Artifact {
        model: PITCHING_MODEL.to_string(),
        as_of: as_of.to_string(),
        corpus_rev: corpus.input_sha256.clone(),
        protocol_hash: protocol.hash(),
        constants_hash: constants.source_sha256.clone(),
        scores: preport,
        forecasts: pforecasts,
        intervals: Some(pmodel),
        created: chrono_now(),
    };
    out.extend(store_artifact(&pitching, paths, &index)?);
    out.extend(score_summary(&pitching.scores, "pitching ERA"));
    Ok(out)
}

/// Write + index one artifact; returns the printed lines (path, hash).
fn store_artifact(artifact: &Artifact, paths: &CliPaths, index: &ArtifactIndex) -> anyhow::Result<Vec<String>> {
    let path = write_artifact(artifact, &paths.artifacts)?;
    index.upsert(artifact, &path).map_err(|e| anyhow::anyhow!("indexing artifact {}: {e}", path.display()))?;
    Ok(vec![
        format!("artifact {} written to {}", artifact.model, path.display()),
        format!("artifact hash (content): {}", content_hash(artifact)),
    ])
}

fn cmd_fit(paths: &CliPaths, rest: &[String], t0: Instant) -> anyhow::Result<i32> {
    check_model(rest, "usage: sabr fit --model marcel [--as-of LABEL]")?;
    let as_of = flag(rest, "--as-of").unwrap_or("2026-preseason").to_string();
    for line in fit_all(paths, &as_of)? {
        println!("{line}");
    }
    println!("fit wall time: {:?}", t0.elapsed());
    Ok(0)
}

fn cmd_backtest(paths: &CliPaths, rest: &[String], t0: Instant) -> anyhow::Result<i32> {
    check_model(rest, "usage: sabr backtest [--model marcel]")?;
    let corpus = load_corpus(paths)?;
    let constants = sabr_forecast::constants::Constants::default();
    let protocol = Protocol::v1();

    let (folds, batting) = walk_forward(&Marcel, &corpus, &protocol, &constants);
    let (pfolds, pitching) = walk_forward_pitching(&PitchingMarcel, &corpus, &protocol);

    println!("season   batting MAE  batting PA-MAE   pitching ERA-MAE  pitching IP-MAE  gate");
    for season in protocol
        .seasons
        .iter()
        .filter(|s| folds.iter().any(|f| f.season == **s) || pfolds.iter().any(|f| f.season == **s))
    {
        let b = folds.iter().find(|f| f.season == *season);
        let p = pfolds.iter().find(|f| f.season == *season);
        println!(
            "{season}  {:>12.4}  {:>14.1}  {:>16.4}  {:>15.1}  {}",
            b.map_or(f64::NAN, |f| f.score.mae),
            b.map_or(f64::NAN, |f| f.score.pa_mae),
            p.map_or(f64::NAN, |f| f.score.mae),
            p.map_or(f64::NAN, |f| f.score.pa_mae),
            if b.is_none_or(|f| f.score.gate_pass) && p.is_none_or(|f| f.score.gate_pass) { "pass" } else { "FAIL" },
        );
    }
    println!("batting  pooled wOBA MAE = {:.4}  PA MAE = {:.1}", batting.pooled_mae, batting.pooled_pa_mae);
    if let (Some(pre), Some(ins)) = (batting.preseason_mae, batting.in_season_mae) {
        // User story 25: in-season cuts score separately. The CI gate is the preseason one.
        println!("batting  preseason MAE = {:.4}  in-season MAE = {:.4}", pre, ins);
    }
    println!("pitching pooled ERA MAE = {:.4}  IP MAE = {:.1}", pitching.pooled_mae, pitching.pooled_pa_mae);
    println!(
        "calibration gate (80% PICP, Wilson 95%, last 3 PRESEASON seasons): batting {}, pitching {}",
        gate_word(batting.gate.pass),
        gate_word(pitching.gate.pass)
    );
    if let Some(g) = &batting.in_season_gate {
        println!("in-season informational gate (not enforced, DR-3.31/story 25): {}", gate_word(g.pass));
    }
    println!("backtest wall time: {:?}", t0.elapsed());
    let pass = batting.gate.pass && pitching.gate.pass;
    println!("gate verdict: {}", if pass { "PASS" } else { "FAIL" });
    for cell in batting.gate.cells.iter().filter(|c| !c.pass) {
        println!(
            "  batting gate cell: {} {:?} picp={:.3} n={} wilson=({:.3},{:.3})",
            cell.season, cell.bucket, cell.picp, cell.n, cell.wilson_low, cell.wilson_high
        );
    }
    for cell in pitching.gate.cells.iter().filter(|c| !c.pass) {
        println!(
            "  pitching gate cell: {} {:?} picp={:.3} n={} wilson=({:.3},{:.3})",
            cell.season, cell.bucket, cell.picp, cell.n, cell.wilson_low, cell.wilson_high
        );
    }
    if !pass {
        eprintln!("gate diagnostics above; exit non-zero per DR-3.7");
    }
    Ok(if pass { 0 } else { 1 })
}

fn gate_word(pass: bool) -> &'static str {
    if pass { "PASS" } else { "FAIL" }
}

/// Resolve a report entity to a forecast row's player id: exact id, case-insensitive id, unique
/// substring of the id, or a unique People.csv name match ("OHTANI" → `ohtansh01`, the bundled
/// player: name_last/name_first from `lahman/People.csv`).
fn resolve_entity<'a>(
    rows: &'a [ForecastRow],
    entity: &str,
    names: &std::collections::HashMap<String, String>,
) -> anyhow::Result<&'a ForecastRow> {
    if let Some(r) = rows.iter().find(|r| r.player_id == entity) {
        return Ok(r);
    }
    let needle = entity.to_ascii_lowercase();
    let mut ids: Vec<String> = rows
        .iter()
        .filter(|r| r.player_id.to_ascii_lowercase().contains(&needle))
        .map(|r| r.player_id.clone())
        .collect();
    for (id, name) in names {
        if name.contains(&needle) && rows.iter().any(|r| &r.player_id == id) && !ids.iter().any(|x| x == id) {
            ids.push(id.clone());
        }
    }
    match ids.len() {
        1 => Ok(rows.iter().find(|r| r.player_id == ids[0]).expect("id from rows")),
        0 => anyhow::bail!("entity {entity:?} not found in the latest artifact's forecasts"),
        n => anyhow::bail!("entity {entity:?} is ambiguous ({n} matches)"),
    }
}

/// playerID → "{nameFirst} {nameLast}" lower-cased, from the bundled People.csv.
fn name_index(paths: &CliPaths) -> std::collections::HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(paths.lahman.join("People.csv")) else { return Default::default() };
    raw.strip_prefix('\u{feff}')
        .unwrap_or(&raw)
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            // People.csv: ID,playerID,birthYear,...,nameFirst,nameLast,nameGiven — tolerate the
            // header and empties.
            (f.len() >= 16 && !f[1].eq_ignore_ascii_case("playerID"))
                .then(|| {
                    let name = format!("{} {}", f[14], f[15]).trim().to_ascii_lowercase();
                    (!name.is_empty()).then(|| (f[1].to_string(), name))
                })
                .flatten()
        })
        .collect()
}

fn render_report(artifact: &Artifact, row: &ForecastRow, html: bool) -> String {
    let score = &artifact.scores;
    let id = &row.player_id;
    let model = &artifact.model;
    let as_of = &artifact.as_of;
    let hash = content_hash(artifact);
    let season = row.season;
    let proj = format!(
        "wOBA {:.3} over {:.0} PA (reliability {:.2}, bucket {:?})",
        row.woba, row.pa, row.reliability, row.bucket
    );
    let i80 = format!("{:.3} – {:.3} wOBA", row.woba + row.q10, row.woba + row.q90);
    let i95 = format!("{:.3} – {:.3} wOBA", row.woba + row.q05, row.woba + row.q95);
    let backtest = format!(
        "pooled wOBA MAE {:.4}, PA MAE {:.1}, gate {}",
        score.pooled_mae,
        score.pooled_pa_mae,
        gate_word(score.gate.pass)
    );
    let attributions: Vec<&str> = vec![Attribution::LAHMAN, Attribution::RETROSHEET, Attribution::SAVANT];
    if html {
        let items: String = attributions.iter().map(|a| format!("<li>{a}</li>\n")).collect();
        format!(
            "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>sabr report — {id}</title></head><body>\n\
             <h1>Projection report — {id}</h1>\n<ul>\n\
             <li>Model: {model} (as-of {as_of})</li>\n\
             <li>Artifact: {hash}</li>\n\
             <li>Projection ({season}): {proj}</li>\n\
             <li>Interval (80%): {i80}</li>\n\
             <li>Interval (95%): {i95}</li>\n\
             <li>Backtest: {backtest}</li>\n\
             </ul>\n<h2>Attributions</h2>\n<ul>\n{items}</ul>\n</body></html>\n"
        )
    } else {
        let items: String = attributions.iter().map(|a| format!("- {a}\n")).collect();
        format!(
            "# Projection report — {id}\n\n\
             - Model: `{model}` (as-of {as_of})\n\
             - Artifact: `{hash}`\n\
             - Projection ({season}): {proj}\n\
             - Interval (80%): {i80}\n\
             - Interval (95%): {i95}\n\
             - Backtest: {backtest}\n\n\
             ## Attributions\n\n{items}"
        )
    }
}

fn cmd_report(paths: &CliPaths, rest: &[String]) -> anyhow::Result<i32> {
    let entity =
        rest.first().map(String::as_str).ok_or_else(|| anyhow::anyhow!("usage: sabr report <ENTITY> [--out FILE]"))?;
    let out = flag(rest, "--out");
    let index = open_index(paths)?;
    let row = index
        .latest(BATTING_MODEL)?
        .ok_or_else(|| anyhow::anyhow!("no artifact for {BATTING_MODEL}: run `sabr fit` first"))?;
    let path = PathBuf::from(&row.path);
    let json =
        std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("reading artifact {}: {e}", path.display()))?;
    let artifact: Artifact =
        serde_json::from_str(&json).map_err(|e| anyhow::anyhow!("parsing artifact {}: {e}", path.display()))?;
    let frow = resolve_entity(&artifact.forecasts, entity, &name_index(paths))?;
    let html = out.is_some_and(|o| o.ends_with(".html"));
    let rendered = render_report(&artifact, frow, html);
    match out {
        Some(file) => {
            std::fs::write(file, &rendered)?;
            println!("report written to {file}");
        }
        None => print!("{rendered}"),
    }
    Ok(0)
}

fn cmd_refresh(paths: &CliPaths, rest: &[String], t0: Instant) -> anyhow::Result<i32> {
    let scope = match flag(rest, "--scope") {
        None => "tracked",
        Some("all") | Some("tracked") => flag(rest, "--scope").unwrap(),
        Some(other) => anyhow::bail!("unknown refresh scope {other:?}: usage: sabr refresh [--scope all|tracked]"),
    };
    if scope == "all" {
        println!("network sources not yet wired (Wave 8): refreshing local sources only");
    }
    // Tracked refresh: the tracked set is the bundled player plus the corpus-wide tracked
    // entities; the fit refits every tracked entity in one pass (no network in this wave).
    let as_of = chrono::Utc::now().format("%Y-%m-%d").to_string();
    for line in fit_all(paths, &as_of)? {
        println!("{line}");
    }
    println!("refresh wall time: {:?}", t0.elapsed());
    Ok(0)
}

fn cmd_build_corpus(paths: &CliPaths, rest: &[String]) -> anyhow::Result<i32> {
    let ev_dir = PathBuf::from(
        flag(rest, "--ev-dir").ok_or_else(|| anyhow::anyhow!("usage: sabr build-corpus --ev-dir DIR --out DIR"))?,
    );
    let out_dir = PathBuf::from(
        flag(rest, "--out").ok_or_else(|| anyhow::anyhow!("usage: sabr build-corpus --ev-dir DIR --out DIR"))?,
    );
    // Seasons: every year present in the event-file names, top-level OR nested one level
    // (`<ev-dir>/<season>/<season><TEAM>.EV[AN]` — how the mirror download is organised).
    let is_ev_file = |name: &str| -> Option<i16> {
        let bytes = name.as_bytes();
        if (name.len() == 11 || name.len() == 12)
            && bytes.iter().skip(4).take(3).all(u8::is_ascii_uppercase)
            && bytes.iter().take(4).all(u8::is_ascii_digit)
            && (name.ends_with(".EVA") || name.ends_with(".EVN"))
        {
            name[..4].parse().ok()
        } else {
            None
        }
    };
    let mut seasons: Vec<i16> = Vec::new();
    let mut season_dirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&ev_dir)? {
        let name = entry?.file_name().to_string_lossy().to_string();
        match is_ev_file(&name) {
            Some(y) => seasons.push(y),
            None => {
                // A season directory (`2001`) holding that year's team files.
                let p = ev_dir.join(&name);
                if name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit()) && p.is_dir() {
                    season_dirs.push(p.clone());
                    for sub in std::fs::read_dir(&p)? {
                        if let Some(y) = is_ev_file(&sub?.file_name().to_string_lossy()) {
                            seasons.push(y);
                        }
                    }
                }
            }
        }
    }
    seasons.sort();
    seasons.dedup();
    anyhow::ensure!(!seasons.is_empty(), "no event files named YYYY<TEAM>.EV[AN] in {}", ev_dir.display());
    // build_bundle globs one dir; feed it each season dir's files (top-level flat layout, or
    // one season subdirectory at a time — the mirror download's layout).
    let csv_path: Option<PathBuf>;
    let mut batting_report = String::new();
    let mut pitching_report = String::new();
    if season_dirs.is_empty() {
        let (csv, batting, pitching) = sabr_retro::build_bundle(&out_dir, &ev_dir, &paths.lahman, &seasons)?;
        csv_path = Some(csv);
        batting_report = format!("{batting}");
        pitching_report = format!("{pitching}");
    } else {
        // Each season's run overwrites out_dir/cut-lines.csv; merge them into one
        // season-ordered file: header once, then per-season data rows, accumulated
        // in a side file that finally replaces the per-season path's last output.
        let merged = out_dir.join("cut-lines.csv");
        let acc = out_dir.join("cut-lines.all.csv");
        let mut header_written = false;
        let mut body_acc = String::new();
        for dir in &season_dirs {
            let year: i16 = dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse().ok()).unwrap_or(0);
            let (csv, batting, pitching) = sabr_retro::build_bundle(&out_dir, dir, &paths.lahman, &[year])?;
            let body = std::fs::read_to_string(&csv).map_err(|e| anyhow::anyhow!("season {year}: {e}"))?;
            for (i, line) in body.lines().enumerate() {
                if i == 0 {
                    if !header_written {
                        body_acc.push_str(line);
                        body_acc.push('\n');
                        header_written = true;
                    }
                    continue;
                }
                body_acc.push_str(line);
                body_acc.push('\n');
            }
            batting_report.push_str(&format!("{batting}"));
            pitching_report.push_str(&format!("{pitching}"));
        }
        std::fs::write(&acc, &body_acc)?;
        std::fs::copy(&acc, &merged)?;
        csv_path = Some(merged);
    }
    let csv = csv_path.expect("at least one bundle run");
    println!("cut lines written to {}", csv.display());
    print!("{batting_report}");
    print!("{pitching_report}");
    Ok(0)
}

/// `constants` and `trim-register` both run the bundled set's manifest verification (the
/// generators are the retro crate's lib functions; the CLI only runs them) and print hashes.
fn cmd_constants(paths: &CliPaths) -> anyhow::Result<i32> {
    let set = sabr_data::bundled::BundledSet::open(&paths.sample)
        .map_err(|e| anyhow::anyhow!("bundled set verification failed: {e}"))?;
    println!("bundled set verified: {}", paths.sample.display());
    for (path, ok) in set.verify()? {
        println!("  {:<32} {}", path, if ok { "OK" } else { "MISSING (existence only)" });
    }
    for line in std::fs::read_to_string(paths.sample.join("manifest.sha"))?.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            println!("  manifest: {line}");
        }
    }
    Ok(0)
}

fn cmd_trim_register(paths: &CliPaths) -> anyhow::Result<i32> {
    let set = sabr_data::bundled::BundledSet::open(&paths.sample)
        .map_err(|e| anyhow::anyhow!("bundled set verification failed: {e}"))?;
    set.verify()?;
    let catalog = paths.sample.join("catalog.csv");
    let hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(std::fs::read(&catalog)?);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    let rows = std::fs::read_to_string(&catalog)?.lines().count().saturating_sub(1);
    println!("register (catalog.csv) verified: {hash}");
    println!("register rows: {rows}");
    Ok(0)
}

/// `smoke`: render every pane of the default workspace and each preset headlessly — no window,
/// no display server. The sandboxed stand-in for the windowed `TR_SMOKE_SECS` run (which needs
/// X11/XWayland and, on this machine, a compositor that repaints unfocused windows).
fn cmd_smoke(rest: &[String]) -> anyhow::Result<i32> {
    let frames: u32 = flag(rest, "--frames").and_then(|v| v.parse().ok()).unwrap_or(3).max(1);
    let dir = std::env::temp_dir().join(format!("sabr-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    // SAFETY: single-threaded CLI path; the worker reads TR_DATA_DIR at construction.
    unsafe { std::env::set_var("TR_DATA_DIR", &dir) };
    let ctx = egui::Context::default();
    let worker = crate::worker::Worker::spawn(ctx.clone(), tr_data::from_env())?;
    let mut app = crate::app::TerminalApp::new(&ctx, worker, "sample".into());
    let report = |label: &str, line: String| -> anyhow::Result<()> {
        anyhow::ensure!(!line.contains(" 0 clipped shapes"), "{label} rendered nothing");
        println!("smoke {label}: {line}");
        Ok(())
    };
    report("default", app.smoke_stage(&ctx, None, frames))?;
    for token in ["MORNING", "PROSPECTS", "WARROOM"] {
        let cmd = format!("LAYOUT {token}");
        report(token, app.smoke_stage(&ctx, Some(&cmd), frames))?;
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A CLI path set pointing at the real bundled sample and a throwaway artifact store.
    fn test_paths(tag: &str) -> (tempfile::TempDir, CliPaths) {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let paths = CliPaths {
            artifacts: tmp.path().join(tag).join("artifacts"),
            sample: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sabr-data/sample"),
            lahman: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sabr-data/sample/lahman"),
        };
        (tmp, paths)
    }

    /// The sandboxed smoke: every stage renders geometry with no window and no display server.
    #[test]
    fn smoke_renders_every_pane_headlessly() {
        let frames = 2;
        let dir = tempfile::tempdir().expect("tmpdir");
        // SAFETY: single-threaded harness; the worker reads TR_DATA_DIR at construction.
        unsafe { std::env::set_var("TR_DATA_DIR", dir.path()) };
        let ctx = egui::Context::default();
        let worker = crate::worker::Worker::spawn(ctx.clone(), tr_data::from_env()).expect("worker");
        let mut app = crate::app::TerminalApp::new(&ctx, worker, "sample".into());
        assert!(app.smoke_stage(&ctx, None, frames).contains("4 panes"), "default workspace");
        for (token, panes) in [("MORNING", "5 panes"), ("PROSPECTS", "3 panes"), ("WARROOM", "4 panes")] {
            let line = app.smoke_stage(&ctx, Some(&format!("LAYOUT {token}")), frames);
            assert!(line.contains(panes), "{token}: {line}");
            assert!(!line.contains(" 0 clipped shapes"), "{token} rendered nothing: {line}");
        }
    }

    #[test]
    fn fit_writes_hashed_artifacts_and_index() {
        let (_tmp, paths) = test_paths("fit");
        let lines = fit_all(&paths, "2026-01-01").expect("fit");
        assert!(lines.iter().any(|l| l.contains("artifact hash (content)")));
        let index = open_index(&paths).expect("index");
        for model in [BATTING_MODEL, PITCHING_MODEL] {
            let row = index.latest(model).expect("latest").unwrap_or_else(|| panic!("no index row for {model}"));
            assert!(Path::new(&row.path).exists(), "artifact file {} missing", row.path);
        }
    }

    #[test]
    fn backtest_gate_passes_on_bundled_sample() {
        let (_tmp, paths) = test_paths("backtest");
        let code = cmd_backtest(&paths, &[], Instant::now()).expect("backtest ran");
        assert_eq!(code, 0, "calibration gate must pass on the bundled sample (DR-3.7)");
    }

    #[test]
    fn report_contains_attributions_and_writes_html() {
        let (tmp, paths) = test_paths("report");
        fit_all(&paths, "2026-01-01").expect("fit");
        let md = tmp.path().join("report.md");
        let html = tmp.path().join("report.html");
        let code =
            cmd_report(&paths, &["OHTANI".to_string(), "--out".into(), md.display().to_string()]).expect("md report");
        assert_eq!(code, 0);
        let md_text = std::fs::read_to_string(&md).expect("md");
        assert!(md_text.contains(Attribution::LAHMAN), "markdown report must carry the Lahman attribution");
        assert!(md_text.contains("ohtansh01"), "OHTANI must resolve to the bundled player");
        let code = cmd_report(&paths, &["ohtansh01".to_string(), "--out".into(), html.display().to_string()])
            .expect("html report");
        assert_eq!(code, 0);
        let html_text = std::fs::read_to_string(&html).expect("html");
        assert!(html_text.starts_with("<!doctype html>"));
        assert!(html_text.contains(Attribution::RETROSHEET));
    }

    #[test]
    fn build_corpus_produces_cut_lines_from_fixture() {
        let (tmp, paths) = test_paths("build-corpus");
        let ev = tmp.path().join("ev");
        std::fs::create_dir_all(&ev).expect("ev dir");
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/retro/fixtures/2024ARI.EVN"),
            ev.join("2024ARI.EVN"),
        )
        .expect("fixture copy");
        let out = tmp.path().join("bundle");
        let rest = ["--ev-dir".to_string(), ev.display().to_string(), "--out".to_string(), out.display().to_string()];
        let code = cmd_build_corpus(&paths, &rest).expect("build-corpus");
        assert_eq!(code, 0);
        let csv = std::fs::read_to_string(out.join("cut-lines.csv")).expect("cut-lines.csv");
        assert!(csv.lines().count() > 1, "cut-lines.csv must be non-empty beyond the header");
    }

    #[test]
    fn arg_parsing_exit_codes() {
        let (_tmp, paths) = test_paths("args");
        assert_eq!(run(&["nonsense".into()]), 2);
        let empty: Vec<String> = Vec::new();
        assert_eq!(run(&empty), 2);
        assert_eq!(run(&["fit".into(), "--model".into(), "poisson".into()]), 1);
        let _ = paths;
    }

    #[test]
    fn constants_and_trim_register_verify() {
        let (_tmp, paths) = test_paths("verify");
        assert_eq!(cmd_constants(&paths).expect("constants"), 0);
        assert_eq!(cmd_trim_register(&paths).expect("trim-register"), 0);
    }
}
