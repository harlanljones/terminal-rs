# 01 — Grill log (Phase 3)

Process: each round asks the whole current frontier — every question whose prerequisites are settled — numbered,
with a recommended answer worded so "yes" accepts it. The user's answers become decision records `DR-3.n`:
decision · alternatives rejected · why · what would change our mind. Facts behind every question live in the
Phase 2 record (blindspot pass, review-corrected 2026-10-20) and `00-audit.md`; this file holds decisions only.
No spec text here — specs are disposable projections of these records.

Phase 0 ledger (D0.1–D0.4) and the Phase 2 decision queue (D2.1–D2.13) are the inputs; D2.13 was resolved by
facts (both anchor postings live; Rockies closes 2026-10-30) and needed no decision.

## Round 1 — the stack (2026-10-21)

Asked 15, all answered; every recommended default accepted.

### DR-3.1 Bundled sample = a committed Statcast slice (D2.2)
- **Decision:** the repo commits one player-season of real Statcast pitch rows (≈2.5–3.6K rows, ≈2 MB) as the
  offline default so Ticket 01 runs from a fresh clone with no network, exactly as the brief requires.
- **Rejected:** (a) amend the brief to "offline after a documented one-time fetch"; (b) Ticket 01 on Lahman/
  Retrosheet season lines with Statcast arriving later by fetch.
- **Why:** the brief's constraints ("everything must run offline on a bundled sample dataset"; Ticket 01 = "real
  Statcast data for one player") are fixed; (a) and (b) each silently change one of them. The slice exceeds MLB
  ToU §1¶2 ("one copy … personal, non-commercial home use"); the user accepts that exposure, as
  `statcast-lakehouse` did publicly. Recorded in `06-risks.md`.
- **Would change our mind:** a takedown or written objection from MLBAM/Savant; or MLB publishing redistribution
  terms for Savant exports. Fallback is (a) with the fetch script already shipped.

### DR-3.2 Entity identity = (player, role) keyed on MLBAM id (D2.4)
- **Decision:** `Entity` = (player MLBAM id, role ∈ {bat, pit}); two-way players are two series and one profile;
  the Chadwick register is pinned by commit SHA and joined on `key_uuid`/`key_mlbam`, never `key_person`.
- **Rejected:** (player) only with role as a series attribute.
- **Why:** Savant splits by `player_type`, FanGraphs has one id with two stat paths, Stats API marks TWP with
  `primaryPosition.code = "Y"`; a role-less key makes every split, comparison and chart carry role anyway.
  `key_person` is documented as re-usable over time.
- **Would change our mind:** a third role (e.g. fielding-only series) that does not fit {bat, pit} — then role
  becomes an open enum, the key shape stays.

### DR-3.3 Language split = Rust-only for v1
- **Decision:** ingestion (HTTP + CSV/JSON parse), metrics, fitting and serving are all Rust; R/Python exist only
  as consumers of exported artifacts.
- **Rejected:** Python fitting sidecar (PyMC/Stan ready); R fitting sidecar (strongest R signal).
- **Why:** every v1 workload (Marcel, naive baseline, 25-fold walk-forward over ≈4.6M season-level rows) is
  single-digit seconds natively; a sidecar adds a second runtime to the PKGBUILD, a process protocol across the
  worker's `Request`/`Response` model and a second CI matrix for zero v1 benefit.
- **Would change our mind:** a v1.1 model family whose fit needs a probabilistic-programming runtime (hierarchical
  Bayes at minutes-per-fold) — then a sidecar is re-priced as its own decision record.

### DR-3.4 Forecasting runtime = native models + a reproducible CLI writing versioned artifacts
- **Decision:** projections are computed natively and written as versioned artifacts (model id, version, training
  window, as-of date, last backtest score); the same CLI runs on demand in the app and nightly; this is the model
  registry.
- **Rejected:** precomputed-only artifacts (terminal as viewer); a sidecar service (HTTP process).
- **Why:** the Rockies posting prices "Building, testing, documenting, and deploying predictive models and
  forecasts" and "Reproducible work"; precomputed-only has nothing running on demand and reads as a dashboard; a
  service doubles the surface of a desktop portfolio binary.
- **Would change our mind:** a reviewer audience that needs a networked endpoint (e.g. a shared team instance) —
  the CLI becomes the service's core, nothing is thrown away.

### DR-3.5 Storage = SQLite app store + columnar backtest corpus
- **Decision:** keep upstream's SQLite pattern (WAL, upsert, per-season file cache) for app state and the series
  cache, re-keyed from `(symbol, tf, ts)` to entity + window + sequence; pitch-level seasons and the walk-forward
  corpus live as Parquet/CSV files outside the DB.
- **Rejected:** SQLite for everything; embedded DuckDB.
- **Why:** per-PA/per-pitch events collide on timestamps and the brief wants event-count windows; a pitch-level
  season is ≈710K rows × 119 columns (≈380 MB CSV) and 11 seasons ≈ 7.8M rows — in SQLite that is a >1 GB file
  and full scans for rolling windows. DuckDB is the strongest analytics story but a large native dependency and a
  second query engine beside rusqlite. Season-level rows (25 seasons ≈ 4.6M × ~15 columns) are comfortable in
  SQLite.
- **Would change our mind:** rolling-window queries over multi-season pitch data becoming an interactive feature
  (not a batch step) — DuckDB over the Parquet files is the upgrade path.

### DR-3.6 Runtime fetch policy = per-entity, on demand, cached; risk worded as tolerance (D2.1)
- **Decision:** the app fetches per entity on demand and caches locally; never bulk season pulls; no
  redistribution of fetched rows. `06-risks.md` states that MLB ToU §1(xi) ("automated scripts to collect
  information") has **no** user-initiated exception, that the basis for fetching is de-facto tolerance plus
  copyright.txt's "individual, non-commercial, non-bulk" grant, and that §1¶2 "one copy … single device" is why the
  cache stays local.
- **Rejected:** no runtime fetch (import-only app).
- **Why:** the brief names incremental ingestion, disk cache and per-source rate-limit/ToS handling as required
  portfolio evidence; import-only deletes that evidence. The honest wording matters more than a false claim of
  conformity.
- **Would change our mind:** MLB enforcing (xi) against desktop clients, or rate limiting the endpoints — then
  import-only becomes the default and the fetch path is a documented opt-in.

### DR-3.7 Backtest compute budget = < 60 s suite inside `cargo test`
- **Decision:** v1 backtests (Marcel + naive, walk-forward 2001–2025, PA-weighted MAE of wOBA + the DR-3.12
  interval spec) run inside the normal test suite with a 60 s cap, so calibration is a per-ticket acceptance gate.
- **Rejected:** a separate long-running backtest target.
- **Why:** per-ticket acceptance by calibration (red-team R2) only works if the gate runs on every change; a
  heavier model family (minutes per fold × 25) is a v1.1 decision that re-opens this record.
- **Would change our mind:** adding a model whose honest backtest cannot fit the cap — then a scheduled CI job
  carries the long suite and the gate keeps a reduced-fold smoke version.

### DR-3.8 Lane ranking = forecasting → biomechanics → amateur-draft
- **Decision:** scope cuts fall in reverse order: amateur-draft first, biomechanics second, forecasting never.
- **Rejected:** forecasting → amateur-draft → biomechanics; forecasting only.
- **Why:** forecasting has the deepest public stack (Statcast + Stats API + Lahman/Retrosheet + free Steamer
  2017–2024) and the anchor posting; biomechanics rides the same Statcast ingest (bat-tracking columns, coverage
  2023-H2 partial / 2024-04-03→) and is priced by the Blue Jays Hitting Analyst; the amateur lane is a thin JSON
  feed (`/api/v1/draft/{year}`: picks, bonuses, schools, scouting blurbs; College = sportId 22, H.S. = 586) with
  no tracking data below MLB.
- **Would change our mind:** public amateur tracking data appearing, or the target postings shifting toward
  draft work.

### DR-3.9 R interop = export-only
- **Decision:** CSV/Arrow artifacts plus one documented R snippet that consumes them; no `extendr` embedding.
- **Rejected:** embed R via extendr; ignore R.
- **Why:** Rays and Blue Jays list R as one of several tools; R-ready artifacts are the evidence, and embedding
  adds a toolchain to the PKGBUILD and CI for a hot path nothing in v1 needs. Ignoring R leaves an explicit gap
  in `05-posting-alignment.md`.
- **Would change our mind:** a target posting requiring R *models* (not R consumption) — then an R sidecar is
  priced as its own record.

### DR-3.10 Cadence = nightly precomputed + manual refresh; no live in-game mode (D2.9)
- **Decision:** data refreshes nightly (Savant collects "daily at 3 a.m."); the user can refresh on demand; the
  "Live/Offline" chip becomes "as-of + freshness".
- **Rejected:** game-feed polling (`/api/v1.1/game/{pk}/feed/live`).
- **Why:** the off-season is ≈4 months of nothing; polling adds a scheduler and a second data path to Ticket 01's
  scope; the feed exists, so the decision is reversible.
- **Would change our mind:** an in-game use case the postings price (none found).

### DR-3.11 Backtest comparison set = Marcel + naive + Steamer 2017–2024 aggregate scores (D2.3)
- **Decision:** Marcel re-implemented in-repo and verified against Tango's 2001–2012 files as goldens (not a
  dependency); "prior season" naive baseline; Steamer preseason 2017–2024 obtained via the Steamer site's free
  registration and reported as aggregate scores only — the scores are computed once on the author's machine and
  committed as a small fixture (system, season, metric, value); rows are never committed. ZiPS aggregate-only
  behind the FanGraphs membership, or dropped.
- **Rejected:** Marcel + naive only; adding ZiPS via a paid FG membership now.
- **Why:** the brief's "vs Steamer/ZiPS/Marcel" is fully met for two of three at $0; no licence text on the
  Steamer site means rows stay non-shippable. ZiPS is a personal $80/yr export on the author's side only, not
  the default path — deferred, not refused.
- **Would change our mind:** Steamer withdrawing free historical downloads (then aggregate scores computed once
  and frozen as a fixture).

### DR-3.12 Evaluation spec = PA-weighted MAE of wOBA + PICP per season per reliability bucket (D2.5)
- **Decision:** rates: PA-weighted MAE of wOBA after re-centering every system to a common league mean, all
  players included with a named missing-player floor. Calibration: 80% nominal interval; PICP ∈ [0.75, 0.85] in
  **each** of 3 walk-forward seasons, PA-weighted, reported per Marcel-reliability bucket (High/Med/Low/None);
  MIS reported for sharpness. Playing time scored separately on PA MAE (Whiffs convention).
- **Rejected:** pooled PICP only.
- **Why:** pooled coverage can pass while the Low/No-reliability buckets are miscalibrated (Tango: forecast
  .326 vs actual .313; .335 vs .319) — exactly the population the brief calls honest uncertainty.
- **Would change our mind:** the bucket gate proving unreachable for every system including Steamer on the same
  data — then the tolerance band widens with the evidence shown, not silently.

### DR-3.13 Alert trigger = projection delta + transaction/IL events (D2.10)
- **Decision:** a tracked player alerts when its projection moves beyond a threshold since the last refresh or
  when a transaction/IL event arrives from the Stats API transactions feed, deduped on
  (person, date, typeCode, description) because the feed returns byte-identical duplicate rows.
- **Rejected:** raw-stat movement.
- **Why:** "move" is undefined for rates at small samples; projection deltas and roster events are what a
  front-office user acts on.
- **Would change our mind:** user testing showing threshold alerts are too quiet during the season.

### DR-3.14 Distribution = PKGBUILD + `cargo install --locked --git`; Ubuntu CI as the non-Arch proof (D2.11)
- **Decision:** a PKGBUILD in-repo (AUR `-git` later), `cargo install --locked --git <url> sabr-rs` as the
  any-distro path (crate argument mandatory in this workspace), the existing Ubuntu CI job kept as red-team R5's
  proof. Flatpak/AppImage deferred.
- **Rejected:** Flatpak now.
- **Why:** Flatpak needs `flatpak-cargo-generator` offline sources and a second packaging pipeline; nothing in the
  postings prices it.
- **Would change our mind:** a reviewer unable to build from source on their distro.

### DR-3.15 XDG split with persisted-format version bumps (D2.12)
- **Decision:** config → `$XDG_CONFIG_HOME`, caches → `$XDG_CACHE_HOME`, layouts/open-state → `$XDG_STATE_HOME`,
  replacing upstream's single `$XDG_DATA_HOME/terminal-rs`; `LayoutDoc.version` and the prefs version are bumped
  and upstream migrations dropped in the same change so stale finance study codes cannot survive the cutover.
- **Rejected:** keep upstream's single data dir.
- **Why:** it is part of the storage re-keying anyway, and `Spec` round-trips unknown codes silently.
- **Would change our mind:** nothing foreseeable.

## Round 2 — data model and domain forks (2026-10-21)

Asked 12 (16–27). Two answers diverged from the recommendation and were re-asked once with quantified risk:
19 reverted to the recommendation; 25 is open pending further research (payroll sources). Facts measured for this
round: Ohtani 2025 Statcast rows — bat 2,940 (1.98 MB), pit 753 (0.52 MB); Judge 2025 bat 2,762 (1.83 MB); Savant
expected-statistics CSV per season/role exists (15/18 columns; 2015: 964 batters + 735 pitchers; 2024: 651 + 855);
Stats API `/api/v1/draft/{year}` returns picks, bonuses, schools, scouting blurbs; sportIds MLB 1, AAA 11, AA 12,
A+ 13, A 14, ROK 16, Minors 21, College 22, H.S. 586; Savant minor-league CSV export works (2025: 25,000 rows ×
119 columns in 35 s — an apparent per-query cap); Spotrac ToS forbids scrapers/data mining/systematic extraction;
Cot's Contracts sits behind an Anubis proof-of-work gate.

### DR-3.16 Sample slice = Ohtani 2025, both roles
- **Decision:** the committed Statcast slice is Shohei Ohtani (MLBAM 660271), 2025 regular season, as batter
  (2,940 rows) and as pitcher (753 rows); ≈2.5 MB CSV. Marcel's prior seasons come from the season corpus
  (DR-3.17), not the slice.
- **Rejected:** Judge 2025 bat only; Ohtani bat only.
- **Why:** exercises the (player, role) identity (DR-3.2) inside Ticket 01's fixture for +0.5 MB.
- **Would change our mind:** the pitcher role making Ticket 01's scope creep (two projections) — then the pit rows
  stay committed but Ticket 01 renders the bat series only.

### DR-3.17 Season totals = Lahman pinned for closed seasons, Stats API for the open season
- **Decision:** finished seasons come from Lahman (Batting/Pitching/People, 1871–2025, CC BY-SA 3.0, every file
  SHA-256 pinned — sabr-jev pattern); the current season's partial totals come from the Stats API and are
  reconciled at the next Lahman release; Retrosheet is the cross-check (prototype P2). The committed derived
  corpus is distributed under BY-SA.
- **Rejected:** Stats API for everything (needs bulk historical pulls DR-3.6 forbids; official totals disagree by
  source); Retrosheet-derived for everything (event-file parser and reconciliation before Ticket 01).
- **Why:** Lahman is the only redistributable, pinned, season-level source that exists today; the open-season gap
  is exactly one season.
- **Would change our mind:** P2 showing modern-season disagreement between Lahman and the Stats API large enough
  to move a projection — then the open-season reconciliation becomes a permanent diff report.

### DR-3.18 League constants derived in-repo from Retrosheet, frozen and hashed
- **Decision:** per-season wOBA linear weights (run-expectancy based) and as-of park factors (past seasons only,
  regressed 1−(1−iPF)·X) are computed from Retrosheet event/game logs by an in-repo tool and frozen as a versioned
  constants file with SHA-256 (saberkit + sabr-jev pattern).
- **Rejected:** fixed published constants (FanGraphs Guts under FG ToS §6/§11; 5-year PF uses two future years).
- **Why:** walk-forward honesty — nothing in a feature may see a later season; Retrosheet permits any use with
  its notice.
- **Would change our mind:** nothing foreseeable; the tool is cheap and the file is auditable.

### DR-3.19 v1 projection = Marcel on season lines; Statcast is the descriptive layer (re-asked, reverted)
- **Decision:** the v1 projection consumes season totals only (as Marcel specifies). Statcast drives the rolling
  windows, bat-tracking, splits and the "where my system is wrong" comparison. An xwOBA-informed model is v1.1,
  scored against this baseline with the Savant expected-statistics CSV fetched on demand.
- **Rejected (user's first answer, reverted on pushback):** blend xwOBA into the v1 projection — costs: training
  window shrinks to the Statcast era (8 walk-forward folds 2018–2025 instead of 25); 2019/2021 ball changes force
  season effects into the first model; needs per-player-season xwOBA for everyone 2015–2025 (≈17K MLB-sourced
  rows) committed to run offline — the DR-3.1 exposure class ×7; and Ticket 01 would ship an improvement before
  the baseline. Also rejected: keep the blend with the corpus committed; keep the blend with a first-run fetch
  (breaks the offline DR-3.7 gate).
- **Why:** honest-baseline ordering (red-team R1/R2) and the smaller committed MLB footprint.
- **Would change our mind:** the v1.1 xwOBA model beating Marcel on the DR-3.12 gate — then it becomes the
  default projection and Marcel the comparator, with the expected-stats corpus decision re-opened.

### DR-3.20 Horizons = preseason + rest-of-season (as-of any date)
- **Decision:** projections are produced as-of any date; preseason uses the Jan-1 cutoff, rest-of-season adds the
  current partial season as a PA-weighted term (Marcel's in-season form); backtests add in-season as-of dates.
  The as-of cadence for backtests is fixed in the spec (not yet specified).
- **Rejected:** preseason only.
- **Why:** the brief's anchor framing is rest-of-season forecasting; for Marcel the extra as-of dates fit the
  DR-3.7 budget.
- **Would change our mind:** in-season calibration failing the DR-3.12 gate for structural reasons (e.g. PT
  shocks) — then ROS ships with its own, wider, documented tolerance rather than being dropped.

### DR-3.21 Intervals = empirical walk-forward residual quantiles by reliability bucket
- **Decision:** p10/p50/p90 and the 80% band come from the distribution of backtest residuals conditioned on
  Marcel reliability bucket and projected PA (bayes-horizon pattern); calibration is checked by the DR-3.12 gate.
- **Rejected:** parametric beta-binomial on projected PA (ignores model error, under-covers low-reliability
  buckets); both with the parametric band as a reference overlay (more to build in Ticket 01).
- **Why:** the only method whose coverage is honest by construction with the data we have.
- **Would change our mind:** too few residuals per bucket for stable quantiles — then buckets merge, never a
  switch to parametric without the gate proving it.

### DR-3.22 Model registry = one JSON artifact per fit + a SQLite index
- **Decision:** each fit writes a JSON artifact (model id, version, as-of, training window, input file hashes,
  backtest scores) under the XDG data dir, indexed in the app DB; every projection panel renders version, training
  window and last backtest score from it.
- **Rejected:** SQLite tables only.
- **Why:** inspectable, diffable, portable to R/Python consumers (DR-3.9) without an export step.
- **Would change our mind:** artifact counts making directory scans slow — the index already covers that.

### DR-3.23 Grammar = `<ENTITY>[.ROLE] <FUNCTION> [ARGS] GO`
- **Decision:** entity = player name, MLBAM id or team abbreviation; optional `.BAT`/`.PIT` role suffix (default =
  primary role); bare-name ambiguity opens a picker ranked by the upstream `catalog::search` scoring; functions
  `CAST PROJ SPLT GAME ROST FARM COMP BTST ALRT`; upstream `parse`/`hint`/`describe` mechanism kept. Ergonomics
  (suffix spelling, picker feel) go to prototype P5.
- **Rejected:** yellow-key analog `<ENTITY> <CLASS> <FUNCTION>`; function-first.
- **Why:** keeps the `<SECURITY> <FUNCTION> <GO>` muscle memory the brief asks for; the class is inferable.
- **Would change our mind:** P5 showing the suffix is unusable at the keyboard — spelling changes, structure stays.

### DR-3.24 Presets = all three; Prospect watch on MiLB season lines + draft feed
- **Decision:** ship "Morning front-office briefing", "Prospect watch" and "Trade deadline war room". Prospect
  watch is built on Stats API minor-league season lines (sportIds 11–16) and the draft feed; MiLB Statcast is not
  a v1 dependency (Savant minors export exists but per-level coverage is unverified).
- **Rejected:** two presets with Prospect watch deferred.
- **Why:** the preset is demoable without MiLB Statcast; it carries the `FARM` function and the amateur lane's
  only cheap evidence.
- **Would change our mind:** MiLB season lines proving too sparse for a useful panel — then the preset ships with
  the draft feed only.

### DR-3.25 Payroll = Cot's Contracts public sheets at runtime (cited, cached) + Lahman ≤2016 bundled
- **Decision:** current-season contract money comes from Cot's Baseball Contracts' public Google Sheets
  (Baseball Prospectus / Jeff Euston): one CSV export per season sheet (Opening-Day salaries 2000–2026, ≈1,250
  rows/yr; per-team sheets for contract length/total value), fetched with a non-browser UA, cached locally, shown
  with the attribution "Cot's Baseball Contracts / Jeff Euston, Baseball Prospectus" in the panel; team joins via
  Stats API rosters (the salary sheet has no team column). The offline sample carries only Lahman Salaries
  (≤2016, CC BY-SA). No Cot's rows are committed.
- **Rejected:** runtime fetch from Spotrac/Cot's HTML (the user's first answer — Spotrac ToS forbids "data mining,
  robots, or similar data gathering and extraction tools" and "systematically extract, collect or harvest …
  any data"; Cot's HTML sits behind an Anubis proof-of-work gate); bundling a Cot's sample (no explicit
  redistribution grant); manual CSV import only (the war-room preset wants current money without a chore);
  Stats API (no salary field in any hydrate — verified); USA Today (JS-only, Gannett anti-scrape clause); ESPN
  (page retired, Disney ToS); Baseball-Reference (§5 "create any database … material substitute", 20 req/min);
  FanGraphs RosterResource (§17); Kaggle sets (Spotrac-derived, licence invalid); AP lists (team totals only).
- **Why:** the only key-free, plain-HTTP, per-player source whose terms permit "noncommercial purposes such as
  research" with citation; a portfolio project is noncommercial research. Verified live 2026-10-21: the 2026 sheet
  export returns 1,258 rows as `text/csv` with the `*` deferral marker intact.
- **Would change our mind:** BP restricting the sheets or objecting; sheet ids/layout churn exceeding one fix per
  season — then the fallback is the documented manual-import path with the same column layout (kept in any case).
- **Risk record (06-risks.md):** noncommercial-only basis; unstable sheet ids (scraped from the legacy index with a
  non-Mozilla UA, or pinned); hand-maintained layout (multi-row headers, `*` markers, "FA"/"cond opt" cells).

### DR-3.26 WAR = in-repo documented WAR under its own label, OAA when available
- **Decision:** batting runs from the DR-3.18 wOBA weights, a positional-adjustment table, replacement level per
  the 2013 unified convention, defense = Savant OAA where present else 0; published as "sabr-rs WAR" with its
  components visible, never presented as fWAR/bWAR.
- **Rejected:** runs above average only (no front office uses wRAA gaps); importing FanGraphs WAR (ToS §6/§11,
  default path must be key-free).
- **Why:** roster gaps need a replacement-level number; an own-label WAR with visible components is honest
  methodology, which the postings price.
- **Would change our mind:** a licensed WAR feed appearing; or OAA coverage gaps making defense = 0 misleading —
  then defense is shown as "not measured" rather than zero.

### DR-3.27 Refresh = CLI `refresh` + in-app worker scheduler; systemd user timer documented
- **Decision:** one refresh code path exposed by the CLI and called by the worker's interval scheduler (upstream
  `TR_CONGRESS_REFRESH_MIN` pattern); a systemd user timer is documented for headless nightly runs.
- **Rejected:** in-app scheduler only.
- **Why:** nightly artifacts must not depend on the terminal being open.
- **Would change our mind:** nothing foreseeable.

## Round 3 — bundle scope and remaining forks (2026-10-21)

Asked 6 (28–33); 29 re-asked once with measured sizes and revised. Facts measured for this round: register shard
`people-0.csv` = 32,871 rows, of which 1,531 MLB-played and 8,495 MLBAM-keyed (6% of rows have no first name);
×16 ≈ 526K rows / 68 MB full, ≈136K rows / 14 MB MLBAM-keyed (14 columns), ≈24.5K rows / 2.7 MB MLB-played;
`terminal-rs/.git` is 9.2 MB today.

### DR-3.28 Corpus file format = CSV (+zstd when large) in v1; Parquet with the pitch-level corpus (v1.1)
- **Decision:** the season-level corpus, the Statcast slice and the R/Python exports are CSV (zstd-compressed when
  large); Parquet via arrow-rs enters with the v1.1 multi-season pitch corpus (DR-3.19's xwOBA model).
- **Rejected:** Parquet from day one.
- **Why:** v1 data is ≈40K season rows + a 3.7K-row slice; arrow-rs/parquet add ≈60 crates to an 88 s release build
  and an Arrow dependency to Ticket 01 for nothing; the 7.8M-row pitch corpus (≈4 GB CSV vs ≈800 MB Parquet) is
  when Parquet pays.
- **Would change our mind:** the pitch corpus arriving earlier than v1.1 — the format switch is local to the
  corpus reader.

### DR-3.29 Catalog = Chadwick register trimmed to MLBAM-keyed rows (re-asked, revised)
- **Decision:** the bundled entity catalog is every register row with a `key_mlbam` (≈136K rows, ≈14 MB, 14
  columns: `key_uuid, key_mlbam, key_retro, key_bbref, key_bbref_minors, key_fangraphs`, names, birth year, MLB
  and pro spans), produced by an in-repo trim tool from a pinned register commit SHA, ODC-By attributed.
- **Rejected (user's first answer, revised on pushback):** the full register — ≈526K rows / 68 MB in a 9.2 MB
  repo that every `cargo install --git` clones, 6% surname-only clusters as picker noise, and a mandatory search
  index in Ticket 01. Also rejected: MLB-played only (24.5K rows) with on-demand minors fetch; Lahman People only
  (no MLBAM/FanGraphs ids — the crosswalk's primary key).
- **Why:** the MLBAM-keyed subset is exactly the population the Stats API and Savant minors can serve, so FARM
  searches resolve offline without the amateur/independent rows nothing can fetch.
- **Would change our mind:** the amateur-draft lane needing college/HS identities — the draft feed carries its own
  MLBAM person ids, so the subset grows with each pinned refresh rather than switching to the full file.

### DR-3.30 Bundled season corpus = Lahman 1998–2025: Batting, Pitching, People, Appearances, Teams
- **Decision:** the committed BY-SA corpus is the expansion era (30 teams): Batting and Pitching lines, People,
  Appearances (primary position for the DR-3.26 positional adjustment) and Teams (park/league joins); ≈45K rows,
  single-digit MB; every source file SHA-256 pinned.
- **Rejected:** full Lahman 1871–2025 (pre-expansion seasons the backtest never touches); 2015–2025 only (8 folds —
  the cost that reverted DR-3.19).
- **Why:** gives Marcel three prior seasons for every 2001–2025 fold (DR-3.7) and nothing more.
- **Would change our mind:** an aging-curve ticket (DR-3.19's "where wrong" panel, the v1.1 survivor-corrected
  curve) wanting longer careers — then the window widens to 1977 (MGL's figures) in its own record.

### DR-3.31 Walk-forward as-of cadence = Jan 1 + May 1, Jun 1, Jul 1, Aug 1, Sep 1
- **Decision:** six as-of dates per backtest season: the preseason Jan-1 cut and five in-season rest-of-season cuts
  at month starts; 25 folds × 6 = 150 Marcel fits inside the DR-3.7 gate; ≈125 in-season evaluations feed the
  DR-3.21 residual buckets.
- **Rejected:** Jan 1 + All-Star break (thin buckets); weekly (650 fits — fine for Marcel, multiplies every later
  model).
- **Why:** enough in-season points for stable quantiles without pre-committing the budget of heavier models.
- **Would change our mind:** ROS calibration failing at specific cuts (e.g. May 1 with ≤100 PA) — then the cadence
  gains later starts, never fewer evaluations.

### DR-3.32 Non-technical surface = `report` CLI subcommand (Markdown/HTML one-pager) in v1
- **Decision:** `report` renders a player/team one-pager (projection, interval, backtest score, as-of, model
  version, attribution lines) from the same DR-3.22 JSON the panels read; the terminal stays the primary surface.
- **Rejected:** defer to v1.1 (posting gap); pane screenshot export (pixels, nothing quotable).
- **Why:** Orioles/Mets price "present to non-technical audiences"; the artifact already holds every number.
- **Would change our mind:** nothing foreseeable; the subcommand is a renderer.

### DR-3.33 Biomechanics lane in v1 = descriptive bat-tracking panel, no model
- **Decision:** `bat_speed`, `swing_length`, `attack_angle`, `attack_direction`, `swing_path_tilt` from the Statcast
  rows already ingested, shown as rolling/percentile panels under SPLT/COMP with the B3 coverage intervals
  ({2023 H2 partial} ∪ [2024-04-03, ∞)) honoured — null = "not tracked", never zero.
- **Rejected:** defer the lane to v1.1.
- **Why:** the data is in the slice already; the cost is one panel; it is the only Blue Jays evidence available.
- **Would change our mind:** the panel needing MiLB bat-tracking (not public) to be useful for prospects — then it
  stays MLB-only and the gap is recorded.

## Round 4 — forecasting-method decisions carried from Phase 2 (2026-10-21)

Asked 3 (34–36), all recommended defaults accepted.

### DR-3.34 Aging = Marcel's linear adjustment in v1; later ticket compares GAM-across vs MGL season-completion
- **Decision:** v1 applies (29 − age) × 0.006 under 29 and × 0.003 over, as Marcel specifies. The survivor-bias
  ticket compares Judge's GAM-across with a career-mean control and age re-centering (BP 2020, code at
  baseballprospectus/bbpro) against MGL's 2016 season-completion method, scored on the DR-3.12 gate.
- **Rejected:** a survivor-corrected delta curve in v1; the Lichtman "phantom year" correction as the later method
  (BP: "The GAM Across method performs as well or better … and does not require post-hoc corrections").
- **Why:** the aging ticket needs the backtest harness to choose a method; Marcel's adjustment is the documented
  baseline.
- **Would change our mind:** the aging ticket's winner beating Marcel-linear on the gate — it replaces it.

### DR-3.35 Rolling windows = the brief's display windows, labelled with shrinkage n/(n+x); r = 0.7 stated
- **Decision:** windows (50-PA xwOBA included) stay as the brief lists them; each window carries a reliability
  label computed as n/(n+x) from the Carleton split-half points (K% 60 PA, BB% 120, ISO 160 AB, OBP 460, AVG
  910 AB, BABIP 820 BIP; r = 0.7 convention stated in the UI/spec); metrics without a published point (xwOBA) are
  labelled "no threshold".
- **Rejected:** Carleton points as default windows with binary "below reliability" styling (flat charts at 910 AB;
  unlike comparison for xwOBA).
- **Why:** the points measure descriptive stability, not predictive thresholds; a weight is honest, a cliff is not.
- **Would change our mind:** a published xwOBA stabilisation study — then it gets a point like the others.

### DR-3.36 Playing time = Marcel's PA formula as the v1 model, backtested on PA MAE; scenario override on top
- **Decision:** projected PA = 0.5·PA₋₁ + 0.1·PA₋₂ + 200 (pitchers: IP with +25 RP / +60 SP), scored separately on
  PA MAE with missing players at 100 PA (Whiffs convention); the user can override PT as a sensitivity, which
  never changes the model's own score.
- **Rejected:** PT as an independent scenario input with no model (contradicts Tango; cuts the brief's
  playing-time model).
- **Why:** PT is the dominant error for every system (Whiffs 2024 PA MAE: Steamer 106.7, Marcel 136.9, ZiPS
  206.8); modelling and scoring it is the honest baseline.
- **Would change our mind:** a v1.1 PT model (injury history, role) beating the formula on PA MAE.

## Status

Grill closed 2026-10-21: 36 questions over four rounds, 36 decision records DR-3.1–DR-3.36; every Phase 2 queue
item (D2.1–D2.13) and every fork the brief names for Phase 3 has a record. Re-asked once with quantified risk and
revised by the user: 19 (reverted), 25 (researched, resolved), 29 (revised). Left to the spec's "Not yet
specified": per-function panel semantics, the missing-player floor convention for the rate MAE (three published
conventions, DR-3.12), feature coverage-interval contracts (B3), the register/Lahman refresh procedure.

Phase 4 candidates (most expensive to reverse, from these records): (1) the series/window/entity data model and
cache key (DR-3.2, DR-3.5, DR-3.35); (2) the forecasting core's artifact and backtest contract (DR-3.4, DR-3.7,
DR-3.12, DR-3.21, DR-3.22, DR-3.31); (3) the data layer's source/fetch/cache shape across Statcast, Stats API,
Lahman, Retrosheet, Chadwick and Cot's (DR-3.6, DR-3.17, DR-3.25, DR-3.29).

## Phase 4 outcomes recorded as decisions (2026-10-21)

The design arena (`02-design-arena.md`) produced two new records and one amendment; the arena verdicts V-A/V-C are
design picks, not stack changes, and live there.

### DR-3.37 In-season backtest corpus = Retrosheet event files → in-repo play parser → bundled cut lines
- **Decision:** per-date player lines for the DR-3.31 in-season cuts (May 1 … Sep 1, 2001–2025) are derived from
  Retrosheet event files (Chadwick mirror `seasons/<year>/<TEAM>.EVA|EVN`, pinned by commit SHA) by an in-repo
  play parser that produces cumulative batting and pitching lines at each cut (≈190K rows, bundled under the
  Retrosheet notice). The parser is validated against Lahman season totals — prototype P2 becomes its acceptance
  test.
- **Rejected:** Stats API `byDateRange&playerPool=All` league pulls (verified: 718 hitters / 457 pitchers per cut;
  250 requests; ≈300K MLB-sourced rows committed — DR-3.1 exposure ×80); preseason-only backtests with prospective
  ROS calibration (thin evidence for the anchor posting's rest-of-season framing).
- **Why:** the only redistributable per-date source; the parser is bounded (≈15 outcome tokens cover the batting
  line; runs/earned runs are explicit markers) and doubles as the official-totals cross-check.
- **Would change our mind:** the parser's pitcher lines (ER via advancement parsing) proving unreliable against
  Lahman — then pitcher ROS cuts use RA9 components instead of ERA, documented in the protocol.

### DR-3.12a Calibration gate amended: Wilson interval per cell
- **Decision:** the pass/fail test is "0.80 lies inside the 95% Wilson interval of the observed 80%-interval
  coverage" per (walk-forward season, reliability bucket), PA-weighted; cells with fewer than `n_min` residuals merge
  upward into the next bucket (DR-3.21's merge clause); every cell carries `(picp, n)`. The brief's 75–85% remains
  the reported band.
- **Rejected:** the fixed [0.75, 0.85] band as the gate (≈150 residuals per cell → binomial SE ≈ 0.033; a correctly
  calibrated model fails a 12-cell gate on luck in a large share of runs; red CI by chance).
- **Why:** the gate must fail on miscalibration, not on sample size.
- **Would change our mind:** cell sizes growing (weekly cuts, more seasons) to where the fixed band is ≥ 3 SE —
  then the fixed band returns as the stricter test.

## Phase 5 — throwaway prototype findings (2026-10-21)

All prototypes live under `/tmp/sabr-proto/` and are discarded; nothing is promoted. Facts below feed the spec;
decisions are recorded as DR-3.21a and DR-3.38–3.41.

### Measured facts
- **Marcel walk-forward (Lahman 1998–2025, hitters, prototype constants):** 25 folds fit in 0.05 s (Python);
  ≈900 players/fold; PA-weighted MAE wOBA 0.033 (prototype constants, all batters, no re-centering — the published
  .027–.029 bar needs the real DR-3.18 weights and the DR-3.12 population rules); PA MAE 162. **DR-3.7's 60 s
  budget is a non-issue for v1** (P7).
- **B-F2 nested vs pooled residual quantiles:** PICP differs by > 0.01 in 51/88 (season, bucket) cells, max 0.106 —
  the strictly-earlier-folds rule must be a harness test, not a convention.
- **Residual model specification:** bucket-only quantiles over-cover Low/None (0.88–0.98) because residual spread
  scales with PA; standardising residuals as z = err·√PA per bucket gives mean PICP 0.80/0.81/0.81/0.80 across
  the four buckets; remaining failing cells are regime years (2019: Low/None at 0.67). → DR-3.21a.
- **Gate behaviour (DR-3.12a):** with the standardised model, Wilson pass 75% of cells vs fixed-band 68%; cell
  sizes 76–501 players. Failures cluster in ball-change years for every bucket — an honest failure, reported, not
  tuned away.
- **A-F1 rolling engine:** defined-count rolling window (skips undefined values) over 180K events = 1.7 ms,
  700K = 4.9 ms per derive (Rust, naive ring). Memoisation is unnecessary; V-A's "no memo" stands.
- **A-F3 LOD gaps (real `LinePyramid::build`):** a 1-point NaN gap vanishes 50% / 75% / 88% / 94% of the time at
  LOD levels 1–4; a 3-point gap widens 50% at level 1 and vanishes 25% at level 2. A gap-preserving merge policy
  (`any NaN → NaN`) is required for coverage/IL gaps; it widens gaps to the group size at coarse levels, which is
  the honest rendering.
- **A-F2 Savant churn (same day, game 777288):** two fetches 10 min apart byte-identical; `(game_pk, at_bat_number,
  pitch_number)` unique and stable; **CSV row order is not pitch order** (starts AB 50 p6→p1) → `Seq` must be
  assigned by sorting on that triple, never by file position. 7-day churn: pending re-fetch of the Ohtani slice
  (due 2026-10-28).
- **C-F1 `FetchKey`:** explicit per-variant keys (`savant/v1/player/660271/bat/2025`, …) are stable and unique;
  range requests (transactions) decompose into per-day keys, which resolves C-obj1.
- **C-F3 Cot's sheets (2016, 2021, 2026 tabs):** one parser → 3,394 rows, 1 unparsed (0.03%); the `gviz` export
  drops the `MLS` header cell — use `export?format=csv&gid=` (gids 2016 = 1187664277, 2021 = 1633405383, 2026 =
  162500738); hazard: an **unmarked trailing block of 40-man players with empty salary** (2021: 363 rows, 2026: 298)
  → empty salary = "no Opening-Day MLB salary", not an error; `*` deferral suffix on 29 names in 2026; no
  `FA`/`cond`/`opt` cells on the salary tabs; parsed totals equal the sheet Total cell exactly.
- **V-B1 Retrosheet parser scope (2024, 30 teams):** 2,429 games; 248,739 `play,` records of which 59,802 `NP` →
  188,937 real plays; 24 base token families + 23 modifier families; 99.9% of plays covered by ≈25 families;
  compound `K+…`/`W+…` 0.26%; 13,348 explicit `-H` run advances + 5,396 implicit HR runs; `(UR)` on 1,602 plays;
  **604 `radj` placed-runner records must be handled** (extra innings) for RE24-based constants (DR-3.18).
- **B-F3 comparator ids:** the Baseball Projection Project archive (Wayback) holds name-only fantasy columns
  (AB/R/HR/RBI/SB/AVG) — unusable for wOBA scoring; Tango's `marcel2010.zip` carries `playerID` (bbref-style) +
  wOBA + components → valid DR-3.11 golden. Steamer downloads need a free login — join-key check deferred to the
  comparator-fixture ticket (user's call).
- **B-F1 paper test:** a distribution-valued trait fits Marcel and a quantile model alike —
  `project(&self, view, player, as_of) -> Option<Forecast>` with `Forecast { pa: Distribution, rate:
  BTreeMap<MetricId, Distribution>, reliability: Option<f64> }`, `Distribution::{Point, Quantiles, Samples,
  Normal}` exposing `mean()`/`quantile(q)`; Marcel returns `Point` and the harness attaches empirical quantiles; a
  Bayesian model returns `Samples` and skips the harness interval; the gate scores both on 80%-interval coverage.
  Ship the distribution-valued trait from day one.
- **P3 lavapipe:** `vulkan-swrast` not installed (sudo needed). Pointing `VK_DRIVER_FILES` at a missing ICD makes
  all four `tests/render.rs` tests print "no wgpu adapter; skipping" and **pass** — confirms the audit's CI
  silent-skip item. Probe runs once the package is installed.
- **Taste prototypes** (`/tmp/sabr-proto/taste/{axis,grammar,fan,layouts}.html`, variant switchers, fake data):
  50 PA spans 22–43 calendar days; `OHTANI PIT PROJ`⏎ = 16 keystrokes; a volume-weighted picker rank buries Edwin
  Díaz 6th of 10 `DIAZ` matches; the where-wrong grid with Wilson whiskers makes a p = .74 column fail
  unambiguously only in its pooled row; the ROST table needs the wide (5fr) dock column.

### DR-3.21a Interval model amended: PA-standardised residuals per bucket
- **Decision:** the residual model conditions on reliability bucket **and** sample size by standardising residuals
  as z = err·√PA (PA = projected PA when forecasting, actual PA when fitting); quantiles are taken on z per bucket
  from strictly-earlier folds; intervals are de-standardised at forecast time. Seasons 2001–2003 are warm-up (no
  interval, excluded from the gate).
- **Rejected:** bucket-only quantiles (over-cover Low/None at 0.88–0.98 in the prototype).
- **Why:** sampling variance scales as 1/PA; the prototype's per-bucket mean PICP lands at 0.80–0.81 only with the
  standardisation.
- **Would change our mind:** a model family whose error does not scale with √PA — then the standardisation is
  model-declared, not harness-imposed.

### DR-3.38 Axis labelling = month ticks on the PA axis + as-of cursor + IL-gap shading (P4 variant c)
- **Decision:** the index axis shows calendar month ticks at the first PA of each month, season dividers, an as-of
  cursor that dims the future, and IL/coverage gaps as shaded bands whose width encodes days (clamped).
- **Rejected:** PA ticks only; date at every 50-PA tick; dual PA/date rows (14 px).
- **Why:** time is 2× non-uniform along the PA axis and gaps have zero width in PA space — both must be visible
  without a second label row.
- **Would change our mind:** the shaded bands fighting the fan-chart ribbons visually — then gaps become hatched
  strips in the axis gutter.

### DR-3.39 Grammar ergonomics = `.BAT/.PIT` suffix + ranked picker on ambiguity (P5 variants a + d)
- **Decision:** DR-3.23 as written; the suffix doubles as the picker's role filter; the picker folds accents and
  Jr./Sr., ranks exact token first and then by a role-neutral weight (not PA+IP volume); picks rewrite the token to
  the MLBAM id.
- **Rejected:** role as a separate word (`PIT ROST` collides with the team code); id-primary entry.
- **Why:** 16 keystrokes for a role-qualified projection; one filter keystroke resolves 10-way surname collisions.
- **Would change our mind:** nothing foreseeable; the weight choice is a spec detail.

### DR-3.40 Projection display = nested 50/80/95 ribbons + where-wrong grid with Wilson whiskers (P6 variants a + d)
- **Decision:** CAST/PROJ draw nested 50/80/95 ribbons on the GPU line (the band primitive from the audit's
  extension list); BTST is the (season × bucket) PICP grid with Wilson 95% whiskers around 0.80, the [.75, .85]
  reporting tick, `↑merge` flags below `n_min`, and a pooled-per-bucket row.
- **Rejected:** quantile lines only (no band primitive, less legible); terminal spark fan + percentile bars as the
  primary display (percentile bars stay as a secondary row).
- **Why:** the ribbons are the uncertainty; the grid is the honesty.
- **Would change our mind:** the band primitive slipping Ticket 01 — then (b) quantile lines are the interim.

### DR-3.41 Presets ship as mocked: 5 / 3 / 4 panes, ROST pinned to the wide column
- **Decision:** Morning front-office briefing (CAST watchlist, CAST movers, FEED transactions, ALRT, GAME slate);
  Prospect watch (FARM, FEED draft, PROJ MLB-equivalent); Trade deadline war room (ROST roster, ROST WAR gaps, COMP,
  PROJ ROS fan); a freshness chip bar per preset (source ages, as-of, deadline T-minus); the ROST roster table is
  pinned to the 5fr column.
- **Rejected:** trimming each preset to 3 panes.
- **Why:** the mock fits one 1400 px screen per preset; the extra panes are the ones the postings describe.
- **Would change our mind:** a preset failing the upstream smoke run's frame budget with real data.

### DR-3.19a Pitching Marcel amended to the recovered constants; golden becomes a hard test (2026-10-22)
- **Decision:** the pitching model adopts the constants implied by Tango's PitchingMarcel 2010 fixture
  (recovered by offline reverse-engineering, no fitting): a ≈268-IP single-scale regression (q10/q90 = 260–275,
  not the 3/2/1-weighted +60/+25 split), projected IP = 0.59·IP₋₁ + 0.16·IP₋₂ + 30 (not 0.5/0.1/+50), and an
  ER projection via a BaseRuns-style composite. The pitching golden test becomes a hard assertion (≥85% of rows
  within ERA ±0.05 / IP ±5 / reliability ±0.01), matching the batting golden's status (90.5% match).
- **Rejected:** keeping the +60/+25 spec'd constants with the golden as a non-asserting diagnosis (the model
  would match 0/964 rows of the file it claims to reproduce).
- **Why:** D0.1 says prior-art methodology is re-derived and cited; the fixture IS the system; the recovered
  constants are empirical facts about Marcel-the-published-system, superseding the paraphrased spec text.
- **Would change our mind:** a Tango publication stating the +60/+25 constants explicitly — then the fixture is
  re-examined for version differences and the constants versioned per year.
